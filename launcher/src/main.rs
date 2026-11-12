//! The launcher: one window with two cards.
//! - Parkour (Faith Runner V): installed from the copy beside the launcher (parkour\), or from the
//!   newest release on the mod's own GitHub once `parkourRepo` is set in launcher.json.
//! - SK8V (becaring, Apache-2.0): never shipped with the mod. The launcher downloads becaring's own
//!   release from his GitHub and runs his own setup steps (tools\get-python.ps1, tools\prepare.py),
//!   showing their progress in the card; returning players get his "-update.zip" (two files) instead.
//! Settings: launcher.json beside the exe (name, version, repos). What it remembers (folders, disc,
//! versions it installed): %LOCALAPPDATA%\<name>\launcher-state.json. Downloads: %LOCALAPPDATA%\<name>\downloads.
#![windows_subsystem = "windows"]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Theme, WindowBuilder};
use wry::http::{header::CONTENT_TYPE, Request, Response};
use wry::WebViewBuilder;

const UA: &str = "gta-mod-launcher";
const NO_WINDOW: u32 = 0x0800_0000;
const GTA_VERSION: &str = "1.0.3889.0";
/// SkateVLegacy.asi of each SK8V release (its files carry no version number). Releases that only
/// change SK8V's setup (1.1.10) keep the previous ASI.
const SK8V_HASHES: &[(&str, &str)] = &[
    ("4ad9ab5fb3688b3969ea315e4f3acfd5d1dd10ddc1702d50255dfc0064a953a0", "1.1.8"),
    ("e7d16c38ca92c1449746e80ad6118ee57da225c8deea32178750dfa9a60cac3d", "1.1.9"),
];

// ---------------------------------------------------------------- settings and state

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    name: String,
    version: String,
    parkour_repo: String,
    sk8v_repo: String,
    /// Where the parkour card's video comes from. Empty = media/parkour.mp4 in `parkour_repo` on GitHub.
    parkour_video: String,
    /// A donation page (Ko-fi etc.): shown as a small "Support" link in the footer when set.
    support: String,
}
impl Default for Config {
    fn default() -> Self {
        Config { name: "LS Flow".into(), version: "0.1.0-beta".into(), parkour_repo: String::new(), sk8v_repo: "becaring/SK8V".into(),
            parkour_video: String::new(), support: String::new() }
    }
}
impl Config {
    fn parkour_video_url(&self) -> Option<String> {
        let v = self.parkour_video.trim();
        if !v.is_empty() { return Some(v.to_string()); }
        let r = self.parkour_repo.trim();
        (!r.is_empty()).then(|| format!("https://raw.githubusercontent.com/{r}/HEAD/media/parkour.mp4"))
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    gta: String,
    mirrors: String,
    disc: String,
    stance: String,
    /// sha256 of an installed ASI -> the version it was installed as
    known: Vec<(String, String)>,
}

#[derive(Clone, Default, Serialize)]
struct Task {
    phase: String,
    pct: Option<f64>,
    detail: String,
}

#[derive(Clone, Default)]
struct Release {
    tag: String,
    date: String,
    notes: String,
    url: String,
    full: Option<(String, u64)>,
    update: Option<(String, u64)>,
}

#[derive(Default)]
struct Card {
    task: Option<Task>,
    error: String,
    message: String,
    log: Vec<String>,
}

#[derive(Default)]
struct Disk {
    gta_ok: bool,
    gta_version: String,
    scripthook: bool,
    /// DLLs in the GTA folder that load .asi mods (ScriptHookV's dinput8.dll, Ultimate ASI Loader under any name...)
    asi_loaders: Vec<String>,
    /// ASI loaders built for 32-bit games (GTA IV, San Andreas...): GTA V skips them, so they load nothing
    asi_loaders_32: Vec<String>,
    writable: bool,
    running: bool,
    parkour_sha: Option<String>,
    bundled_sha: Option<String>,
    sk8v_sha: Option<String>,
    sk8v_setup_done: bool,
    mirrors_ok: bool,
    /// Faith Runner's copy of the Mirror's Edge files it reads, in the GTA folder
    me_copied: bool,
}

struct App {
    cfg: Config,
    dir: PathBuf,
    data: PathBuf,
    saved: Saved,
    disk: Disk,
    parkour: Card,
    sk8v: Card,
    parkour_remote: Option<Release>,
    sk8v_remote: Option<Release>,
    /// newest SK8V release that changed the game files (has an update zip, or is the first release)
    sk8v_files_tag: String,
    checking: bool,
    check_error: String,
    child: Option<u32>,
    cancelled: bool,
}

enum Ev {
    Push,
    Quit,
}

struct Ctx {
    app: Mutex<App>,
    proxy: Mutex<EventLoopProxy<Ev>>,
}
type Shared = Arc<Ctx>;

fn push(c: &Shared) {
    let _ = c.proxy.lock().unwrap().send_event(Ev::Push);
}

impl App {
    fn gta(&self) -> PathBuf {
        PathBuf::from(&self.saved.gta)
    }
    fn save(&self) {
        let _ = fs::create_dir_all(&self.data);
        let _ = fs::write(self.data.join("launcher-state.json"), serde_json::to_string_pretty(&self.saved).unwrap());
    }
    fn known(&self, sha: &str) -> Option<String> {
        self.saved.known.iter().rev().find(|(s, _)| s == sha).map(|(_, v)| v.clone())
            .or_else(|| SK8V_HASHES.iter().find(|(s, _)| *s == sha).map(|(_, v)| v.to_string()))
    }
    fn remember(&mut self, sha: Option<String>, version: &str) {
        if let Some(sha) = sha {
            self.saved.known.retain(|(s, _)| *s != sha);
            self.saved.known.push((sha, version.to_string()));
            self.save();
        }
    }
    /// Re-reads everything on disk the cards show.
    fn refresh(&mut self) {
        let g = self.gta();
        let d = &mut self.disk;
        d.gta_ok = g.join("GTA5.exe").is_file();
        d.gta_version = if d.gta_ok { file_version(&g.join("GTA5.exe")).unwrap_or_default() } else { String::new() };
        d.scripthook = g.join("ScriptHookV.dll").is_file();
        let (l64, l32) = if d.gta_ok { asi_loaders(&g) } else { (vec![], vec![]) };
        d.asi_loaders = l64;
        d.asi_loaders_32 = l32;
        d.writable = d.gta_ok && writable(&g);
        d.parkour_sha = sha256(&g.join("FaithRunnerV.asi"));
        d.bundled_sha = sha256(&self.dir.join("parkour").join("FaithRunnerV.asi"));
        d.sk8v_sha = sha256(&g.join("SkateVLegacy.asi"));
        d.sk8v_setup_done = g.join("SkateVLegacy.installed.json").is_file() && g.join("SK8V").is_dir();
        d.mirrors_ok = mirrors_ok(Path::new(&self.saved.mirrors));
        d.me_copied = d.gta_ok && mirrors_ok(&g.join(ME_COPY));
    }

