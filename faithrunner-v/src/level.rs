// Ported to Rust from SK8V (https://github.com/becaring/SK8V): host/src/physics_level.cpp
// and host/src/game_probe.cpp (FragSlot, FragInstOf). Copyright 2026 becaring, Apache-2.0;
// the live collision reader is adapted from Sol4ra's LS-Skate-LiveCollision (see NOTICE).
// Changed for Faith Runner V: Rust port, no periodic census (a failing walk relocates instead).
//
//! Finds GTA's physics level: the table of every collision object the game has
//! loaded. Every private read-write cell holding one of the instances GTA's own
//! entities name votes for the table base its level index implies.
use crate::{log, shv, win};

const MASK: u64 = 0x7FFF_FFFF_FFF0;
const STRIDE: u64 = 0x30;

/// GetFragInst's vtable slot: its call site carries the slot byte at +0x1D.
fn frag_slot() -> Option<usize> {
    static SLOT: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *SLOT.get_or_init(|| {
        const SITE: [u8; 29] = [
            0x0F, 0x84, 0x8F, 0x00, 0x00, 0x00, 0x8A, 0x48, 0x28, 0x80, 0xE9, 0x02, 0x80, 0xF9, 0x03, 0x0F, 0x87, 0x80, 0x00,
            0x00, 0x00, 0x48, 0x8B, 0x10, 0x48, 0x8B, 0xC8, 0xFF, 0x52,
        ];
        let (lo, hi) = win::gta_image();
        let mut p = lo;
        while p < hi {
            let mut mbi = win::MemoryBasicInformation::default();
            if unsafe { win::VirtualQuery(p as *const _, &mut mbi, size_of::<win::MemoryBasicInformation>()) } == 0 {
                break;
            }
            let (r0, r1) = (mbi.base_address, (mbi.base_address + mbi.region_size).min(hi));
            let exec = mbi.protect & (win::PAGE_EXECUTE_READ | win::PAGE_EXECUTE_READWRITE | win::PAGE_EXECUTE_WRITECOPY) != 0
                && mbi.protect & (win::PAGE_GUARD | win::PAGE_NOACCESS) == 0;
            if mbi.state == win::MEM_COMMIT && exec && r1 > r0 + SITE.len() {
                let region = unsafe { std::slice::from_raw_parts(r0 as *const u8, r1 - r0) };
                if let Some(at) = region.windows(SITE.len()).position(|w| w == SITE) {
                    let slot = region[at + 0x1D] as i8;
                    log!("level: GetFragInst vtable slot {:#x}", slot);
                    return (slot >= 0).then_some(slot as usize);
                }
            }
            p = mbi.base_address + mbi.region_size;
        }
        log!("level: GetFragInst call site not found on this build");
        None
    })
}

/// The fragInst of a GTA entity (script thread only).
pub fn frag_inst(entity: usize) -> Option<u64> {
    frag_inst_of(entity)
}

/// The fragInst (physics instance) of a GTA entity (script thread only: calls GTA).
fn frag_inst_of(entity: usize) -> Option<u64> {
    let slot = frag_slot()?;
    let vtable = win::read_u64(entity)? as usize;
    let f = win::read_u64(vtable + slot)?;
    if f == 0 {
        return None;
    }
    let f: unsafe extern "system" fn(usize) -> usize = unsafe { std::mem::transmute(f as usize) };
    let inst = unsafe { f(entity) } as u64;
    (inst != 0).then_some(inst)
}

/// The physics instances of the world's peds, vehicles and objects (script thread).
pub fn named_instances() -> Vec<u64> {
    let s = shv::shv();
    let mut out = Vec::new();
    let mut h = vec![0i32; 1024];
    for get in [s.all_peds, s.all_vehicles, s.all_objects] {
        let n = unsafe { get(h.as_mut_ptr(), h.len() as i32) }.clamp(0, 1024) as usize;
        for &handle in &h[..n] {
            let e = unsafe { (s.handle_address)(handle) } as usize;
            if e != 0
                && let Some(f) = frag_inst_of(e)
            {
                out.push(f);
            }
        }
    }
    out
}

