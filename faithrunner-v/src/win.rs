//! The few Windows functions used, declared by hand.
#![allow(non_snake_case, clippy::upper_case_acronyms)]
use std::ffi::c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MemoryBasicInformation {
    pub base_address: usize,
    pub allocation_base: usize,
    pub allocation_protect: u32,
    pub partition_id: u16,
    pub region_size: usize,
    pub state: u32,
    pub protect: u32,
    pub kind: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct WorkingSetExInformation {
    pub virtual_address: usize,
    pub attributes: usize,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct XInputState {
    pub packet: u32,
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_PRIVATE: u32 = 0x20000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_GUARD: u32 = 0x100;
pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetModuleHandleA(name: *const i8) -> *mut c_void;
    pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    pub fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
    pub fn LoadLibraryA(name: *const i8) -> *mut c_void;
    pub fn GetModuleFileNameW(module: *mut c_void, out: *mut u16, size: u32) -> u32;
    pub fn VirtualQuery(address: *const c_void, out: *mut MemoryBasicInformation, size: usize) -> usize;
    pub fn ReadProcessMemory(process: isize, base: *const c_void, buffer: *mut c_void, size: usize, read: *mut usize) -> i32;
    pub fn GetCurrentProcess() -> isize;
    pub fn GetCurrentProcessId() -> u32;
    pub fn K32QueryWorkingSetEx(process: isize, info: *mut WorkingSetExInformation, size: u32) -> i32;
    pub fn DisableThreadLibraryCalls(module: *mut c_void) -> i32;
    pub fn VirtualAlloc(address: *const c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    pub fn VirtualProtect(address: *const c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    pub fn FlushInstructionCache(process: isize, address: *const c_void, size: usize) -> i32;
    pub fn GetCurrentThread() -> isize;
    pub fn SetThreadPriority(thread: isize, priority: i32) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    pub fn GetAsyncKeyState(vk: i32) -> i16;
    pub fn GetForegroundWindow() -> *mut c_void;
    pub fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
}

/// Reads this process's memory; false when any byte is unreadable.
pub fn read(address: usize, out: &mut [u8]) -> bool {
    let mut got = 0usize;
    address != 0
        && unsafe { ReadProcessMemory(GetCurrentProcess(), address as *const c_void, out.as_mut_ptr() as *mut c_void, out.len(), &mut got) } != 0
        && got == out.len()
}

pub fn read_u64(address: usize) -> Option<u64> {
    let mut b = [0u8; 8];
    read(address, &mut b).then(|| u64::from_le_bytes(b))
}

pub fn read_u16(address: usize) -> Option<u16> {
    let mut b = [0u8; 2];
    read(address, &mut b).then(|| u16::from_le_bytes(b))
}

/// GTA's main image (start, end).
pub fn gta_image() -> (usize, usize) {
    unsafe {
        let base = GetModuleHandleW(std::ptr::null()) as usize;
        let e_lfanew = *((base + 0x3C) as *const u32) as usize;
        let size = *((base + e_lfanew + 24 + 56) as *const u32) as usize;
        (base, base + size)
    }
}

/// Whether GTA's window has the keyboard (keys pressed in other apps are ignored).
pub fn game_focused() -> bool {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        pid == GetCurrentProcessId()
    }
}

pub fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

type XInputGetState = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

pub fn pad() -> Option<XInputState> {
    static FN: std::sync::OnceLock<Option<XInputGetState>> = std::sync::OnceLock::new();
    let f = (*FN.get_or_init(|| unsafe {
        let m = LoadLibraryA(c"xinput1_4.dll".as_ptr());
        if m.is_null() {
            return None;
        }
        let p = GetProcAddress(m, c"XInputGetState".as_ptr());
        (!p.is_null()).then(|| std::mem::transmute::<*mut c_void, XInputGetState>(p))
    }))?;
    let mut s = XInputState::default();
    (unsafe { f(0, &mut s) } == 0).then_some(s)
}