    fn parkour_installed_version(&self) -> Option<String> {
        let sha = self.disk.parkour_sha.as_ref()?;
        Some(self.known(sha).unwrap_or_else(|| {
            if Some(sha) == self.disk.bundled_sha.as_ref() { self.cfg.version.clone() } else { "unknown".into() }
        }))
    }
    fn parkour_available(&self) -> Option<String> {
        if let Some(r) = &self.parkour_remote { return Some(r.tag.clone()); }
        self.disk.bundled_sha.as_ref().map(|_| self.cfg.version.clone())
    }
    fn sk8v_installed_version(&self) -> Option<String> {
        let sha = self.disk.sk8v_sha.as_ref()?;
        Some(self.known(sha).unwrap_or_else(|| "unknown".into()))
    }

    fn snapshot(&self) -> Value {
        let d = &self.disk;
        let busy = self.parkour.task.is_some() || self.sk8v.task.is_some();
        let p_inst = self.parkour_installed_version();
        let p_avail = self.parkour_available();
        let p_update = match (&p_inst, &p_avail) {
            (Some(i), Some(a)) => i != a,
            _ => false,
        };
        let s_inst = self.sk8v_installed_version();
        let r = self.sk8v_remote.clone().unwrap_or_default();
        // An installed SK8V is current when its ASI is the newest that changed the game files.
        let s_current = match &s_inst {
            Some(v) if v != "unknown" && !self.sk8v_files_tag.is_empty() => cmp_ver(v, &self.sk8v_files_tag) >= 0,
            Some(v) if v != "unknown" && !r.tag.is_empty() => cmp_ver(v, &r.tag) >= 0,
            _ => false,
        };
        let s_update = s_inst.is_some() && self.sk8v_remote.is_some() && !s_current;
        let can_patch = s_update && d.sk8v_setup_done && r.update.is_some()
            && s_inst.as_deref().is_some_and(|v| v != "unknown");
        let release_dir = self.sk8v_release_dir(&r.tag);
        json!({
            "name": self.cfg.name, "version": self.cfg.version,
            "busy": busy, "checking": self.checking, "checkError": self.check_error,
            "gta": { "path": self.saved.gta, "ok": d.gta_ok, "version": d.gta_version,
                     "versionOk": d.gta_version == GTA_VERSION, "scripthook": d.scripthook, "asiLoaders": d.asi_loaders, "asiLoaders32": d.asi_loaders_32,
                     "writable": d.writable, "running": d.running },
            "parkour": { "installed": p_inst, "available": p_avail, "update": p_update,
                         "source": if self.parkour_remote.is_some() { "github" } else { "bundled" },
                         "repo": self.cfg.parkour_repo, "mirrors": self.saved.mirrors, "mirrorsOk": d.mirrors_ok, "mirrorsCopied": d.me_copied,
                         "task": self.parkour.task, "error": self.parkour.error, "message": self.parkour.message,
                         "log": self.parkour.log },
            "sk8v": { "installed": s_inst, "setupDone": d.sk8v_setup_done, "latest": r.tag, "date": r.date,
                      "notes": r.notes, "url": r.url, "filesTag": self.sk8v_files_tag,
                      "current": s_current, "update": s_update, "canPatch": can_patch,
                      "fullSize": r.full.as_ref().map(|f| f.1), "updateSize": r.update.as_ref().map(|f| f.1),
                      "downloaded": release_dir.as_ref().is_some_and(|p| p.join("tools").join("prepare.py").is_file()),
                      "pythonReady": release_dir.as_ref().is_some_and(|p| p.join("python").join("python.exe").is_file()),
                      "disc": self.saved.disc, "stance": if self.saved.stance.is_empty() { "Regular" } else { &self.saved.stance },
                      "repo": self.cfg.sk8v_repo,
                      "task": self.sk8v.task, "error": self.sk8v.error, "message": self.sk8v.message, "log": self.sk8v.log },
        })
    }

    fn sk8v_release_dir(&self, tag: &str) -> Option<PathBuf> {
        (!tag.is_empty()).then(|| self.data.join("SK8V").join(tag))
    }
}

// ---------------------------------------------------------------- small helpers

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn file_version(path: &Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let p = wide(&path.to_string_lossy());
    unsafe {
        let mut h = 0u32;
        let size = GetFileVersionInfoSizeW(p.as_ptr(), &mut h);
        if size == 0 { return None; }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(p.as_ptr(), 0, size, buf.as_mut_ptr().cast()) == 0 { return None; }
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let root = wide("\\");
        if VerQueryValueW(buf.as_ptr().cast(), root.as_ptr(), &mut ptr, &mut len) == 0 || ptr.is_null() { return None; }
        let fi = &*(ptr as *const VS_FIXEDFILEINFO);
        Some(format!("{}.{}.{}.{}", fi.dwFileVersionMS >> 16, fi.dwFileVersionMS & 0xFFFF, fi.dwFileVersionLS >> 16, fi.dwFileVersionLS & 0xFFFF))
    }
}

fn sha256(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".launcher-write-test-{}", std::process::id()));
    match File::create(&probe) {
        Ok(_) => { let _ = fs::remove_file(&probe); true }
        Err(_) => false,
    }
}

/// "1.1.10" vs "1.1.9" (numbers, not text); a leading v is ignored.
fn cmp_ver(a: &str, b: &str) -> i32 {
    let parts = |s: &str| s.trim_start_matches(['v', 'V']).split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if p != q { return if p > q { 1 } else { -1 }; }
    }
    0
}

