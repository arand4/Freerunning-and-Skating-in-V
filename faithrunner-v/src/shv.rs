//! ScriptHookV, looked up at run time from the already loaded ScriptHookV.dll
//! (no SDK library needed), and a small native-call helper.
use std::ffi::{CString, c_void};
use std::sync::OnceLock;

use crate::win;

pub struct Shv {
    pub script_register: unsafe extern "C" fn(*mut c_void, extern "C" fn()),
    pub script_unregister: unsafe extern "C" fn(*mut c_void),
    init: unsafe extern "C" fn(u64),
    push: unsafe extern "C" fn(u64),
    call: unsafe extern "C" fn() -> *mut u64,
    pub wait: unsafe extern "C" fn(u32),
    pub handle_address: unsafe extern "C" fn(i32) -> *mut u8,
    pub all_peds: unsafe extern "C" fn(*mut i32, i32) -> i32,
    pub all_vehicles: unsafe extern "C" fn(*mut i32, i32) -> i32,
    pub all_objects: unsafe extern "C" fn(*mut i32, i32) -> i32,
    pub present_register: unsafe extern "C" fn(extern "C" fn(*mut c_void)),
    pub present_unregister: unsafe extern "C" fn(extern "C" fn(*mut c_void)),
}

static SHV: OnceLock<Option<Shv>> = OnceLock::new();

pub fn load() -> Option<&'static Shv> {
    SHV.get_or_init(|| unsafe {
        // Other ASI mods usually pull ScriptHookV in first; when Faith Runner V is the only one, it must
        // load it itself (found in the game folder, next to GTA5.exe), or nothing runs.
        let mut m = win::GetModuleHandleA(c"ScriptHookV.dll".as_ptr());
        if m.is_null() {
            m = win::LoadLibraryA(c"ScriptHookV.dll".as_ptr());
        }
        if m.is_null() {
            return None;
        }
        macro_rules! f {
            ($name:literal) => {{
                let p = win::GetProcAddress(m, CString::new($name).unwrap().as_ptr());
                if p.is_null() {
                    return None;
                }
                std::mem::transmute(p)
            }};
        }
        Some(Shv {
            script_register: f!("?scriptRegister@@YAXPEAUHINSTANCE__@@P6AXXZ@Z"),
            script_unregister: f!("?scriptUnregister@@YAXPEAUHINSTANCE__@@@Z"),
            init: f!("?nativeInit@@YAX_K@Z"),
            push: f!("?nativePush64@@YAX_K@Z"),
            call: f!("?nativeCall@@YAPEA_KXZ"),
            wait: f!("?scriptWait@@YAXK@Z"),
            handle_address: f!("?getScriptHandleBaseAddress@@YAPEAEH@Z"),
            all_peds: f!("?worldGetAllPeds@@YAHPEAHH@Z"),
            all_vehicles: f!("?worldGetAllVehicles@@YAHPEAHH@Z"),
            all_objects: f!("?worldGetAllObjects@@YAHPEAHH@Z"),
            present_register: f!("?presentCallbackRegister@@YAXP6AXPEAX@Z@Z"),
            present_unregister: f!("?presentCallbackUnregister@@YAXP6AXPEAX@Z@Z"),
        })
    })
    .as_ref()
}

pub fn shv() -> &'static Shv {
    load().expect("ScriptHookV")
}

/// A native argument, pushed as ScriptHookV's 64-bit slot (value in the low bytes).
pub trait Arg {
    fn bits(self) -> u64;
}
impl Arg for i32 {
    fn bits(self) -> u64 {
        self as u32 as u64
    }
}
impl Arg for u32 {
    fn bits(self) -> u64 {
        self as u64
    }
}
impl Arg for bool {
    fn bits(self) -> u64 {
        self as u64
    }
}
impl Arg for f32 {
    fn bits(self) -> u64 {
        self.to_bits() as u64
    }
}
impl Arg for *const i8 {
    fn bits(self) -> u64 {
        self as u64
    }
}

pub fn begin(hash: u64) {
    unsafe { (shv().init)(hash) }
}
pub fn push(a: impl Arg) {
    unsafe { (shv().push)(a.bits()) }
}
pub fn call() -> *mut u64 {
    unsafe { (shv().call)() }
}

/// `native!(HASH, args...)` calls a native and gives the result pointer.
#[macro_export]
macro_rules! native {
    ($hash:expr $(, $a:expr)* $(,)?) => {{
        $crate::shv::begin($hash);
        $( $crate::shv::push($a); )*
        $crate::shv::call()
    }};
}

pub fn ret_i32(p: *mut u64) -> i32 {
    unsafe { *(p as *const i32) }
}
pub fn ret_bool(p: *mut u64) -> bool {
    ret_i32(p) != 0
}
pub fn ret_f32(p: *mut u64) -> f32 {
    unsafe { *(p as *const f32) }
}
/// ScriptHookV's Vector3: x, y, z each in an 8-byte slot.
pub fn ret_vec3(p: *mut u64) -> glam::Vec3 {
    unsafe {
        let f = p as *const f32;
        glam::Vec3::new(*f, *f.add(2), *f.add(4))
    }
}

pub fn wait(ms: u32) {
    unsafe { (shv().wait)(ms) }
}
