//! GTA's ladders, read live from memory (run from the menu's debug button on a background
//! thread; nothing here runs every frame and nothing calls GTA).
//!
//! GTA keeps a ladder as a 2dfx record on the archetype (model) it belongs to: the ytyp's
//! `CExtensionDefLadder` (CodeWalker MetaTypes.cs) becomes a ladder record when the archetype
//! loads. Any map, add-on maps included, goes through the same code.
//!
//! Layout on 1.0.3889, read off the Strawberry ladder in run 2 (2026-11-09, see LADDERS.md):
//! - archetype +0xA8 -> a 2dfx list whose first qword is an array of record pointers (count taken
//!   as a u16 at list +0x08: run 3 logs the list head to confirm);
//! - the ladder record, 0x48 bytes: vtable +0x00, position +0x08, **bottom +0x18, top +0x24,
//!   normal +0x30** (packed 12-byte vec3s), 12 more bytes (material / template / get-off flags,
//!   not read yet: logged raw).
//!
//! The search (a few seconds):
//! 1. one heap pass: **entities near the player** by shape (fwEntity, FiveM EntitySystem.h:
//!    archetype +0x20, matrix rows +0x60, position +0x90) and a **census of every ladder record**
//!    in memory (vtable in GTA's image + the strict ladder test at the fixed layout);
//! 2. each nearby archetype's ladders through +0xA8 (direct route); only if that finds none at
//!    all, a pointer walk with the same strict test, paths ranked by different ladders found;
//! 3. ladders placed through each entity's matrix.
//!
//! All reads go through `win::read` / ReadProcessMemory: a bad pointer fails, never crashes.
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use glam::Vec3;

use crate::{log, win};

const PAGE: usize = 0x1000;
const CHUNK: usize = 1 << 20;
const PAGE_WRITECOPY: u32 = 0x08;
const MEM_IMAGE: u32 = 0x100_0000;

/// How far (each axis) from the player an entity's origin may be: a big building's origin can
/// sit well away from its ladder.
const ENTITY_REACH: f32 = 150.0;
const LOG_RADIUS: f32 = 30.0;
/// Pointer-walk limits from each archetype.
const DEPTH: usize = 3;
const NODES: usize = 800;

/// GTA's string hash (Jenkins one-at-a-time), lower-cased first (FiveM's HashString).
pub fn joaat(s: &str) -> u32 {
    joaat_raw(&s.to_ascii_lowercase())
}

/// The same without lower-casing (FiveM's HashRageString).
pub fn joaat_raw(s: &str) -> u32 {
    let mut h: u32 = 0;
    for c in s.bytes() {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

fn rd<T: Copy>(a: usize) -> Option<T> {
    let mut b = vec![0u8; size_of::<T>()];
    win::read(a, &mut b).then(|| unsafe { std::ptr::read_unaligned(b.as_ptr() as *const T) })
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn vec3_at(b: &[u8], o: usize) -> Vec3 {
    Vec3::new(f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8))
}

/// A ladder in its archetype's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLadder {
    pub bottom: Vec3,
    pub top: Vec3,
    /// Out from the wall (GTA: where the ped stands to climb; to be checked in game).
    pub normal: Vec3,
    pub material: Option<u32>,
    pub can_get_off_top: Option<bool>,
    pub can_get_off_bottom: Option<bool>,
}

/// A ladder in GTA's world space (Z up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GtaLadder {
    pub bottom: Vec3,
    pub top: Vec3,
    pub normal: Vec3,
    pub can_get_off_top: bool,
    /// The entity it came from (for logging / de-duplication).
    pub entity: usize,
}