/// Where the launcher copies the Mirror's Edge files Faith Runner reads (inside the GTA folder).
const ME_COPY: &str = r"FaithRunnerV\MirrorsEdge";
/// Every file Faith Runner V reads from Mirror's Edge's TdGame\CookedPC (found by tracing its
/// loaders: body, first- and third-person animations, textures and sounds), about 65 MB.
/// The first two are required; the rest add the legs' materials, the disarm clips and sounds.
const ME_FILES: [&str; 16] = [
    r"Characters\CH_TKY_Crim_Fixer_1P.upk",
    r"Animations\AS_C1P_Unarmed.upk",
    r"Animations\AS_C1P_OneHanded_Common.upk",
    r"Animations\AS_F3P_Unarmed.upk",
    r"Characters\CH_TKY_Crim_Fixer.upk",
    r"Characters\CH_Faith_Cinematic.upk",
    r"Audio\A_Bodyfalls.upk",
    r"Audio\A_Character_Disarm.upk",
    r"Audio\A_Character_Effects.upk",
    r"Audio\A_Character_Female_01.upk",
    r"Audio\A_Character_Melee.upk",
    r"Audio\A_Material_Footstep.upk",
    r"Audio\A_Material_Handstep.upk",
    r"Audio\A_Props_Interactive.upk",
    r"Audio\A_WP_Handling.upk",
    r"Audio\int\A_VO_AI_CUE.upk",
];

/// The CookedPC folder of a Mirror's Edge install (or of a copy), if it has Faith in it.
fn me_cooked(dir: &Path) -> Option<PathBuf> {
    if dir.as_os_str().is_empty() { return None; }
    [dir.join("TdGame").join("CookedPC"), dir.join("CookedPC"), dir.to_path_buf()].into_iter().find(|c| {
        c.join("Characters").join("CH_TKY_Crim_Fixer_1P.upk").is_file() && c.join("Animations").join("AS_C1P_Unarmed.upk").is_file()
    })
}

/// Copies ME_FILES from the player's Mirror's Edge into the GTA folder (files already there with
/// the same size are kept). Returns how many were copied.
fn copy_me_files(c: &Shared, mirrors: &Path, gta: &Path) -> Result<usize, String> {
    let src = me_cooked(mirrors).ok_or("That Mirror's Edge folder has no Faith files in it.")?;
    let dst = gta.join(ME_COPY).join("TdGame").join("CookedPC");
    let total: u64 = ME_FILES.iter().filter_map(|f| fs::metadata(src.join(f)).ok()).map(|m| m.len()).sum();
    let (mut done, mut copied) = (0u64, 0usize);
    for f in ME_FILES {
        let from = src.join(f);
        let Ok(m) = fs::metadata(&from) else { continue };
        let to = dst.join(f);
        set_task(c, Which::Parkour, "Copying Mirror's Edge files", Some(done as f64 / total.max(1) as f64 * 100.0), f);
        if fs::metadata(&to).map(|t| t.len()) .ok() != Some(m.len()) {
            fs::create_dir_all(to.parent().unwrap()).map_err(|e| format!("Couldn't make {}: {e}", to.parent().unwrap().display()))?;
            fs::copy(&from, &to).map_err(|e| format!("Copying {f} failed: {e}"))?;
            copied += 1;
        }
        done += m.len();
    }
    if me_cooked(&gta.join(ME_COPY)).is_none() { return Err("Copying Faith's files from Mirror's Edge failed.".into()); }
    Ok(copied)
}

fn mirrors_ok(dir: &Path) -> bool {
    me_cooked(dir).is_some()
}

fn reg(root: winreg::HKEY, key: &str, value: &str) -> Option<String> {
    winreg::RegKey::predef(root).open_subkey(key).ok()?.get_value::<String, _>(value).ok()
}

fn steam_libraries() -> Vec<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    let Some(steam) = reg(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath") else { return vec![] };
    let steam = PathBuf::from(steam.replace('/', "\\"));
    let mut libs = vec![steam.clone()];
    if let Ok(vdf) = fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in vdf.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("\"path\"") {
                let p = rest.trim().trim_matches('"').replace("\\\\", "\\");
                if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&p)) { libs.push(PathBuf::from(p)); }
            }
        }
    }
    libs
}

fn find_gta() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    for (k, v) in [(r"SOFTWARE\WOW6432Node\Rockstar Games\Grand Theft Auto V", "InstallFolder"),
                   (r"SOFTWARE\WOW6432Node\Rockstar Games\GTAV", "InstallFolderSteam"),
                   (r"SOFTWARE\WOW6432Node\Rockstar Games\GTAV", "InstallFolder"),
                   (r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 271590", "InstallLocation")] {
        if let Some(p) = reg(HKEY_LOCAL_MACHINE, k, v) {
            if Path::new(&p).join("GTA5.exe").is_file() { return p.trim_end_matches('\\').to_string(); }
        }
    }
    for l in steam_libraries() {
        let d = l.join(r"steamapps\common\Grand Theft Auto V");
        if d.join("GTA5.exe").is_file() { return d.to_string_lossy().into(); }
    }
    String::new()
}

fn find_mirrors() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    for (k, v) in [(r"SOFTWARE\WOW6432Node\EA Games\Mirror's Edge", "Install Dir"),
                   (r"SOFTWARE\EA Games\Mirror's Edge", "Install Dir"),
                   (r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 17410", "InstallLocation")] {
        if let Some(p) = reg(HKEY_LOCAL_MACHINE, k, v) {
            if mirrors_ok(Path::new(&p)) { return p.trim_end_matches('\\').to_string(); }
        }
    }
    for l in steam_libraries() {
        let d = l.join(r"steamapps\common\mirrors edge");
        if mirrors_ok(&d) { return d.to_string_lossy().into(); }
    }
    String::new()
}

/// Every ASI loader searches the game folder for "*.asi", so that text (plain or UTF-16) is inside each one,
/// whatever it is named; none of the game's own DLLs contain it.
/// Returns (64-bit loaders, 32-bit loaders); only 64-bit ones work in GTA V.
fn asi_loaders(dir: &Path) -> (Vec<String>, Vec<String>) {
    let (mut found, mut found32) = (vec![], vec![]);
    let Ok(entries) = fs::read_dir(dir) else { return (found, found32) };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !name.to_lowercase().ends_with(".dll") || e.metadata().map(|m| m.len() > 8 << 20).unwrap_or(true) { continue; }
        let Ok(b) = fs::read(&p) else { continue };
        let lower: Vec<u8> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
        let wide: Vec<u8> = b"*.asi".iter().flat_map(|&c| [c, 0]).collect();
        if lower.windows(5).any(|w| w == b"*.asi") || lower.windows(wide.len()).any(|w| w == wide.as_slice()) {
            // PE header: e_lfanew at 0x3C, then "PE\0\0" and the machine type (0x8664 = x64).
            let pe = b.get(0x3C..0x40).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).unwrap_or(0);
            let x64 = b.get(pe..pe + 6).is_some_and(|h| &h[..4] == b"PE\0\0" && u16::from_le_bytes([h[4], h[5]]) == 0x8664);
            if x64 { found.push(name) } else { found32.push(name) }
        }
    }
    found.sort();
    found32.sort();
    (found, found32)
}

