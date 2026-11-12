#![allow(non_snake_case)]
//! Faith Runner V: Mirror's Edge parkour movement (Faith Runner's `faith_move`,
//! MIT, Copyright (c) 2026 Tanner) on GTA V Legacy's own collision (read live
//! the way SK8V does, Apache-2.0, becaring; reader by Sol4ra).
//!
//! Step 1: movement only. The GTA player is held (frozen, no gravity) and
//! placed every frame where Faith's controller puts her; a plain GTA clip
//! plays per move. No Mirror's Edge files are read yet.
//!
//! Loaded by ScriptHookV's ASI loader as FaithRunnerV.asi.
use std::ffi::c_void;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub mod audio;
pub mod body;
pub mod collision;
pub mod config;
pub mod dynamic;
pub mod ladders;
pub mod level;
pub mod menu;
pub mod live;
pub(crate) mod mode;
pub mod natives;
pub mod render;
pub mod posehook;
pub mod retarget;
pub mod third;
pub mod shv;
pub mod win;

static MODULE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn log_line(s: &str) {
    if let Ok(mut g) = LOG.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = writeln!(f, "{s}");
        let _ = f.flush();
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log_line(&format!($($t)*)) };
}

/// %LOCALAPPDATA%\FaithRunnerV (the game folder may not be writable).
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("FaithRunnerV")
}

/// The folder this ASI was loaded from (the GTA folder).
pub fn module_dir() -> PathBuf {
    let mut buf = [0u16; 1024];
    let n = unsafe { win::GetModuleFileNameW(MODULE.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void, buf.as_mut_ptr(), 1024) } as usize;
    let p = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    p.parent().map(|d| d.to_path_buf()).unwrap_or_default()
}

/// FaithRunnerV.log next to the ASI (the GTA folder); %LOCALAPPDATA%\FaithRunnerV if that
/// folder isn't writable.
fn open_log() {
    let near = module_dir().join("FaithRunnerV.log");
    let file = std::fs::File::create(&near).or_else(|_| {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        std::fs::File::create(dir.join("FaithRunnerV.log"))
    });
    if let Ok(f) = file {
        *LOG.lock().unwrap() = Some(f);
    }
}

extern "C" fn script_main() {
    let result = std::panic::catch_unwind(mode::run);
    if result.is_err() {
        log!("Faith Runner V stopped after an error (see above).");
    }
    // Never return to ScriptHookV: idle.
    loop {
        shv::wait(1000);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(module: *mut c_void, reason: u32, _: *mut c_void) -> i32 {
    const ATTACH: u32 = 1;
    const DETACH: u32 = 0;
    match reason {
        ATTACH => {
            MODULE.store(module as usize, std::sync::atomic::Ordering::Relaxed);
            unsafe { win::DisableThreadLibraryCalls(module) };
            open_log();
            let Some(s) = shv::load() else {
                log!("ScriptHookV.dll couldn't be loaded (is it in the GTA V folder?): Faith Runner V does nothing.");
                return 1;
            };
            std::panic::set_hook(Box::new(|info| log!("error: {info}")));
            log!("Faith Runner V {} loaded from {}", env!("CARGO_PKG_VERSION"), module_dir().display());
            unsafe { (s.script_register)(module, script_main) };
            unsafe { (s.present_register)(render::on_present) };
        }
        DETACH => {
            if let Some(s) = shv::load() {
                unsafe { (s.present_unregister)(render::on_present) };
                unsafe { (s.script_unregister)(module) };
            }
        }
        _ => {}
    }
    1
}