thread_local! {
    /// The automatic search (the watcher) logs only its summary.
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

macro_rules! say {
    ($($t:tt)*) => {
        if !QUIET.with(|q| q.get()) {
            log!($($t)*);
        }
    };
}

/// How far the watcher keeps ladders (GTA metres, flat), and how far the player may move, or
/// how long pass, before it searches again.
const KEEP: f32 = 140.0;
const RESEARCH_MOVE: f32 = 60.0;
const RESEARCH_SECS: u64 = 60;

/// `discover` without its step-by-step log lines.
pub fn discover_quiet(player: Vec3) -> Vec<GtaLadder> {
    QUIET.with(|q| q.set(true));
    let r = std::panic::catch_unwind(|| discover(player));
    QUIET.with(|q| q.set(false));
    r.unwrap_or_default()
}

/// The ladder watcher: a lowest-priority background thread. While Faith mode is on it searches
/// GTA's memory near the player when Faith mode starts, after every 60 m moved and once a
/// minute (for map parts streamed in meanwhile), and hands the ladders to the collision
/// builder, which puts them in Faith's world. Reads memory only; nothing per frame.
pub fn start_watch(shared: crate::collision::Handle) {
    let _ = std::thread::Builder::new().name("faithrunner-ladders".into()).spawn(move || {
        unsafe {
            win::SetThreadPriority(win::GetCurrentThread(), -2);
        }
        let mut last: Option<(Vec3, Instant)> = None;
        let mut runs = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (active, player) = {
                let s = shared.lock().unwrap();
                (s.active, s.player)
            };
            if !active {
                if last.take().is_some() {
                    shared.lock().unwrap().ladders = Default::default();
                }
                continue;
            }
            let due = match last {
                None => true,
                Some((c, t)) => (player - c).truncate().length() > RESEARCH_MOVE || t.elapsed().as_secs() >= RESEARCH_SECS,
            };
            if !due {
                continue;
            }
            let t0 = Instant::now();
            let near: Vec<GtaLadder> =
                discover_quiet(player).into_iter().filter(|l| (l.bottom - player).truncate().length() < KEEP).collect();
            if runs < 5 || runs % 20 == 0 {
                log!(
                    "ladders: {} ladders within {KEEP:.0} m of ({:.0}, {:.0}, {:.0}), searched in {:.1} s",
                    near.len(), player.x, player.y, player.z, t0.elapsed().as_secs_f32()
                );
            }
            runs += 1;
            shared.lock().unwrap().ladders = std::sync::Arc::new(near);
            last = Some((player, Instant::now()));
        }
    });
}

/// Faith's ladder for a GTA ladder, fitted to the collision around it: the normal points away
/// from the wall the ladder is on (whichever way GTA's points), the foot sits on the wall's
/// face, and the top is the floor she steps out onto when one is found near GTA's top.
pub fn fit(l: &GtaLadder, origin: Vec3, world: &dyn faith_move::World) -> faith_move::climb::Ladder {
    let mut f = to_faith(l, origin);
    if f.normal.length_squared() < 0.5 {
        return f;
    }
    let half = Vec3::splat(0.05);
    let reach = 1.2;
    let probe = f.base + Vec3::Y * 1.0;
    let dist = |dir: Vec3| world.sweep(half, probe, dir * reach).map(|h| h.t * reach + 0.05);
    // Which side the wall is on. First: the roof she climbs out onto is on the wall's side, a
    // drop on the climbing side (probed well clear of the ladder's own hoops).
    let roof = |dir: Vec3| {
        let from = Vec3::new(f.base.x, f.top + 1.6, f.base.z) + dir * 0.9;
        world.sweep(half, from, -Vec3::Y * 2.6).is_some_and(|h| h.normal.y > 0.7)
    };
    let side = match (roof(-f.normal), roof(f.normal)) {
        (true, false) => Some(f.normal),
        (false, true) => Some(-f.normal),
        _ => None,
    };
    let wall = match side {
        Some(n) => dist(-n).map(|d| (n, d)).or(Some((n, 0.0))),
        // Else the nearer wall behind or in front of it.
        None => match (dist(-f.normal), dist(f.normal)) {
            (Some(b), Some(fr)) if fr < b => Some((-f.normal, fr)),
            (None, Some(fr)) => Some((-f.normal, fr)),
            (Some(b), _) => Some((f.normal, b)),
            _ => None,
        },
    };
    if let Some((n, d)) = wall {
        f.normal = n;
        f.base -= n * d;
    }
    // The floor at the foot, in front of the ladder: GTA's bottom can float up to a metre
    // above it, and she only gets on from the floor when her feet are at the foot.
    let front = Vec3::new(f.base.x, f.base.y + 0.5, f.base.z) + f.normal * 0.45;
    if let Some(h) = world.sweep(half, front, -Vec3::Y * 2.0) {
        let y = front.y - h.t * 2.0 - 0.05;
        if h.normal.y > 0.7 && y < f.base.y + 0.3 && y > f.base.y - 1.5 {
            f.base.y = y;
        }
    }
    // The floor at the top: just over the wall's edge.
    let over = Vec3::new(f.base.x, f.top + 1.5, f.base.z) - f.normal * 0.4;
    if let Some(h) = world.sweep(half, over, -Vec3::Y * 3.0) {
        let y = over.y - h.t * 3.0 - 0.05;
        if h.normal.y > 0.7 && (y - f.top).abs() < 1.5 {
            f.top = y;
        }
    }
    f
}