fn process_running(name: &str) -> bool {
    Command::new("tasklist").args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"]).creation_flags(NO_WINDOW)
        .output().map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&name.to_lowercase())).unwrap_or(false)
}

fn gta_running() -> bool {
    process_running("GTA5.exe")
}

fn shell_open(target: &str, verb: &str, params: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let (v, t, p) = (wide(verb), wide(target), wide(params));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), v.as_ptr(), t.as_ptr(), if params.is_empty() { std::ptr::null() } else { p.as_ptr() }, std::ptr::null(), 1) };
    r as usize > 32
}

fn mb(n: u64) -> String {
    format!("{:.0} MB", n as f64 / 1_048_576.0)
}

// ---------------------------------------------------------------- GitHub

fn get_json(url: &str) -> Result<Value, String> {
    let mut r = ureq::get(url).header("User-Agent", UA).header("Accept", "application/vnd.github+json")
        .call().map_err(|e| format!("GitHub: {e}"))?;
    r.body_mut().read_json::<Value>().map_err(|e| e.to_string())
}

fn release_from(v: &Value) -> Release {
    let mut r = Release {
        tag: v["tag_name"].as_str().unwrap_or("").to_string(),
        date: v["published_at"].as_str().unwrap_or("").chars().take(10).collect(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        url: v["html_url"].as_str().unwrap_or("").to_string(),
        ..Default::default()
    };
    for a in v["assets"].as_array().into_iter().flatten() {
        let name = a["name"].as_str().unwrap_or("").to_lowercase();
        let item = (a["browser_download_url"].as_str().unwrap_or("").to_string(), a["size"].as_u64().unwrap_or(0));
        if !name.ends_with(".zip") { continue; }
        if name.contains("-update") { r.update = Some(item); } else if r.full.is_none() { r.full = Some(item); }
    }
    r
}

fn check(c: Shared) {
    {
        let mut a = c.app.lock().unwrap();
        if a.checking { return; }
        a.checking = true;
        a.check_error.clear();
    }
    push(&c);
    let (sk8v_repo, parkour_repo) = { let a = c.app.lock().unwrap(); (a.cfg.sk8v_repo.clone(), a.cfg.parkour_repo.clone()) };
    let mut errors = vec![];
    match get_json(&format!("https://api.github.com/repos/{sk8v_repo}/releases?per_page=15")) {
        Ok(list) => {
            let rels: Vec<Release> = list.as_array().into_iter().flatten()
                .filter(|v| !v["draft"].as_bool().unwrap_or(false) && !v["prerelease"].as_bool().unwrap_or(false))
                .map(release_from).collect();
            let files_tag = rels.iter().find(|r| r.update.is_some()).map(|r| r.tag.clone()).unwrap_or_default();
            let mut a = c.app.lock().unwrap();
            a.sk8v_remote = rels.first().cloned();
            a.sk8v_files_tag = files_tag;
        }
        Err(e) => errors.push(format!("SK8V: {e}")),
    }
    if !parkour_repo.trim().is_empty() {
        match get_json(&format!("https://api.github.com/repos/{}/releases/latest", parkour_repo.trim())) {
            Ok(v) => { let r = release_from(&v); c.app.lock().unwrap().parkour_remote = r.full.is_some().then_some(r); }
            Err(e) if e.contains("404") => {} // no release published yet: the copy beside the launcher is used
            Err(e) => errors.push(format!("Faith Runner: {e}")),
        }
    }
    let mut a = c.app.lock().unwrap();
    a.checking = false;
    a.check_error = errors.join("  ");
    drop(a);
    push(&c);
}

/// Downloads to `dest`, reporting progress into the card's task.
fn download(c: &Shared, card: Which, url: &str, dest: &Path, what: &str) -> Result<(), String> {
    let resp = ureq::get(url).header("User-Agent", UA).call().map_err(|e| format!("download failed: {e}"))?;
    let total = resp.body().content_length();
    let mut r = resp.into_body().into_reader();
    if let Some(p) = dest.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
    let part = dest.with_extension("part");
    let mut f = File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 16];
    let (mut got, mut last, start) = (0u64, Instant::now(), Instant::now());
    loop {
        if c.app.lock().unwrap().cancelled { drop(f); let _ = fs::remove_file(&part); return Err("Cancelled.".into()); }
        let n = r.read(&mut buf).map_err(|e| format!("download failed: {e}"))?;
        if n == 0 { break; }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if last.elapsed() > Duration::from_millis(200) {
            last = Instant::now();
            let speed = got as f64 / start.elapsed().as_secs_f64().max(0.1);
            let detail = match total {
                Some(t) => format!("{} of {}  ·  {:.1} MB/s", mb(got), mb(t), speed / 1_048_576.0),
                None => format!("{}  ·  {:.1} MB/s", mb(got), speed / 1_048_576.0),
            };
            set_task(c, card, what, total.map(|t| got as f64 / t as f64 * 100.0), &detail);
        }
    }
    drop(f);
    fs::rename(&part, dest).map_err(|e| e.to_string())
}

fn unzip(zip_path: &Path, to: &Path, strip_top: bool) -> Result<(), String> {
    let f = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = e.enclosed_name() else { continue };
        let rel: PathBuf = if strip_top { name.components().skip(1).collect() } else { name.to_path_buf() };
        if rel.as_os_str().is_empty() { continue; }
        let out = to.join(rel);
        if e.is_dir() { fs::create_dir_all(&out).map_err(|e| e.to_string())?; continue; }
        if let Some(p) = out.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        let mut o = File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut e, &mut o).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- tasks

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Parkour,
    Sk8v,
}

