// Ported to Rust from SK8V (https://github.com/becaring/SK8V): host/src/pose_hook.cpp and
// game_probe.cpp (ResolvePedSkeleton, the crBoneData layout). Copyright 2026 becaring,
// Apache-2.0. Changed for Faith Runner V: Rust port; no fades, gun blend or live clip; the
// pose is published in the ped's own (object) space.
//
//! Writes a pose onto the player's GTA skeleton, the way SK8V does for its skater.
//! GTA rebuilds the skeleton every frame in its animation pass (ped vtable slot 65)
//! and two animation job tasks on a worker thread; each is wrapped and, right after
//! GTA's own work, the published pose replaces it for this one skeleton. GTA then
//! draws, lights and shadows the character as usual.
//!
//! Offsets are GTA V Legacy 1.0.3889's (SK8V's); each is checked before patching,
//! and nothing is patched if they differ (or if SK8V is using the same hooks).
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glam::Mat4;

use crate::retarget::{GtaSkeleton, PoseOut};
use crate::{log, win};

const SLOT: usize = 65; // vtable offset 0x208
const SLOT_RVA: usize = 0x7B0270; // CPed per-frame animation/IK pass
const COPIED_SLOTS: usize = 512;
const MAX_BONES: usize = 512;
/// The two anim job tasks and their params offset of the locals pointer.
const JOBS: [(usize, usize); 2] = [(0x15B9948, 0x20), (0x15B8680, 0x18)];
// mov rax, rsp; push rbp; push rbx; push rsi; push rdi; push r12..r15
const JOB_PROLOGUE: [u8; 15] = [0x48, 0x8B, 0xC4, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57];

/// The skeleton (crSkeleton) being driven; 0: none.
static SKELETON: AtomicUsize = AtomicUsize::new(0);
static ENTITY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SLOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TABLE: AtomicUsize = AtomicUsize::new(0);
static COPY: AtomicUsize = AtomicUsize::new(0);
static TRAMPOLINES: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static PATCHED: Mutex<Option<[(usize, u64); 2]>> = Mutex::new(None);
static RELAY: AtomicUsize = AtomicUsize::new(0);

/// The pose to write, in the ped's object space (one matrix per bone).
static POSE: Mutex<Option<Arc<PoseOut>>> = Mutex::new(None);

pub fn publish(pose: Option<Arc<PoseOut>>) {
    *POSE.lock().unwrap() = pose;
}

fn rd<T: Copy>(a: usize) -> Option<T> {
    let mut b = vec![0u8; size_of::<T>()];
    win::read(a, &mut b).then(|| unsafe { std::ptr::read_unaligned(b.as_ptr() as *const T) })
}

/// The skeleton of a ped entity: the fragInst's cache entry, or the drawable's (SK8V's two routes).
pub fn skeleton_of(entity: usize) -> Option<usize> {
    if let Some(frag) = crate::level::frag_inst(entity)
        && let Some(cache) = rd::<usize>(frag as usize + 0x68).filter(|c| *c != 0)
        && let Some(s) = rd::<usize>(cache + 0x178).filter(|s| *s != 0)
    {
        return Some(s);
    }
    let draw = rd::<usize>(entity + 0x50).filter(|d| *d != 0)?;
    rd::<usize>(draw + 0x28).filter(|s| *s != 0)
}

/// A skeleton's bind pose (crSkeletonData: bones of 80 bytes).
pub fn read_skeleton(s: usize) -> Option<GtaSkeleton> {
    let data = rd::<usize>(s)?;
    let count = rd::<i32>(s + 0x20)?;
    if data == 0 || count <= 0 || count as usize > MAX_BONES {
        return None;
    }
    let bones = rd::<usize>(data + 0x20)?;
    let mut sk = GtaSkeleton { names: vec![], parents: vec![], rot: vec![], pos: vec![] };
    for i in 0..count as usize {
        let mut b = [0u8; 80];
        if !win::read(bones + 80 * i, &mut b) {
            return None;
        }
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let parent = i16::from_le_bytes([b[0x32], b[0x33]]);
        let name_ptr = usize::from_le_bytes(b[0x38..0x40].try_into().unwrap());
        let mut name = [0u8; 64];
        if name_ptr != 0 {
            win::read(name_ptr, &mut name[..63]);
        }
        let end = name.iter().position(|c| *c == 0).unwrap_or(63);
        sk.names.push(String::from_utf8_lossy(&name[..end]).into_owned());
        sk.parents.push(parent as i32);
        sk.rot.push(glam::Quat::from_xyzw(f(0), f(4), f(8), f(12)).normalize());
        sk.pos.push(glam::Vec3::new(f(0x10), f(0x14), f(0x18)));
    }
    Some(sk)
}