/// Whether `p` (Faith's space) is in the room a climber needs on and over the top of `l`:
/// the ladder's own rails, rungs and hoops there are left out of the collision (they made
/// floors in mid-air and caught her at the top).
pub fn in_climb_space(l: &faith_move::climb::Ladder, p: Vec3) -> bool {
    let side = Vec3::Y.cross(l.normal).normalize_or_zero();
    let d = p - l.base;
    let (out, along) = (d.dot(l.normal), d.dot(side));
    if along.abs() > 0.45 || out > 0.8 || p.y > l.top + 1.8 {
        return false;
    }
    // In front of the wall all the way up; over the top, back over the wall's edge too.
    (out > 0.04 && p.y > l.base.y - 0.2) || (out > -1.3 && p.y > l.top + 0.15)
}

/// Archetype offset of its 2dfx list (run 2).
pub const ARCH_2DFX: usize = 0xA8;
/// A ladder record's size and fields (run 2).
pub const RECORD: usize = 0x48;
const AT_POSITION: usize = 0x08;
const AT_BOTTOM: usize = 0x18;
const AT_TOP: usize = 0x24;
const AT_NORMAL: usize = 0x30;

/// A real ladder: bottom straight below top (1 cm), a 1.5..60 m rise, a unit level normal.
fn strict(bottom: Vec3, top: Vec3, normal: Vec3) -> bool {
    let finite = bottom.is_finite() && top.is_finite() && normal.is_finite();
    let small = bottom.abs().max_element() < 5000.0 && top.abs().max_element() < 5000.0;
    finite
        && small
        && (bottom.truncate() - top.truncate()).length() < 0.01
        && (1.5..=60.0).contains(&(top.z - bottom.z))
        && (normal.length() - 1.0).abs() < 0.01
        && normal.z.abs() < 0.02
}

/// A ladder record at the fixed layout (the caller checks the vtable).
pub fn decode_record(b: &[u8]) -> Option<LocalLadder> {
    if b.len() < AT_NORMAL + 12 {
        return None;
    }
    let (bottom, top, normal) = (vec3_at(b, AT_BOTTOM), vec3_at(b, AT_TOP), vec3_at(b, AT_NORMAL));
    strict(bottom, top, normal).then_some(LocalLadder { bottom, top, normal, material: None, can_get_off_top: None, can_get_off_bottom: None })
}

/// RAGE's row-vector matrix (rows: x, y, z axes, position) applied to a point / a direction.
pub fn to_world(rows: &[Vec3; 4], p: Vec3) -> Vec3 {
    rows[0] * p.x + rows[1] * p.y + rows[2] * p.z + rows[3]
}
pub fn dir_to_world(rows: &[Vec3; 4], d: Vec3) -> Vec3 {
    rows[0] * d.x + rows[1] * d.y + rows[2] * d.z
}

pub fn place(l: &LocalLadder, rows: &[Vec3; 4], entity: usize) -> GtaLadder {
    GtaLadder {
        bottom: to_world(rows, l.bottom),
        top: to_world(rows, l.top),
        normal: dir_to_world(rows, l.normal).normalize_or_zero(),
        can_get_off_top: l.can_get_off_top.unwrap_or(true),
        entity,
    }
}

/// Faith's ladder (Y up, relative to `origin`; see collision.rs). GTA has no drainpipes.
pub fn to_faith(l: &GtaLadder, origin: Vec3) -> faith_move::climb::Ladder {
    let n = Vec3::new(l.normal.x, l.normal.y, 0.0).normalize_or_zero();
    faith_move::climb::Ladder {
        base: crate::collision::to_faith(l.bottom, origin),
        top: crate::collision::to_faith(l.top, origin).y,
        normal: crate::collision::dir_to_faith(n),
        pipe: false,
        exit: l.can_get_off_top,
    }
}