fn card(a: &mut App, w: Which) -> &mut Card {
    match w { Which::Parkour => &mut a.parkour, Which::Sk8v => &mut a.sk8v }
}

fn set_task(c: &Shared, w: Which, phase: &str, pct: Option<f64>, detail: &str) {
    card(&mut c.app.lock().unwrap(), w).task = Some(Task { phase: phase.into(), pct, detail: detail.into() });
    push(c);
}

fn log(c: &Shared, w: Which, line: &str) {
    let mut a = c.app.lock().unwrap();
    let l = &mut card(&mut a, w).log;
    l.push(line.to_string());
    if l.len() > 400 { l.drain(..100); }
}

/// Runs `job` on its own thread as the card's task; its error (or message) ends up on the card.
fn run_task(c: Shared, w: Which, job: impl FnOnce(&Shared) -> Result<String, String> + Send + 'static) {
    {
        let mut a = c.app.lock().unwrap();
        if a.parkour.task.is_some() || a.sk8v.task.is_some() { return; }
        a.cancelled = false;
        let k = card(&mut a, w);
        k.error.clear();
        k.message.clear();
        k.task = Some(Task { phase: "Starting".into(), pct: None, detail: String::new() });
    }
    push(&c);
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&c))).unwrap_or_else(|_| Err("Something went wrong (internal error).".into()));
        let mut a = c.app.lock().unwrap();
        a.child = None;
        a.refresh();
        let k = card(&mut a, w);
        k.task = None;
        match r { Ok(m) => k.message = m, Err(e) => k.error = e }
        drop(a);
        push(&c);
    });
}

/// What stops an install into the GTA folder.
fn gta_problems(a: &App, need_scripthook: bool) -> Result<(), String> {
    let d = &a.disk;
    if !d.gta_ok { return Err("Pick your GTA V folder (the one with GTA5.exe) first.".into()); }
    if d.gta_version != GTA_VERSION { return Err(format!("This GTA5.exe is version {}. Both mods need GTA V Legacy {GTA_VERSION}.", d.gta_version)); }
    if need_scripthook && !d.scripthook { return Err("ScriptHookV isn't installed in the GTA V folder (ScriptHookV.dll and dinput8.dll). Install it first: dev-c.com/gtav/scripthookv".into()); }
    if need_scripthook && d.asi_loaders.is_empty() && !d.asi_loaders_32.is_empty() { return Err(format!("{} in the GTA V folder is an ASI loader for 32-bit games (like GTA IV), so GTA V skips it and loads no mods. Replace it with ScriptHookV's own dinput8.dll, from its download's bin folder.", d.asi_loaders_32.join(", "))); }
    if need_scripthook && d.asi_loaders.is_empty() { return Err("There's no ASI loader in the GTA V folder, so GTA would load no mods. ScriptHookV's is dinput8.dll, in its download's bin folder next to ScriptHookV.dll.".into()); }
    if gta_running() { return Err("GTA V is running. Close it first.".into()); }
    if !d.writable { return Err("ADMIN".into()); }
    Ok(())
}

fn parkour_install(c: &Shared) -> Result<String, String> {
    let (gta, mirrors, remote, bundled, data, version) = {
        let mut a = c.app.lock().unwrap();
        a.refresh();
        gta_problems(&a, true)?;
        if !a.disk.mirrors_ok && !a.disk.me_copied { return Err("Pick your Mirror's Edge folder: the one that holds Binaries and TdGame.".into()); }
        (a.gta(), a.saved.mirrors.clone(), a.parkour_remote.clone(), a.dir.join("parkour"), a.data.clone(), a.cfg.version.clone())
    };
    let (src, tag) = match remote {
        Some(r) => {
            let (url, _) = r.full.clone().unwrap();
            let dir = data.join("parkour").join(&r.tag);
            if !dir.join("FaithRunnerV.asi").is_file() {
                let zip = data.join("downloads").join(format!("parkour-{}.zip", r.tag));
                download(c, Which::Parkour, &url, &zip, "Downloading")?;
                set_task(c, Which::Parkour, "Unpacking", None, "");
                let tmp = dir.with_extension("unpack");
                let _ = fs::remove_dir_all(&tmp);
                unzip(&zip, &tmp, false)?;
                let asi = walk_find(&tmp, "FaithRunnerV.asi").ok_or("The release has no FaithRunnerV.asi in it.")?;
                let _ = fs::remove_dir_all(&dir);
                fs::rename(asi.parent().unwrap(), &dir).map_err(|e| e.to_string())?;
                let _ = fs::remove_dir_all(&tmp);
                let _ = fs::remove_file(&zip);
            }
            (dir, r.tag)
        }
        None => (bundled, version),
    };
    set_task(c, Which::Parkour, "Installing", None, "");
    if !src.join("FaithRunnerV.asi").is_file() { return Err("FaithRunnerV.asi is missing from the launcher's parkour folder (Faith Runner's files).".into()); }
    fs::copy(src.join("FaithRunnerV.asi"), gta.join("FaithRunnerV.asi")).map_err(|e| format!("Copying FaithRunnerV.asi failed: {e}"))?;
    // Faith's files from the player's own Mirror's Edge, copied in once (Mirror's Edge can then go).
    if !mirrors.is_empty() && mirrors_ok(Path::new(&mirrors)) { copy_me_files(c, Path::new(&mirrors), &gta)?; }
    // An ini that's already there keeps the player's settings: only the Mirror's Edge line changes.
    let ini = gta.join("FaithRunnerV.ini");
    let from = if ini.is_file() { ini.clone() } else { src.join("FaithRunnerV.ini") };
    let text = fs::read(&from).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let set = format!("MirrorsEdge = {ME_COPY}");
    let mut found = false;
    let mut lines: Vec<String> = text.lines().map(|l| {
        if l.trim_start().starts_with("MirrorsEdge") && l.contains('=') { found = true; set.clone() } else { l.to_string() }
    }).collect();
    if !found { lines.push(set); }
    fs::write(&ini, lines.join("\r\n") + "\r\n").map_err(|e| format!("Writing FaithRunnerV.ini failed: {e}"))?;
    let sha = sha256(&gta.join("FaithRunnerV.asi"));
    c.app.lock().unwrap().remember(sha, &tag);
    Ok(format!("Faith Runner {tag} is installed. In story mode, tap F8 (or click both sticks) to switch it on, and hold it for half a second to open its settings menu."))
}