/// RAGE bone rows: X, Y, Z axes and translation (row vectors) = glam's columns.
unsafe fn store_xyz(at: usize, m: &Mat4) {
    let p = at as *mut f32;
    for (r, col) in [m.x_axis, m.y_axis, m.z_axis, m.w_axis].iter().enumerate() {
        unsafe {
            *p.add(r * 4) = col.x;
            *p.add(r * 4 + 1) = col.y;
            *p.add(r * 4 + 2) = col.z;
        }
    }
}

/// After GTA's pass: the published pose into the skeleton's object and local matrices.
fn apply(s: usize) {
    let Some(pose) = POSE.lock().ok().and_then(|g| g.clone()) else { return };
    unsafe {
        let data = *(s as *const usize);
        let locals = *((s + 0x10) as *const usize);
        let objects = *((s + 0x18) as *const usize);
        let count = *((s + 0x20) as *const i32);
        if data == 0 || locals == 0 || objects == 0 || count <= 0 || count as usize != pose.objects.len() || pose.locals.len() != pose.objects.len() {
            return;
        }
        for (i, (obj, local)) in pose.objects.iter().zip(&pose.locals).enumerate() {
            store_xyz(objects + 64 * i, obj);
            store_xyz(locals + 64 * i, local);
        }
    }
}

type SlotFn = unsafe extern "system" fn(usize, u8) -> u8;
type JobFn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;

unsafe extern "system" fn slot_hook(this: usize, flag: u8) -> u8 {
    let original: SlotFn = unsafe { std::mem::transmute(ORIGINAL_SLOT.load(Ordering::Acquire)) };
    let r = unsafe { original(this, flag) };
    if this == ENTITY.load(Ordering::Acquire) {
        let s = SKELETON.load(Ordering::Acquire);
        if s != 0 {
            let _ = std::panic::catch_unwind(|| apply(s));
        }
    }
    r
}

unsafe fn job<const I: usize>(params: usize, b: usize, c: usize, d: usize) -> usize {
    let t: JobFn = unsafe { std::mem::transmute(TRAMPOLINES[I].load(Ordering::Acquire)) };
    let r = unsafe { t(params, b, c, d) };
    let s = SKELETON.load(Ordering::Acquire);
    if s != 0 && params != 0 {
        let locals = unsafe { *((s + 0x10) as *const usize) };
        if locals != 0 && unsafe { *((params + JOBS[I].1) as *const usize) } == locals {
            let _ = std::panic::catch_unwind(|| apply(s));
        }
    }
    r
}
unsafe extern "system" fn job0(a: usize, b: usize, c: usize, d: usize) -> usize {
    unsafe { job::<0>(a, b, c, d) }
}
unsafe extern "system" fn job1(a: usize, b: usize, c: usize, d: usize) -> usize {
    unsafe { job::<1>(a, b, c, d) }
}

/// 4 KiB of executable memory within a 32-bit jump of GTA's image.
fn alloc_near(base: usize) -> usize {
    let mut a = base.saturating_sub(0x10000) & !0xFFFF;
    let low = base.saturating_sub(0x7000_0000);
    while a > low {
        let p = unsafe { win::VirtualAlloc(a as *const c_void, 4096, win::MEM_COMMIT | win::MEM_RESERVE, win::PAGE_EXECUTE_READWRITE) };
        if !p.is_null() {
            return p as usize;
        }
        a -= 0x10000;
    }
    0
}

fn patch_qword(at: usize, value: u64) {
    unsafe {
        let mut old = 0u32;
        win::VirtualProtect(at as *const c_void, 8, win::PAGE_EXECUTE_READWRITE, &mut old);
        (*(at as *const std::sync::atomic::AtomicU64)).store(value, Ordering::SeqCst);
        win::VirtualProtect(at as *const c_void, 8, old, &mut old);
        win::FlushInstructionCache(win::GetCurrentProcess(), at as *const c_void, 8);
    }
}