/// An entity's rows (x, y, z axes, position) from its first 0xA0 bytes.
fn entity_rows(e: &[u8]) -> [Vec3; 4] {
    [vec3_at(e, 0x60), vec3_at(e, 0x70), vec3_at(e, 0x80), vec3_at(e, 0x90)]
}

/// Matrix rows that can be an entity's: finite, sensibly scaled, at right angles.
fn rows_ok(r: &[Vec3; 4]) -> bool {
    let lens = [r[0].length(), r[1].length(), r[2].length()];
    if !r.iter().all(|v| v.is_finite()) || lens.iter().any(|l| !(0.05..50.0).contains(l)) {
        return false;
    }
    let cos = |a: Vec3, b: Vec3| (a.dot(b) / (a.length() * b.length())).abs();
    cos(r[0], r[1]) < 0.02 && cos(r[0], r[2]) < 0.02 && cos(r[1], r[2]) < 0.02
}

fn heap_ptr(v: u64, image: (usize, usize)) -> bool {
    v >= 0x10000 && v < 0x7FF0_0000_0000 && v % 8 == 0 && !(v as usize >= image.0 && (v as usize) < image.1)
}

/// An archetype as FiveM's fwArchetype has it: a vtable in GTA's image, a model hash at +0x18,
/// a sane radius (+0x2C) and box (+0x30 min, +0x40 max).
fn archetype_ok(a: usize, image: (usize, usize)) -> Option<u32> {
    let mut b = [0u8; 0x50];
    if !win::read(a, &mut b) {
        return None;
    }
    let vt = u64_at(&b, 0) as usize;
    let hash = u32_at(&b, 0x18);
    let radius = f32_at(&b, 0x2C);
    let (lo, hi) = (vec3_at(&b, 0x30), vec3_at(&b, 0x40));
    let ok = vt >= image.0 && vt < image.1 && hash != 0 && radius.is_finite() && (0.0..10_000.0).contains(&radius)
        && lo.is_finite() && hi.is_finite() && lo.cmple(hi).all();
    ok.then_some(hash)
}

// ---------------------------------------------------------------- memory search

fn query(a: usize) -> Option<win::MemoryBasicInformation> {
    let mut mbi = win::MemoryBasicInformation::default();
    let n = unsafe { win::VirtualQuery(a as *const _, &mut mbi, size_of::<win::MemoryBasicInformation>()) };
    (n == size_of::<win::MemoryBasicInformation>()).then_some(mbi)
}

fn readable(p: u32) -> bool {
    p & (win::PAGE_GUARD | win::PAGE_NOACCESS) == 0 && p != 0
}
fn writable(p: u32) -> bool {
    p & (win::PAGE_READWRITE | PAGE_WRITECOPY | win::PAGE_EXECUTE_READWRITE | win::PAGE_EXECUTE_WRITECOPY) != 0
}