fn walk_find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() { if let Some(f) = walk_find(&p, name) { return Some(f); } }
        else if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)) { return Some(p); }
    }
    None
}

fn parkour_remove(c: &Shared) -> Result<String, String> {
    let gta = {
        let a = c.app.lock().unwrap();
        if gta_running() { return Err("GTA V is running. Close it first.".into()); }
        a.gta()
    };
    for f in ["FaithRunnerV.asi", "FaithRunnerV.ini", "FaithRunnerV.log", "FaithRunnerV"] {
        let p = gta.join(f);
        if p.exists() { trash::delete(&p).map_err(|e| format!("Couldn't move {f} to the Recycle Bin: {e}"))?; }
    }
    Ok("Faith Runner removed (its files are in the Recycle Bin).".into())
}

/// Returning players: becaring's update zip, two files copied over the old ones (his own instructions).
fn sk8v_patch(c: &Shared) -> Result<String, String> {
    let (gta, r, data) = {
        let mut a = c.app.lock().unwrap();
        a.refresh();
        gta_problems(&a, true)?;
        (a.gta(), a.sk8v_remote.clone().ok_or("Check for updates first.")?, a.data.clone())
    };
    // The newest update zip may sit on an older release than the newest one (setup-only releases).
    let tag = c.app.lock().unwrap().sk8v_files_tag.clone();
    let (url, url_tag) = match (&r.update, tag == r.tag) {
        (Some(u), _) => (u.0.clone(), r.tag.clone()),
        _ => {
            let sk8v_repo = c.app.lock().unwrap().cfg.sk8v_repo.clone();
            let v = get_json(&format!("https://api.github.com/repos/{sk8v_repo}/releases/tags/{tag}"))?;
            let rel = release_from(&v);
            (rel.update.ok_or("That release has no update zip.")?.0, tag.clone())
        }
    };
    let zip = data.join("downloads").join(format!("SK8V-{url_tag}-update.zip"));
    download(c, Which::Sk8v, &url, &zip, "Downloading the update")?;
    set_task(c, Which::Sk8v, "Installing", None, "");
    let tmp = data.join("downloads").join(format!("SK8V-{url_tag}-update"));
    let _ = fs::remove_dir_all(&tmp);
    unzip(&zip, &tmp, false)?;
    for f in ["SkateVLegacy.asi", "SkateVRuntime.dll"] {
        let s = walk_find(&tmp, f).ok_or(format!("The update has no {f}."))?;
        fs::copy(&s, gta.join(f)).map_err(|e| format!("Copying {f} failed: {e}"))?;
    }
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_file(&zip);
    let sha = sha256(&gta.join("SkateVLegacy.asi"));
    c.app.lock().unwrap().remember(sha, &url_tag);
    Ok(format!("SK8V updated to {url_tag}. Your settings and prepared data stay as they are."))
}

/// First install (or a setup rerun): becaring's full release, then his own setup steps.
fn sk8v_setup(c: &Shared) -> Result<String, String> {
    let (gta, r, dir, disc, stance) = {
        let mut a = c.app.lock().unwrap();
        a.refresh();
        gta_problems(&a, true)?;
        // SK8V's setup reads GTA's own game files: catch an incomplete GTA folder before the 207 MB download.
        if !a.gta().join("update").join("update.rpf").is_file() {
            return Err("This GTA V folder has no update\\update.rpf, which SK8V's setup reads. The GTA install is incomplete: verify it in Steam (or reinstall) first.".into());
        }
        let r = a.sk8v_remote.clone().ok_or("Couldn't reach SK8V's GitHub. Check your connection and press Check for updates.")?;
        let disc = a.saved.disc.clone();
        if disc.is_empty() || !Path::new(&disc).exists() { return Err("Pick your Skate 3 disc image (or its extracted folder) first.".into()); }
        let stance = if a.saved.stance.is_empty() { "Regular".to_string() } else { a.saved.stance.clone() };
        (a.gta(), r.clone(), a.sk8v_release_dir(&r.tag).unwrap(), disc, stance)
    };
    if !dir.join("tools").join("prepare.py").is_file() {
        let (url, _) = r.full.clone().ok_or("That SK8V release has no download.")?;
        let zip = dir.with_extension("zip");
        download(c, Which::Sk8v, &url, &zip, &format!("Downloading SK8V {}", r.tag))?;
        set_task(c, Which::Sk8v, "Unpacking", None, "");
        let tmp = dir.with_extension("unpack");
        let _ = fs::remove_dir_all(&tmp);
        unzip(&zip, &tmp, true)?;
        let _ = fs::remove_dir_all(&dir);
        fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(&zip);
    }
    let python = dir.join("python").join("python.exe");
    if !python.is_file() {
        let ps = dir.join("tools").join("get-python.ps1");
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&ps);
        run_logged(c, cmd, &dir, "Getting Python for SK8V's setup (31 MB)")?;
    }
    let mut cmd = Command::new(&python);
    cmd.args(["-B", "-u"]).arg(dir.join("tools").join("prepare.py"))
        .arg("--skate").arg(&disc).arg("--gta").arg(&gta).args(["--stance", &stance, "--install", "--clean"])
        .env("PYTHONIOENCODING", "utf-8").env("PYTHONNOUSERSITE", "1").env("PYTHONHOME", dir.join("python")).env_remove("PYTHONPATH");
    run_logged(c, cmd, &dir, "Preparing SK8V")?;
    let sha = sha256(&gta.join("SkateVLegacy.asi"));
    let files_tag = c.app.lock().unwrap().sk8v_files_tag.clone();
    // A setup-only release installs the ASI of the last release that changed it.
    let v = if !files_tag.is_empty() && cmp_ver(&files_tag, &r.tag) < 0 && c.app.lock().unwrap().known(sha.as_deref().unwrap_or("")).is_some() {
        c.app.lock().unwrap().known(sha.as_deref().unwrap_or("")).unwrap()
    } else { r.tag.clone() };
    c.app.lock().unwrap().remember(sha, &v);
    Ok("SK8V is installed. In story mode, pick the skateboard in the weapon wheel and press Y.".into())
}