fn install_jobs(base: usize) -> bool {
    if PATCHED.lock().unwrap().is_some() {
        return true;
    }
    for (rva, _) in JOBS {
        let t = base + rva;
        let bytes = unsafe { std::slice::from_raw_parts(t as *const u8, JOB_PROLOGUE.len()) };
        if t & 7 != 0 || bytes != JOB_PROLOGUE {
            log!("pose: anim task +{rva:#x} differs (another mod such as SK8V may be using it); not hooked");
            return false;
        }
    }
    let mem = match RELAY.load(Ordering::Acquire) {
        0 => {
            let m = alloc_near(base);
            RELAY.store(m, Ordering::Release);
            m
        }
        m => m,
    };
    if mem == 0 {
        log!("pose: no memory near the game image; not hooked");
        return false;
    }
    let wraps: [usize; 2] = [job0 as *const () as usize, job1 as *const () as usize];
    let mut patched = [(0usize, 0u64); 2];
    for (i, (rva, _)) in JOBS.iter().enumerate() {
        let t = base + rva;
        let relay = mem + 64 * i;
        let tramp = mem + 64 * i + 32;
        unsafe {
            // relay: jmp [rip+0]; dq wrap
            let r = relay as *mut u8;
            std::ptr::copy_nonoverlapping([0xFFu8, 0x25, 0, 0, 0, 0].as_ptr(), r, 6);
            std::ptr::copy_nonoverlapping(wraps[i].to_le_bytes().as_ptr(), r.add(6), 8);
            // trampoline: the 5 prologue bytes; jmp [rip+0]; dq t+5
            let tp = tramp as *mut u8;
            std::ptr::copy_nonoverlapping(JOB_PROLOGUE.as_ptr(), tp, 5);
            std::ptr::copy_nonoverlapping([0xFFu8, 0x25, 0, 0, 0, 0].as_ptr(), tp.add(5), 6);
            std::ptr::copy_nonoverlapping((t + 5).to_le_bytes().as_ptr(), tp.add(11), 8);
        }
        TRAMPOLINES[i].store(tramp, Ordering::Release);
        let original = unsafe { *(t as *const u64) };
        let rel = (relay as i64 - (t as i64 + 5)) as i32;
        let mut patch = original.to_le_bytes();
        patch[0] = 0xE9;
        patch[1..5].copy_from_slice(&rel.to_le_bytes());
        patched[i] = (t, original);
        unsafe { win::FlushInstructionCache(win::GetCurrentProcess(), mem as *const c_void, 4096) };
        patch_qword(t, u64::from_le_bytes(patch));
    }
    *PATCHED.lock().unwrap() = Some(patched);
    log!("pose: anim job tasks hooked");
    true
}

fn uninstall_jobs() {
    if let Some(p) = PATCHED.lock().unwrap().take() {
        for (t, original) in p {
            patch_qword(t, original);
        }
        log!("pose: anim job tasks restored");
    }
}

/// Whether another mod (SK8V while skating) holds the pose hooks on this ped right now.
pub fn taken_by_other(entity: usize) -> bool {
    if PATCHED.lock().unwrap().is_some() || entity == 0 {
        return false; // ours
    }
    let base = win::gta_image().0;
    let jobs_changed = JOBS.iter().any(|(rva, _)| {
        let bytes = unsafe { std::slice::from_raw_parts((base + rva) as *const u8, JOB_PROLOGUE.len()) };
        bytes != JOB_PROLOGUE
    });
    let slot_changed = win::read_u64(entity)
        .and_then(|table| win::read_u64(table as usize + 8 * SLOT))
        .is_some_and(|slot| slot as usize != base + SLOT_RVA);
    jobs_changed || slot_changed
}

/// Start driving `entity`'s skeleton `s` (the player ped). False: hooks not available.
pub fn install(entity: usize, s: usize) -> bool {
    let base = win::gta_image().0;
    if ENTITY.load(Ordering::Acquire) == entity && entity != 0 {
        SKELETON.store(s, Ordering::Release);
        return true;
    }
    uninstall();
    unsafe {
        let table = *(entity as *const usize);
        let slot = *((table as *const usize).add(SLOT));
        if slot != base + SLOT_RVA {
            log!("pose: ped's animation pass is image +{:#x}, expected +{SLOT_RVA:#x} (another mod may have it); not hooked", slot.wrapping_sub(base));
            return false;
        }
        if !install_jobs(base) {
            return false;
        }
        let copy = win::VirtualAlloc(std::ptr::null(), 8 * (COPIED_SLOTS + 1), win::MEM_COMMIT | win::MEM_RESERVE, win::PAGE_READWRITE) as *mut usize;
        if copy.is_null() {
            uninstall_jobs();
            return false;
        }
        std::ptr::copy_nonoverlapping((table - 8) as *const usize, copy, COPIED_SLOTS + 1); // RTTI + slots
        ORIGINAL_SLOT.store(slot, Ordering::Release);
        *copy.add(1 + SLOT) = slot_hook as *const () as usize;
        COPY.store(copy as usize, Ordering::Release);
        ORIGINAL_TABLE.store(table, Ordering::Release);
        SKELETON.store(s, Ordering::Release);
        ENTITY.store(entity, Ordering::Release);
        (*(entity as *const AtomicUsize)).store(copy as usize + 8, Ordering::SeqCst);
    }
    log!("pose: driving the player's skeleton");
    true
}

pub fn uninstall() {
    publish(None);
    SKELETON.store(0, Ordering::Release);
    uninstall_jobs();
    let entity = ENTITY.swap(0, Ordering::AcqRel);
    if entity != 0 {
        let copy = COPY.load(Ordering::Acquire);
        let slot0 = unsafe { &*(entity as *const AtomicUsize) };
        // Only if it still points at our copy (the ped may be gone or replaced).
        if win::read_u64(entity).is_some_and(|v| v as usize == copy + 8) {
            slot0.store(ORIGINAL_TABLE.load(Ordering::Acquire), Ordering::SeqCst);
        }
        // The copied table stays allocated: a game thread may still be in it.
        log!("pose: the player's skeleton is GTA's again");
    }
}