/// Finds the table from the named instances (any thread: only reads memory). 0: not found.
pub fn locate(mut insts: Vec<u64>) -> u64 {
    insts.sort_unstable();
    insts.dedup();
    if insts.is_empty() {
        return 0;
    }
    let index: Vec<Option<u64>> = insts.iter().map(|&i| win::read_u16(i as usize + 0x18).map(|v| v as u64)).collect();
    let indexed = index.iter().filter(|i| i.is_some()).count() as u32;
    let settled = 8u32.max(indexed - indexed / 10);
    let (first, last) = (insts[0], *insts.last().unwrap());
    let mut votes: std::collections::HashMap<u64, u32> = Default::default();
    let (mut best, mut top) = (0u64, 0u32);
    const PAGE: usize = 0x1000;
    const CHUNK: usize = 1 << 20;
    let mut buf = vec![0u8; CHUNK];
    let mut ws = vec![win::WorkingSetExInformation::default(); CHUNK / PAGE];
    let me = unsafe { win::GetCurrentProcess() };
    // Our own copies of the pointers (the list, the read buffer) must not vote.
    let own = [
        (insts.as_ptr() as u64, insts.as_ptr() as u64 + insts.len() as u64 * 8),
        (buf.as_ptr() as u64, buf.as_ptr() as u64 + CHUNK as u64),
    ];
    let mut a: usize = 0x10000;
    while top < settled && a < 0x7FFF_FFFF_0000 {
        let mut mbi = win::MemoryBasicInformation::default();
        if unsafe { win::VirtualQuery(a as *const _, &mut mbi, size_of::<win::MemoryBasicInformation>()) } != size_of::<win::MemoryBasicInformation>() {
            break;
        }
        a = mbi.base_address + mbi.region_size;
        if mbi.state != win::MEM_COMMIT || mbi.kind != win::MEM_PRIVATE || mbi.protect & win::PAGE_GUARD != 0 || mbi.protect & win::PAGE_READWRITE == 0 {
            continue;
        }
        let (lo, hi) = (mbi.base_address, mbi.base_address + mbi.region_size);
        let mut p = lo;
        while p < hi && top < settled {
            let want = CHUNK.min(hi - p);
            let pages = want / PAGE;
            // Only pages in RAM: reading paged-out memory pulls it back from disk (SK8V, 2026-10-09).
            for (k, w) in ws.iter_mut().enumerate().take(pages) {
                w.virtual_address = p + k * PAGE;
            }
            let known = unsafe { win::K32QueryWorkingSetEx(me, ws.as_mut_ptr(), (pages * size_of::<win::WorkingSetExInformation>()) as u32) } != 0;
            let valid = |k: usize| !known || ws[k].attributes & 1 != 0;
            let mut k0 = 0;
            while k0 < pages {
                if !valid(k0) {
                    k0 += 1;
                    continue;
                }
                let mut k1 = k0 + 1;
                while k1 < pages && valid(k1) {
                    k1 += 1;
                }
                let run = p + k0 * PAGE;
                let mut got = 0usize;
                unsafe { win::ReadProcessMemory(me, run as *const _, buf.as_mut_ptr() as *mut _, (k1 - k0) * PAGE, &mut got) };
                let mut o = 0;
                while o + 8 <= got {
                    let m = u64::from_le_bytes(buf[o..o + 8].try_into().unwrap()) & MASK;
                    if m >= first && m <= last
                        && let Ok(at) = insts.binary_search(&m)
                        && let Some(ix) = index[at]
                    {
                        let cell = (run + o) as u64;
                        if cell >= ix * STRIDE && !own.iter().any(|&(l, h)| cell >= l && cell < h) {
                            let base = cell - ix * STRIDE;
                            let n = votes.entry(base).or_insert(0);
                            *n += 1;
                            if *n > top {
                                top = *n;
                                best = base;
                            }
                        }
                    }
                    o += 8;
                }
                k0 = k1;
            }
            p += CHUNK;
        }
    }
    log!("level: locate: {} instances named by GTA, best table candidate {:#x} with {} hits", insts.len(), best, top);
    if top as usize >= 8.max(insts.len() / 2) { best } else { 0 }
}