/// Runs one of SK8V's setup tools, turning its "== stage: text" and "@progress N" lines into the card's progress.
fn run_logged(c: &Shared, mut cmd: Command, cwd: &Path, title: &str) -> Result<(), String> {
    set_task(c, Which::Sk8v, title, None, "");
    let mut child = cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(NO_WINDOW).spawn().map_err(|e| format!("Couldn't start {title}: {e}"))?;
    c.app.lock().unwrap().child = Some(child.id());
    let last = Arc::new(Mutex::new(String::new()));
    let pct = Arc::new(Mutex::new(None::<f64>));
    let phase = Arc::new(Mutex::new(title.to_string()));
    let mut readers = vec![];
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
        let (c, last, pct, phase) = (c.clone(), last.clone(), pct.clone(), phase.clone());
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let t = line.trim();
                if let Some(n) = t.strip_prefix("@progress ") { *pct.lock().unwrap() = n.trim().parse().ok(); }
                else if let Some(rest) = t.strip_prefix("== ") {
                    if let Some((_, what)) = rest.split_once(": ") { *phase.lock().unwrap() = what.to_string(); }
                    log(&c, Which::Sk8v, t);
                } else if !t.is_empty() { *last.lock().unwrap() = t.to_string(); log(&c, Which::Sk8v, t); }
                let detail = last.lock().unwrap().chars().take(120).collect::<String>();
                set_task(&c, Which::Sk8v, &phase.lock().unwrap(), *pct.lock().unwrap(), &detail);
            }
        }));
    }
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? { break s; }
        std::thread::sleep(Duration::from_millis(500));
        let el = start.elapsed().as_secs();
        let mut a = c.app.lock().unwrap();
        if let Some(t) = a.sk8v.task.as_mut() {
            let base = t.detail.split("  ·  ⏱").next().unwrap_or("").to_string();
            t.detail = format!("{base}  ·  ⏱ {}:{:02}", el / 60, el % 60);
        }
        drop(a);
        push(c);
    };
    for r in readers { let _ = r.join(); }
    if status.success() { return Ok(()); }
    if c.app.lock().unwrap().cancelled { return Err("Stopped. Press the button again to pick up where it left off.".into()); }
    let l = last.lock().unwrap().clone();
    Err(format!("{title} stopped: {l}\nNothing is lost: press the button again to pick up where it left off."))
}

fn cancel(c: &Shared) {
    let mut a = c.app.lock().unwrap();
    a.cancelled = true;
    if let Some(pid) = a.child {
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(NO_WINDOW).status();
    }
}

// ---------------------------------------------------------------- messages from the page