/// Every resident run of GTA's heap (and the image's own writable data), in pieces of up to
/// 1 MiB, to `each(address, bytes)`; `each` returns false to stop. Only pages in RAM are read
/// (reading paged-out memory pulls it back from disk: SK8V, 2026-10-09). This thread's stack is
/// left out.
fn walk_heap(image: (usize, usize), mut each: impl FnMut(usize, &[u8]) -> bool) {
    let mut buf = vec![0u8; CHUNK];
    let mut ws = vec![win::WorkingSetExInformation::default(); CHUNK / PAGE];
    let here = 0u8;
    let stack = query(&here as *const u8 as usize).map(|m| m.allocation_base).unwrap_or(usize::MAX);
    let me = unsafe { win::GetCurrentProcess() };
    let mut a: usize = 0x10000;
    while a < 0x7FFF_FFFF_0000 {
        let Some(mbi) = query(a) else { break };
        a = mbi.base_address + mbi.region_size;
        let in_image = mbi.base_address >= image.0 && mbi.base_address < image.1;
        let ok = mbi.state == win::MEM_COMMIT
            && readable(mbi.protect)
            && writable(mbi.protect)
            && (mbi.kind == win::MEM_PRIVATE || (mbi.kind == MEM_IMAGE && in_image))
            && mbi.allocation_base != stack;
        if !ok {
            continue;
        }
        let (r0, r1) = (mbi.base_address, mbi.base_address + mbi.region_size);
        let mut p = r0;
        while p < r1 {
            let pages = CHUNK.min(r1 - p) / PAGE;
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
                if got > 0 && !each(run, &buf[..got]) {
                    return;
                }
                k0 = k1;
            }
            p += CHUNK;
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.chunks(16).map(|r| r.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" | ")
}

struct Entity {
    addr: usize,
    arch: usize,
    rows: [Vec3; 4],
    kind: u8,
}

/// One ladder record reached from an archetype, and how.
#[derive(Clone)]
struct Found {
    path: Vec<u16>,
    record: usize,
    ladder: LocalLadder,
}

/// A record pointer that is a ladder: vtable in GTA's image and the strict test.
fn record_at(p: u64, image: (usize, usize)) -> Option<LocalLadder> {
    if !heap_ptr(p, image) {
        return None;
    }
    let mut b = [0u8; RECORD];
    if !win::read(p as usize, &mut b) {
        return None;
    }
    let vt = u64_at(&b, 0) as usize;
    if vt < image.0 || vt >= image.1 {
        return None;
    }
    decode_record(&b)
}

/// The direct route: archetype +0xA8 -> list; list +0 -> pointer array, count u16 at list +8
/// (if that count is not sane, up to 32 pointers are tried). Gives the ladders, the list's first
/// 16 bytes (for the log) and how many entries were read; None when there is no list.
fn direct(arch: usize, image: (usize, usize)) -> Option<(Vec<Found>, [u8; 16], usize)> {
    let list = rd::<u64>(arch + ARCH_2DFX).filter(|p| heap_ptr(*p, image))? as usize;
    let mut head = [0u8; 16];
    if !win::read(list, &mut head) {
        return None;
    }
    let data = u64_at(&head, 0);
    if !heap_ptr(data, image) {
        return None;
    }
    let count = u16::from_le_bytes([head[8], head[9]]) as usize;
    let n = if (1..=256).contains(&count) { count } else { 32 };
    let mut ptrs = vec![0u8; n * 8];
    if !win::read(data as usize, &mut ptrs) {
        ptrs.truncate(8);
        if !win::read(data as usize, &mut ptrs) {
            return None;
        }
    }
    let found = (0..ptrs.len() / 8)
        .filter_map(|k| {
            let p = u64_at(&ptrs, k * 8);
            record_at(p, image).map(|ladder| Found { path: vec![ARCH_2DFX as u16, 0, (k * 8) as u16], record: p as usize, ladder })
        })
        .collect();
    Some((found, head, ptrs.len() / 8))
}

/// Fallback: pointers followed from an archetype (breadth first, `DEPTH` deep, `NODES` records
/// at most), every pointer tested as a ladder record. Shortest path per record.
fn walk(arch: usize, image: (usize, usize)) -> Vec<Found> {
    let mut out: Vec<Found> = vec![];
    let mut seen = HashSet::from([arch]);
    let mut queue = std::collections::VecDeque::from([(arch, Vec::<u16>::new())]);
    let mut nodes = 0;
    while let Some((at, path)) = queue.pop_front() {
        nodes += 1;
        if nodes > NODES {
            break;
        }
        let mut b = vec![0u8; if path.is_empty() { 0x200 } else { 0x100 }];
        if !win::read(at, &mut b) {
            b.truncate(0x40);
            if !win::read(at, &mut b) {
                continue;
            }
        }
        for o in (0..b.len().saturating_sub(7)).step_by(8) {
            let v = u64_at(&b, o);
            if !heap_ptr(v, image) || !seen.insert(v as usize) {
                continue;
            }
            let mut p = path.clone();
            p.push(o as u16);
            if let Some(ladder) = record_at(v, image) {
                out.push(Found { path: p, record: v as usize, ladder });
            } else if p.len() < DEPTH {
                queue.push_back((v as usize, p));
            }
        }
    }
    out
}

fn path_str(p: &[u16]) -> String {
    p.iter().map(|o| format!("+{o:#x}")).collect::<Vec<_>>().join(" -> ")
}

/// The search (any thread: only reads memory). Logs every step; gives the ladders found on the
/// entities near `player` (GTA position), in world space.
pub fn discover(player: Vec3) -> Vec<GtaLadder> {
    let t0 = Instant::now();
    let image = win::gta_image();

    // 1. One heap pass: entity-shaped objects near the player, and every ladder record.
    const MAX_E: usize = 20_000;
    const MAX_R: usize = 20_000;
    let mut entities: Vec<Entity> = Vec::with_capacity(MAX_E);
    let mut census: Vec<(usize, usize)> = Vec::with_capacity(MAX_R);
    let own = [
        (entities.as_ptr() as usize, entities.as_ptr() as usize + MAX_E * size_of::<Entity>()),
        (census.as_ptr() as usize, census.as_ptr() as usize + MAX_R * size_of::<(usize, usize)>()),
    ];
    walk_heap(image, |run, b| {
        for o in (0..b.len().saturating_sub(RECORD - 1)).step_by(8) {
            let cell = run + o;
            if own.iter().any(|&(l, h)| cell >= l && cell < h) {
                continue;
            }
            let v = u64_at(b, o) as usize;
            if v >= image.0 && v < image.1 && census.len() < MAX_R && decode_record(&b[o..o + RECORD]).is_some() {
                census.push((cell, v));
            }
            if o % 16 != 0 {
                continue;
            }
            let (x, y, z) = (f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8));
            if (x - player.x).abs() < ENTITY_REACH && (y - player.y).abs() < ENTITY_REACH && (z - player.z).abs() < ENTITY_REACH
                && cell >= 0x90 && entities.len() < MAX_E
            {
                let e = cell - 0x90;
                let mut eb = [0u8; 0xA0];
                if win::read(e, &mut eb) {
                    let rows = entity_rows(&eb);
                    let vt = u64_at(&eb, 0) as usize;
                    let arch = u64_at(&eb, 0x20);
                    if vt >= image.0 && vt < image.1 && rows_ok(&rows) && heap_ptr(arch, image) && archetype_ok(arch as usize, image).is_some() {
                        entities.push(Entity { addr: e, arch: arch as usize, rows, kind: eb[0x28] });
                    }
                }
            }
        }
        true
    });
    let mut kinds: HashMap<u8, usize> = HashMap::new();
    for e in &entities {
        *kinds.entry(e.kind).or_default() += 1;
    }
    let mut vts: HashMap<usize, usize> = HashMap::new();
    for (_, vt) in &census {
        *vts.entry(*vt).or_default() += 1;
    }
    let mut vts: Vec<(usize, usize)> = vts.into_iter().collect();
    vts.sort_by(|a, b| b.1.cmp(&a.1));
    let archs: HashSet<usize> = entities.iter().map(|e| e.arch).collect();
    say!(
        "ladders: heap pass: {} entity-shaped objects near the player (type byte: count {:?}) on {} archetypes; {} ladder-shaped records in memory, by vtable (image offset, count) {:x?} ({:.1} s)",
        entities.len(), kinds, archs.len(), census.len(),
        vts.iter().take(6).map(|(v, n)| (v - image.0, *n)).collect::<Vec<_>>(), t0.elapsed().as_secs_f32()
    );

    // 2. Each archetype's ladders: the direct route, else the walk.
    let mut by_arch: HashMap<usize, Vec<Found>> = HashMap::new();
    let (mut lists, mut logged) = (0usize, 0usize);
    for &a in &archs {
        if let Some((found, head, n)) = direct(a, image) {
            lists += 1;
            if !found.is_empty() {
                if logged < 4 {
                    logged += 1;
                    say!("ladders: model {:#010x}: 2dfx list head {} ({} entries read), {} ladders", rd::<u32>(a + 0x18).unwrap_or(0), hex(&head), n, found.len());
                }
                by_arch.insert(a, found);
            }
        }
    }
    let direct_n: usize = by_arch.values().map(|f| f.len()).sum();
    say!(
        "ladders: direct route (archetype +{ARCH_2DFX:#x}): {lists} archetypes with a 2dfx list, {} with ladders, {direct_n} ladders ({:.1} s)",
        by_arch.len(), t0.elapsed().as_secs_f32()
    );
    if by_arch.is_empty() {
        let mut paths: HashMap<Vec<u16>, HashSet<usize>> = HashMap::new();
        for &a in &archs {
            let found = walk(a, image);
            for f in &found {
                paths.entry(f.path.clone()).or_default().insert(f.record);
            }
            if !found.is_empty() {
                by_arch.insert(a, found);
            }
        }
        let mut ranked: Vec<(Vec<u16>, HashSet<usize>)> = paths.into_iter().collect();
        ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        for (path, recs) in ranked.iter().take(8) {
            say!("ladders: walk: path archetype {} -> ladder record: {} different ladders", path_str(path), recs.len());
        }
        say!("ladders: walk: {} archetypes with ladders ({:.1} s)", by_arch.len(), t0.elapsed().as_secs_f32());
    }
    let reached: HashSet<usize> = by_arch.values().flatten().map(|f| f.record).collect();
    let in_census = census.iter().filter(|(c, _)| reached.contains(c)).count();
    say!("ladders: {} ladder records reached from nearby models, {} of them in the census", reached.len(), in_census);

    // 3. Placed through each entity.
    let mut out: Vec<(GtaLadder, Found, u32)> = vec![];
    let mut placed = HashSet::new();
    for e in &entities {
        let Some(found) = by_arch.get(&e.arch) else { continue };
        let hash = rd::<u32>(e.arch + 0x18).unwrap_or(0);
        for f in found {
            let g = place(&f.ladder, &e.rows, e.addr);
            let key = (g.bottom.x.to_bits(), g.bottom.y.to_bits(), g.bottom.z.to_bits());
            if placed.insert(key) {
                out.push((g, f.clone(), hash));
            }
        }
    }
    out.sort_by(|a, b| a.0.bottom.distance(player).total_cmp(&b.0.bottom.distance(player)));
    for (g, f, hash) in out.iter().take(12) {
        let mut raw = [0u8; RECORD];
        win::read(f.record, &mut raw);
        say!(
            "ladders: {:.1} m away: bottom {:.2} top {:.2} normal {:.2}, model {hash:#010x}, path {}, record {:#x}: position {:.2}, tail +0x3c {}",
            g.bottom.distance(player), g.bottom, g.top, g.normal, path_str(&f.path), f.record, vec3_at(&raw, AT_POSITION), hex(&raw[0x3C..])
        );
    }
    say!("ladders: done ({:.1} s)", t0.elapsed().as_secs_f32());
    out.into_iter().map(|(g, _, _)| g).collect()
}