fn on_message(c: &Shared, msg: &str) {
    let Ok(m) = serde_json::from_str::<Value>(msg) else { return };
    let cmd = m["cmd"].as_str().unwrap_or("");
    let arg = m["value"].as_str().unwrap_or("").to_string();
    match cmd {
        "ready" => { push(c); let c2 = c.clone(); std::thread::spawn(move || check(c2)); }
        "check" => { let c2 = c.clone(); std::thread::spawn(move || check(c2)); }
        "pick" => {
            let c2 = c.clone();
            std::thread::spawn(move || {
                let start = { let a = c2.app.lock().unwrap(); match arg.as_str() { "gta" => a.saved.gta.clone(), "mirrors" => a.saved.mirrors.clone(), _ => a.saved.disc.clone() } };
                let mut d = rfd::FileDialog::new();
                if !start.is_empty() { d = d.set_directory(Path::new(&start).parent().filter(|_| arg.starts_with("disc")).unwrap_or(Path::new(&start))); }
                let picked = match arg.as_str() {
                    "gta" => d.set_title("Your GTA V Legacy folder (the one with GTA5.exe)").pick_folder(),
                    "mirrors" => d.set_title("Your Mirror's Edge folder (the one with Binaries and TdGame)").pick_folder(),
                    "discFolder" => d.set_title("Your extracted Skate 3 disc folder").pick_folder(),
                    _ => d.set_title("Your Skate 3 (Xbox 360) disc image").add_filter("Xbox 360 disc image", &["iso"]).add_filter("All files", &["*"]).pick_file(),
                };
                if let Some(p) = picked {
                    let p = p.to_string_lossy().to_string();
                    let mut a = c2.app.lock().unwrap();
                    match arg.as_str() { "gta" => a.saved.gta = p, "mirrors" => a.saved.mirrors = p, _ => a.saved.disc = p }
                    a.save();
                    a.refresh();
                    drop(a);
                    push(&c2);
                }
            });
        }
        "stance" => { let mut a = c.app.lock().unwrap(); a.saved.stance = arg; a.save(); drop(a); push(c); }
        "clear" => { let mut a = c.app.lock().unwrap(); let k = if arg == "sk8v" { &mut a.sk8v } else { &mut a.parkour }; k.error.clear(); k.message.clear(); drop(a); push(c); }
        "parkourInstall" => run_task(c.clone(), Which::Parkour, parkour_install),
        "parkourRemove" => run_task(c.clone(), Which::Parkour, parkour_remove),
        "sk8vSetup" => run_task(c.clone(), Which::Sk8v, sk8v_setup),
        "sk8vPatch" => run_task(c.clone(), Which::Sk8v, sk8v_patch),
        "cancel" => cancel(c),
        "url" => { if arg.starts_with("https://") { shell_open(&arg, "open", ""); } }
        "open" => {
            let a = c.app.lock().unwrap();
            let target = match arg.as_str() {
                "gta" => a.gta(),
                "parkourLog" => a.gta().join("FaithRunnerV.log"),
                "parkourIni" => a.gta().join("FaithRunnerV.ini"),
                "sk8vLogs" => PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("SkateV"),
                "notice" => a.dir.join("NOTICE.txt"),
                "readme" => a.dir.join("README.txt"),
                _ => return,
            };
            drop(a);
            if target.exists() { shell_open(&target.to_string_lossy(), "open", ""); }
        }
        "play" => {
            // Story Mode with BattlEye off (it blocks ScriptHookV), started directly rather than through
            // Steam's Play button, which installs a pending update first.
            // A Steam copy refuses to start without the Steam client running ("Steam failed to initialize",
            // shown as "Unable to launch game"): start Steam quietly first and give it time to sign in.
            let g = c.app.lock().unwrap().gta();
            let exe = g.join("PlayGTAV.exe");
            if !exe.is_file() { return; }
            let c2 = c.clone();
            std::thread::spawn(move || {
                let steam_copy = g.join("steam_api64.dll").is_file();
                if steam_copy && !process_running("steam.exe") {
                    use winreg::enums::HKEY_CURRENT_USER;
                    if let Some(steam) = reg(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamExe") {
                        set_task(&c2, Which::Parkour, "Starting Steam first", None, "The Steam version of GTA V needs Steam running");
                        let _ = Command::new(steam.replace('/', "\\")).arg("-silent").spawn();
                        let start = Instant::now();
                        while !process_running("steamwebhelper.exe") && start.elapsed() < Duration::from_secs(60) { std::thread::sleep(Duration::from_secs(1)); }
                        std::thread::sleep(Duration::from_secs(8));
                        c2.app.lock().unwrap().parkour.task = None;
                        push(&c2);
                    }
                }
                let _ = Command::new(&exe).arg("-nobattleye").current_dir(&g).spawn();
            });
        }
        "elevate" => {
            if let Ok(me) = std::env::current_exe() {
                if shell_open(&me.to_string_lossy(), "runas", "") { let _ = c.proxy.lock().unwrap().send_event(Ev::Quit); }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- the window

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn main() {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let cfg: Config = fs::read_to_string(dir.join("launcher.json")).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
    let slug: String = cfg.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let data = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join(slug.trim_matches('-'));
    let mut saved: Saved = fs::read_to_string(data.join("launcher-state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // Command line: Launcher.exe --install-parkour [--gta "<GTA V folder>"] [--mirrors "<Mirror's Edge folder>"]
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.iter().position(|a| a.eq_ignore_ascii_case(k)).and_then(|i| args.get(i + 1)).cloned();
    let headless = args.iter().any(|a| a.eq_ignore_ascii_case("--install-parkour"));
    if let Some(g) = arg("--gta") { saved.gta = g.trim_end_matches(['\\', '"']).to_string(); }
    if let Some(m) = arg("--mirrors") { saved.mirrors = m.trim_end_matches(['\\', '"']).to_string(); }
    if saved.gta.is_empty() || !Path::new(&saved.gta).join("GTA5.exe").is_file() { if arg("--gta").is_none() { saved.gta = find_gta(); } }
    if saved.mirrors.is_empty() || !mirrors_ok(Path::new(&saved.mirrors)) { if arg("--mirrors").is_none() { saved.mirrors = find_mirrors(); } }
    let title = cfg.name.clone();
    let mut app = App { cfg, dir: dir.clone(), data, saved, disk: Disk::default(), parkour: Card::default(), sk8v: Card::default(),
        parkour_remote: None, sk8v_remote: None, sk8v_files_tag: String::new(), checking: false, check_error: String::new(),
        child: None, cancelled: false };
    app.refresh();
    app.disk.running = gta_running();

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let ctx: Shared = Arc::new(Ctx { app: Mutex::new(app), proxy: Mutex::new(event_loop.create_proxy()) });

    // GTA starting or closing changes what the buttons may do.
    {
        let c = ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(3));
            let r = gta_running();
            let mut a = c.app.lock().unwrap();
            if a.disk.running != r { a.disk.running = r; a.refresh(); drop(a); push(&c); }
        });
    }

    // --install-parkour: install Faith Runner without opening the window, answer in the console.
    if headless {
        unsafe { windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS) };
        let (gta, me) = { let a = ctx.app.lock().unwrap(); (a.saved.gta.clone(), a.saved.mirrors.clone()) };
        println!("\nGTA V folder: {}\nMirror's Edge folder: {}", if gta.is_empty() { "not found (add --gta \"<folder>\")" } else { &gta },
            if me.is_empty() { "not found (add --mirrors \"<folder>\")" } else { &me });
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parkour_install(&ctx))).unwrap_or_else(|_| Err("Something went wrong (internal error).".into()));
        let code = match r {
            Ok(m) => { println!("{m}"); 0 }
            Err(e) if e == "ADMIN" => { println!("Windows only lets administrators change this GTA V folder: run this from an administrator command prompt."); 1 }
            Err(e) => { println!("{e}"); 1 }
        };
        std::process::exit(code);
    }

    let window = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(tao::dpi::LogicalSize::new(1120.0, 760.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(880.0, 640.0))
        .with_theme(Some(Theme::Dark))
        .build(&event_loop)
        .expect("window");

    // Card pictures: images\ beside the exe (lets anyone swap in their own); without one, the parkour
    // card streams its video straight from the mod's GitHub (nothing is saved on the PC).
    let images = dir.join("images");
    let stream = ctx.app.lock().unwrap().cfg.parkour_video_url().unwrap_or_default();
    let support = ctx.app.lock().unwrap().cfg.support.trim().to_string();
    let support = if support.starts_with("https://") { support } else { String::new() };
    let ipc_ctx = ctx.clone();
    let webview = WebViewBuilder::new()
        .with_background_color((12, 12, 16, 255))
        .with_custom_protocol("app".into(), move |_id, req: Request<Vec<u8>>| {
            let path = req.uri().path().to_string();
            let (body, kind): (Cow<'static, [u8]>, &str) = if path == "/" || path == "/index.html" {
                (Cow::Borrowed(include_str!("../ui/index.html").as_bytes()), "text/html")
            } else if let Some(name) = path.strip_prefix("/img/") {
                let name = name.replace(['/', '\\'], "");
                match fs::read(images.join(&name)) { Ok(b) => (Cow::Owned(b), mime(&name)), Err(_) => (Cow::Borrowed(&[][..]), "text/plain") }
            } else { (Cow::Borrowed(&[][..]), "text/plain") };
            let status = if body.is_empty() { 404 } else { 200 };
            Response::builder().status(status).header(CONTENT_TYPE, kind).body(body).unwrap()
        })
        .with_initialization_script(&format!("window.STREAM = {}; window.SUPPORT = {};", json!({ "parkour": stream }), json!(support)))
        .with_ipc_handler(move |req: Request<String>| on_message(&ipc_ctx, req.body()))
        .with_devtools(cfg!(debug_assertions))
        .with_url("app://localhost/")
        .build(&window)
        .expect("webview");

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Push) => {
                let snap = ctx.app.lock().unwrap().snapshot();
                let _ = webview.evaluate_script(&format!("window.onState && window.onState({snap})"));
            }
            Event::UserEvent(Ev::Quit) => *flow = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                let busy = { let a = ctx.app.lock().unwrap(); a.child.is_some() };
                if busy { cancel(&ctx); }
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