/// Debug: logs the ladders GTA has loaded within 30 m of `player` (GTA position) to
/// FaithRunnerV.log, with everything the search learnt on the way.
pub fn debug_log_near(player: Vec3) -> Vec<GtaLadder> {
    log!("ladders: searching memory for the ladders near {player}");
    let near: Vec<GtaLadder> = discover(player).into_iter().filter(|l| l.bottom.distance(player).min(l.top.distance(player)) <= LOG_RADIUS).collect();
    for l in &near {
        log!(
            "ladders: ladder bottom {:.2} top {:.2} (rise {:.2} m) normal {:.2} get-off-top {} (entity {:#x}, {:.1} m away)",
            l.bottom, l.top, l.top.z - l.bottom.z, l.normal, l.can_get_off_top, l.entity, l.bottom.distance(player)
        );
    }
    log!("ladders: {} within {LOG_RADIUS} m", near.len());
    near
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 m wall (GTA y = 10) with a roof behind it; GTA's normal pointing into the wall and
    /// its top half a metre short: the fit turns the normal round, puts the foot on the wall
    /// and the top on the roof.
    #[test]
    fn fit_finds_wall_side_and_roof() {
        let q = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| [[a, b, c], [a, c, d]];
        let mut t = vec![];
        t.extend(q(Vec3::new(-5.0, 0.0, -10.0), Vec3::new(5.0, 0.0, -10.0), Vec3::new(5.0, 5.0, -10.0), Vec3::new(-5.0, 5.0, -10.0)));
        t.extend(q(Vec3::new(-5.0, 5.0, -10.0), Vec3::new(5.0, 5.0, -10.0), Vec3::new(5.0, 5.0, -14.0), Vec3::new(-5.0, 5.0, -14.0)));
        let world = faith_move::MeshWorld::new(t, vec![]);
        let g = GtaLadder { bottom: Vec3::new(0.0, 9.8, 0.0), top: Vec3::new(0.0, 9.8, 4.5), normal: Vec3::Y, can_get_off_top: true, entity: 0 };
        let f = fit(&g, Vec3::ZERO, &world);
        assert!(f.normal.z > 0.99, "{f:?}");
        assert!((f.base.z + 10.0).abs() < 0.08, "{f:?}");
        assert!((f.top - 5.0).abs() < 0.08, "{f:?}");
    }

    fn put(b: &mut [u8], o: usize, v: Vec3) {
        for (k, f) in v.to_array().iter().enumerate() {
            b[o + 4 * k..o + 4 * k + 4].copy_from_slice(&f.to_le_bytes());
        }
    }

    #[test]
    fn joaat_matches_gta() {
        assert_eq!(joaat("adder"), 0xB779A091);
        assert_eq!(joaat("ADDER"), 0xB779A091, "lower-cased");
        assert_ne!(joaat_raw("ADDER"), 0xB779A091);
    }

    /// The Strawberry ladder's record as run 2 logged it (0x1e11495cfe0, model 0xf049f9e6).
    const RUN2: &str = "b06ea0d1f77f0000c0500640 0041a6c1a0af434000000000c0500640 0041a6c160504cc0c0500640 0041a6c1a0af4340582a443f 067c24bf00000000d546dfe4 017443e95da32046";

    fn run2() -> Vec<u8> {
        let h: String = RUN2.split_whitespace().collect();
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn strawberry_record_decodes() {
        let b = run2();
        assert_eq!(b.len(), RECORD);
        let l = decode_record(&b).unwrap();
        assert!((l.bottom - Vec3::new(2.0987, -20.7817, -3.1924)).length() < 1e-3);
        assert!((l.top.z - l.bottom.z - 6.25).abs() < 1e-3, "rise 6.25 m");
        assert!((l.normal - Vec3::new(0.7663, -0.6425, 0.0)).length() < 1e-3);
        assert_eq!(vec3_at(&b, AT_POSITION), l.top, "position is the top here");
    }

    #[test]
    fn strict_rejects_near_misses() {
        let mut b = run2();
        put(&mut b, AT_TOP, Vec3::new(2.12, -20.7817, 3.0576)); // 2 cm off vertical
        assert!(decode_record(&b).is_none());
        let mut b = run2();
        put(&mut b, AT_TOP, Vec3::new(2.0987, -20.7817, -2.2)); // under 1.5 m
        assert!(decode_record(&b).is_none());
        let mut b = run2();
        put(&mut b, AT_NORMAL, Vec3::new(0.7663, -0.6425, 0.05)); // tilted normal
        assert!(decode_record(&b).is_none());
        assert!(decode_record(&[0u8; RECORD]).is_none());
    }

    #[test]
    fn entity_rows_checked() {
        let good = [Vec3::Y, -Vec3::X, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)];
        assert!(rows_ok(&good));
        assert!(!rows_ok(&[Vec3::X, Vec3::X, Vec3::Z, Vec3::ZERO]), "not at right angles");
        assert!(!rows_ok(&[Vec3::ZERO, Vec3::Y, Vec3::Z, Vec3::ZERO]), "zero axis");
    }

    #[test]
    fn placed_and_turned_for_faith() {
        // Entity turned 90 degrees (its x axis is GTA's y) at (100, 200, 30).
        let rows = [Vec3::Y, -Vec3::X, Vec3::Z, Vec3::new(100.0, 200.0, 30.0)];
        let l = LocalLadder { bottom: Vec3::new(1.0, 0.0, 0.0), top: Vec3::new(1.0, 0.0, 5.0), normal: Vec3::X, material: None, can_get_off_top: Some(false), can_get_off_bottom: None };
        let g = place(&l, &rows, 0);
        assert_eq!(g.bottom, Vec3::new(100.0, 201.0, 30.0));
        assert_eq!(g.normal, Vec3::Y);
        let f = to_faith(&g, Vec3::new(100.0, 200.0, 30.0));
        assert_eq!(f.base, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(f.top, 5.0);
        assert_eq!(f.normal, -Vec3::Z, "GTA north is Faith's -Z");
        assert!(!f.exit && !f.pipe);
    }
}
