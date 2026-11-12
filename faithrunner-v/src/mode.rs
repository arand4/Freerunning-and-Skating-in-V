//! The script loop: Faith mode on/off, input, stepping Faith's controller on
//! GTA's collision, and placing the GTA player where she is.
use std::f32::consts::{PI, TAU};
use std::ffi::CString;
use std::sync::Arc;
use std::time::Instant;

use faith_anim::sound::{Director, SoundCmd};
use faith_move::greybox::Surface;
use faith_move::{CameraFx, CameraFxSettings, Controller, Input, LookLimiter, MeshWorld, State, Tuning};
use glam::{Vec2, Vec3};

use crate::collision::{self, dir_to_faith, dir_to_gta, to_faith, to_gta};
use crate::config::{self, Config};
use crate::natives::*;
use crate::shv::{ret_bool, ret_f32, ret_i32, ret_vec3, wait};
use crate::audio::{self, Msg};
use crate::body::{self, Body, Loaded};
use crate::dynamic::{self, Dimensions};
use crate::render;
use crate::third::{self, Faith3p};
use crate::{level, live, log, native, win};

fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

fn say(text: &str, ms: i32) {
    let s = CString::new(text).unwrap_or_default();
    native!(BEGIN_TEXT_COMMAND_PRINT, c"STRING".as_ptr());
    native!(ADD_TEXT_COMPONENT_SUBSTRING_PLAYER_NAME, s.as_ptr());
    native!(END_TEXT_COMMAND_PRINT, ms, true);
}

/// Buttons as levels; edges come from comparing with last frame.
#[derive(Clone, Copy, Default, PartialEq)]
struct Buttons {
    jump: bool,
    crouch: bool,
    turn: bool,
    melee: bool,
    toggle: bool,
    /// Pressed edges only: the toggle held for half a second (the menu), or the menu key.
    menu: bool,
    /// The menu key (MenuKey) as a level.
    menu_key: bool,
}

struct Pads {
    prev: Buttons,
    /// When the toggle went down, and whether its hold already opened the menu.
    toggle_since: Option<Instant>,
    hold_fired: bool,
}

impl Pads {
    /// This frame's move axis, buttons and pressed edges.
    fn read(&mut self, cfg: &Config) -> (Vec2, Buttons, Buttons) {
        let mut b = Buttons::default();
        let mut axis = Vec2::ZERO;
        if let Some(p) = win::pad() {
            let btn = p.buttons;
            let stick = Vec2::new(p.lx as f32, p.ly as f32) / 32767.0;
            if stick.length() > 0.24 {
                axis = stick;
            }
            b.jump |= btn & 0x1000 != 0 || btn & 0x0100 != 0; // A, LB
            b.crouch |= btn & 0x2000 != 0 || p.left_trigger > 100; // B, LT
            b.turn |= btn & 0x8000 != 0; // Y
            b.melee |= btn & 0x4000 != 0; // X
            b.toggle |= cfg.pad_toggle && btn & 0x00C0 == 0x00C0; // both sticks clicked
        }
        if win::game_focused() {
            let k = win::key_down;
            let kb = Vec2::new((k(0x44) as i32 - k(0x41) as i32) as f32, (k(0x57) as i32 - k(0x53) as i32) as f32);
            if kb != Vec2::ZERO {
                axis = kb;
            }
            b.jump |= k(0x20); // Space
            b.crouch |= k(0x10) || k(0x43) || k(0x11); // Shift, C, Ctrl
            b.turn |= k(0x51); // Q
            b.melee |= k(0x46); // F
            b.toggle |= k(cfg.toggle_key);
            b.menu_key |= cfg.menu_key != 0 && k(cfg.menu_key);
        }
        let prev = self.prev;
        self.prev = b;
        // The toggle: a tap (released within half a second) switches Faith mode; holding it
        // opens the menu.
        let (mut tap, mut menu) = (false, false);
        if b.toggle {
            let since = *self.toggle_since.get_or_insert_with(Instant::now);
            if !self.hold_fired && since.elapsed().as_secs_f32() >= 0.5 {
                self.hold_fired = true;
                menu = true;
            }
        } else if let Some(_since) = self.toggle_since.take() {
            tap = !self.hold_fired;
            self.hold_fired = false;
        }
        let pressed = Buttons {
            jump: b.jump && !prev.jump,
            crouch: b.crouch && !prev.crouch,
            turn: b.turn && !prev.turn,
            melee: b.melee && !prev.melee,
            toggle: tap,
            menu: menu || (b.menu_key && !prev.menu_key),
            menu_key: false,
        };
        (axis, b, pressed)
    }
}

/// A move's GTA clip: candidates tried in order until one loads.
struct Clip {
    options: Vec<(CString, CString)>,
    chosen: Option<usize>,
    tried: usize,
    since: Option<Instant>,
}

impl Clip {
    fn new(spec: &str) -> Clip {
        let options = spec
            .split('|')
            .filter_map(|o| {
                let (d, c) = o.split_once(',')?;
                Some((CString::new(d.trim()).ok()?, CString::new(c.trim()).ok()?))
            })
            .collect();
        Clip { options, chosen: None, tried: 0, since: None }
    }

    /// The loaded (dictionary, clip), requesting it if needed.
    fn ready(&mut self) -> Option<(&CString, &CString)> {
        if let Some(i) = self.chosen {
            let (d, c) = &self.options[i];
            return Some((d, c));
        }
        let (d, _) = self.options.get(self.tried)?;
        native!(REQUEST_ANIM_DICT, d.as_ptr());
        if ret_bool(native!(HAS_ANIM_DICT_LOADED, d.as_ptr())) {
            self.chosen = Some(self.tried);
            log!("anim: using {:?} {:?}", self.options[self.tried].0, self.options[self.tried].1);
            let (d, c) = &self.options[self.tried];
            return Some((d, c));
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        if since.elapsed().as_secs_f32() > 3.0 {
            log!("anim: {:?} did not load; trying the next one", d);
            self.tried += 1;
            self.since = None;
        }
        None
    }

    /// The chosen clip didn't play (the dictionary loads but has no such clip, or the
    /// clip can't be played): move on to the next option.
    fn reject(&mut self) {
        if let Some(i) = self.chosen.take() {
            log!("anim: {:?} {:?} does not play; trying the next one", self.options[i].0, self.options[i].1);
            self.tried = i + 1;
            self.since = None;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pose {
    Idle,
    Run,
    Air,
    Wall,
    Hang,
    Slide,
}

fn pose_of(c: &Controller) -> Pose {
    match c.state {
        State::Ground if c.horizontal_speed() > 0.6 => Pose::Run,
        State::Ground => Pose::Idle,
        State::Slide { .. } | State::Roll { .. } => Pose::Slide,
        State::WallRun { .. } | State::WallClimb { .. } | State::WallClimbTurned { .. } => Pose::Wall,
        State::LedgeHang { .. } | State::Traverse(_) | State::Vault(_) => Pose::Hang,
        State::Air => Pose::Air,
        _ => Pose::Idle,
    }
}

struct Faith {
    ped: i32,
    ctrl: Controller,
    origin: Vec3,
    pose: Option<Pose>,
    /// When the current clip was started (to check it really plays).
    started: Option<Instant>,
    last_state: &'static str,
    /// First person: the script camera, Faith's camera effects and look limits.
    cam: i32,
    fx: CameraFx,
    look: LookLimiter,
    /// Cars and props: model sizes, and how many were near last frame (for the log).
    dims: Dimensions,
    near_bodies: usize,
    /// Faith's arms and legs (Body = faith, first person, Mirror's Edge found).
    body: Option<Body>,
    /// Third person: Faith's third-person animations on the GTA character's skeleton.
    third: Option<third::Driver>,
    /// Body = gta: in first person the GTA character's own body wears Faith's first-person
    /// animations, its head shrunk away; `own` once set up.
    own_body: bool,
    own: Option<OwnBody>,
    /// HeadHide setting, and whether GTA's own first-person camera is in use (HeadHide = gtacam).
    head_hide: String,
    /// Body setting: faith (her arms drawn), gta (own body), none.
    body_mode: String,
    gta_fp: bool,
    /// GTA's camera view mode before Faith mode (given back on leaving).
    view_mode_before: i32,
}

struct OwnBody {
    gta: crate::retarget::GtaSkeleton,
    retarget: crate::retarget::Retarget,
    head: usize,
}

/// The player character's skeleton: (entity address, crSkeleton, bind pose).
fn player_skeleton(ped: i32) -> Option<(usize, usize, crate::retarget::GtaSkeleton)> {
    let entity = unsafe { (crate::shv::shv().handle_address)(ped) } as usize;
    let s = (entity != 0).then(|| crate::posehook::skeleton_of(entity)).flatten()?;
    Some((entity, s, crate::posehook::read_skeleton(s)?))
}

/// Body = gta: the character's own body in first person (its head hidden). False: not possible
/// (Faith's own arms are drawn instead).
fn own_body_on(f: &mut Faith) -> bool {
    let Some(b) = f.body.as_ref() else { return false };
    let Some((entity, s, gta)) = player_skeleton(f.ped) else {
        log!("own body: the character's skeleton wasn't found");
        return false;
    };
    if f.own.as_ref().is_none_or(|o| o.gta.names.len() != gta.names.len()) {
        match crate::retarget::Retarget::new(&gta, &b.loaded.arms.mesh, &b.rest_globals()) {
            Ok(r) => {
                let Some(head) = gta.index("SKEL_Head") else {
                    log!("own body: the character has no head bone");
                    return false;
                };
                log!("own body: {} of the character's {} bones wear Faith's first-person animations", r.mapped(), gta.names.len());
                f.own = Some(OwnBody { gta, retarget: r, head });
            }
            Err(e) => {
                log!("own body: this character can't wear Faith's animations ({e})");
                return false;
            }
        }
    }
    crate::posehook::install(entity, s)
}

fn faith3p() -> Option<Arc<Faith3p>> {
    third::LOADED.lock().unwrap().clone()
}

/// First person: the script camera at Faith's eye, the GTA character hidden.
fn first_on(f: &mut Faith) {
    if f.cam != 0 || f.gta_fp {
        return;
    }
    crate::posehook::uninstall();
    if f.own_body && f.head_hide == "gtacam" {
        // GTA's own first-person camera (its own head hiding); the character wears Faith's pose.
        if own_body_on(f) {
            f.gta_fp = true;
            native!(SET_FOLLOW_PED_CAM_VIEW_MODE, 4);
            log!("faith: first person, GTA's own camera (HeadHide = gtacam)");
            return;
        }
        crate::posehook::uninstall();
    }
    render::publish(None);
    let cam = ret_i32(native!(CREATE_CAM, c"DEFAULT_SCRIPTED_CAMERA".as_ptr(), true));
    native!(SET_CAM_NEAR_CLIP, cam, 0.05f32);
    native!(SET_CAM_ACTIVE, cam, true);
    if !(f.own_body && own_body_on(f)) {
        f.own = None;
        // Hide the GTA character: the camera sits inside its head. Shown again on leaving.
        native!(SET_ENTITY_VISIBLE, f.ped, false, false);
    }
    native!(RENDER_SCRIPT_CAMS, true, false, 0, true, false, 0);
    f.cam = cam;
    log!("faith: first person");
}

/// Third person: GTA's camera, the character shown, wearing Faith's third-person animations.
fn third_on(f: &mut Faith) {
    if f.gta_fp {
        f.gta_fp = false;
        native!(SET_FOLLOW_PED_CAM_VIEW_MODE, if f.view_mode_before == 4 { 1 } else { f.view_mode_before });
    }
    if f.cam != 0 {
        native!(RENDER_SCRIPT_CAMS, false, false, 0, true, false, 0);
        if ret_bool(native!(DOES_CAM_EXIST, f.cam)) {
            native!(SET_CAM_ACTIVE, f.cam, false);
            native!(DESTROY_CAM, f.cam, false);
        }
        f.cam = 0;
        render::publish(None);
        if ret_bool(native!(DOES_ENTITY_EXIST, f.ped)) {
            native!(SET_ENTITY_VISIBLE, f.ped, true, false);
        }
        native!(SET_GAMEPLAY_CAM_RELATIVE_HEADING, 0.0f32);
    }
    crate::posehook::uninstall();
    log!("faith: third person");
    let entity = unsafe { (crate::shv::shv().handle_address)(f.ped) } as usize;
    let Some(s) = (entity != 0).then(|| crate::posehook::skeleton_of(entity)).flatten() else {
        log!("third person: the character's skeleton wasn't found");
        return;
    };
    if f.third.as_ref().is_none_or(|d| d.bones() != crate::posehook::read_skeleton(s).map_or(0, |g| g.names.len())) {
        f.third = None;
        match (faith3p(), crate::posehook::read_skeleton(s)) {
            (Some(fa), Some(gta)) => {
                let n = gta.names.len();
                match third::Driver::new(fa, gta) {
                    Ok(d) => {
                        log!("third person: {} of the character's {n} bones wear Faith's animations", d.mapped());
                        f.third = Some(d);
                    }
                    Err(e) => log!("third person: this character can't wear Faith's animations ({e})"),
                }
            }
            (None, _) => log!("third person: Faith's third-person animations aren't loaded (Mirror's Edge not found?)"),
            (_, None) => log!("third person: the character's skeleton couldn't be read"),
        }
    }
    if f.third.is_some() && !crate::posehook::install(entity, s) {
        f.third = None;
    }
}

/// Faith's sounds: the output thread, and the director once the cues are read.
struct Sound {
    tx: std::sync::mpsc::Sender<Msg>,
    pending: Option<std::sync::mpsc::Receiver<std::collections::HashMap<String, faith_anim::sound::CueInfo>>>,
    director: Option<Director>,
}

/// The body, once it has loaded in the background (None: not wanted, not found, or failed).
fn loaded_body() -> Option<Arc<Loaded>> {
    body::LOADED.lock().unwrap().as_ref().and_then(|r| r.as_ref().ok().cloned())
}

enum Mode {
    Off,
    Waiting { since: Instant, origin: Vec3 },
    On(Box<Faith>),
}

// GTA controls taken while Faith has the player: sprint, jump, enter, attack, aim, duck,
// weapon wheel, cover, reload, dive, throw, melee, phone, move. Look (1, 2) and pause stay GTA's.
const TAKEN: [i32; 30] = [
    21, 22, 23, 24, 25, 36, 37, 44, 45, 47, 55, 58, 140, 141, 142, 143, 257, 263, 264, 27, 30, 31, 32, 33, 34, 35, 68, 69, 70, 75,
];

// First person also takes the look axes (Faith turns herself) and GTA's camera buttons.
const TAKEN_FIRST_PERSON: [i32; 6] = [0, 1, 2, 26, 5, 6];

fn hold(ped: i32, on: bool) {
    native!(FREEZE_ENTITY_POSITION, ped, on);
    native!(SET_ENTITY_COLLISION, ped, !on, false);
    if !on {
        native!(SET_ENTITY_COMPLETELY_DISABLE_COLLISION, ped, true, false);
    }
    native!(SET_ENTITY_HAS_GRAVITY, ped, !on);
    native!(SET_PED_CAN_RAGDOLL, ped, !on);
}

fn player_ok(ped: i32) -> bool {
    ped != 0
        && ret_bool(native!(DOES_ENTITY_EXIST, ped))
        && !ret_bool(native!(IS_ENTITY_DEAD, ped, false))
        && !ret_bool(native!(IS_PED_IN_ANY_VEHICLE, ped, false))
}

pub fn run() {
    let mut cfg = config::load();
    let mut menu = crate::menu::Menu::new();
    let shared = collision::start(cfg.collision_radius, cfg.rebuild_distance);
    crate::ladders::start_watch(shared.clone());
    let mut pads = Pads { prev: Buttons::default(), toggle_since: None, hold_fired: false };
    let mut mode = Mode::Off;
    let mut clips = [
        (Pose::Run, Clip::new(&cfg.anim_run)),
        (Pose::Air, Clip::new(&cfg.anim_air)),
        (Pose::Wall, Clip::new(&cfg.anim_wall)),
        (Pose::Hang, Clip::new(&cfg.anim_hang)),
        (Pose::Slide, Clip::new(&cfg.anim_slide)),
    ];
    let mut next_locate = Instant::now();
    let locating = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let image = win::gta_image();
    // Faith's first-person rig drives both her own arms (faith) and the character's body (gta).
    // Faith's first-person rig drives her own arms (faith) and the character's body (gta); always
    // loaded when Mirror's Edge is found, so the menu can switch between them.
    let want_body = true;
    match body::find_install(&cfg.mirrors_edge) {
        Some(dir) => {
            if want_body {
                body::load_in_background(dir.clone());
            }
            third::load_in_background(dir);
        }
        None => log!("body: Mirror's Edge not found (set MirrorsEdge in FaithRunnerV.ini); no body or third-person animations"),
    }
    let volumes = audio::Volumes {
        all: cfg.sound_volume,
        footsteps: cfg.footstep_volume,
        hands: cfg.hand_volume,
        voice: cfg.voice_volume,
        effects: cfg.effects_volume,
    };
    let mut sound = cfg.sounds.then(|| Sound { tx: audio::start(volumes), pending: None, director: None });
    let mut cues_asked = false;
    // The character model (menu "Character"): a model change being loaded, the model the game
    // gave you (to go back to), and Faith mode to switch back on after the change.
    let mut model_job: Option<(u32, Instant)> = None;
    let mut game_model: Option<u32> = None;
    let mut model_swap: Option<u32> = None;
    let mut resume_faith = false;
    let mut model_applied = cfg.ped_model == "game";
    let started = Instant::now();
    log!("Faith Runner V running. Toggle: key {:#x}{}", cfg.toggle_key, if cfg.pad_toggle { " or both sticks clicked" } else { "" });

    loop {
        wait(0);
        let ped = ret_i32(native!(PLAYER_PED_ID));

        // Find GTA's physics level in the background (once the world has spawned some things).
        let have_level = shared.lock().unwrap().level.is_some();
        if !have_level && !locating.load(std::sync::atomic::Ordering::Relaxed) && Instant::now() >= next_locate {
            let insts = level::named_instances();
            if insts.len() < 20 {
                next_locate = Instant::now() + std::time::Duration::from_millis(500);
            } else {
                next_locate = Instant::now() + std::time::Duration::from_secs(5);
                locating.store(true, std::sync::atomic::Ordering::Relaxed);
                let (shared, locating) = (shared.clone(), locating.clone());
                std::thread::spawn(move || {
                    let t0 = Instant::now();
                    let base = level::locate(insts);
                    if base != 0 {
                        log!("level: table at {base:#x}, {:.0} ms to find", t0.elapsed().as_secs_f64() * 1e3);
                        shared.lock().unwrap().level = Some(live::Level { base: base as usize, image });
                    } else {
                        log!("level: table not found; will retry");
                    }
                    locating.store(false, std::sync::atomic::Ordering::Relaxed);
                });
            }
        }

        // Once the body has loaded, read its sounds (on the audio thread).
        if let Some(snd) = sound.as_mut() {
            if !cues_asked && let Some(l) = loaded_body() {
                cues_asked = true;
                let (reply, rx) = std::sync::mpsc::channel();
                let wanted = faith_anim::sound::wanted_cues(Some(&l.arms));
                let _ = snd.tx.send(Msg::Load { cooked: l.arms.cooked_pc.clone(), wanted, reply });
                snd.pending = Some(rx);
            }
            if let Some(rx) = &snd.pending
                && let Ok(infos) = rx.try_recv()
            {
                snd.director = (!infos.is_empty()).then(|| Director::new(infos));
                snd.pending = None;
            }
        }

        let (mut axis, mut held, mut pressed) = pads.read(&cfg);
        // The settings menu: open on a hold, closed by a tap (or B / Backspace inside it).
        let mut menu_action = crate::menu::Action::None;
        if pressed.menu {
            menu.open = !menu.open;
        } else if menu.open && pressed.toggle {
            menu.open = false;
            pressed.toggle = false;
        }
        if menu.open {
            menu_action = menu.tick(&mut cfg);
            // The menu's keys and buttons aren't Faith's.
            axis = Vec2::ZERO;
            held = Buttons::default();
            pressed = Buttons::default();
            match menu_action {
                crate::menu::Action::Ladders => {
                    static SEARCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if SEARCHING.swap(true, std::sync::atomic::Ordering::AcqRel) {
                        menu.note("Already searching...");
                    } else {
                        let at = shared.lock().unwrap().player;
                        menu.note("Searching for ladders (a few seconds; results in the log)");
                        std::thread::spawn(move || {
                            let t0 = Instant::now();
                            let found = std::panic::catch_unwind(|| crate::ladders::debug_log_near(at)).map(|l| l.len());
                            match found {
                                Ok(n) => log!("ladders: search done in {:.1} s, {n} ladders found near the player", t0.elapsed().as_secs_f32()),
                                Err(_) => log!("ladders: the search stopped after an error"),
                            }
                            SEARCHING.store(false, std::sync::atomic::Ordering::Release);
                        });
                    }
                }
                crate::menu::Action::Save => match config::save(&cfg) {
                    Ok(()) => menu.note("Saved to FaithRunnerV.ini"),
                    Err(e) => menu.note(&format!("Couldn't save: {e}")),
                },
                crate::menu::Action::Volumes => {
                    if let Some(s) = &sound {
                        let _ = s.tx.send(Msg::Volumes(audio::Volumes {
                            all: cfg.sound_volume,
                            footsteps: cfg.footstep_volume,
                            hands: cfg.hand_volume,
                            voice: cfg.voice_volume,
                            effects: cfg.effects_volume,
                        }));
                    }
                }
                _ => {}
            }
        }
        // Character model: start loading on a menu change (or the saved setting once the player
        // is about); once loaded, leave Faith mode if it's on, change, and put Faith mode back.
        if (menu_action == crate::menu::Action::Model || (!model_applied && started.elapsed().as_secs() > 15)) && player_ok(ped) {
            model_applied = true;
            let current = ret_i32(native!(GET_ENTITY_MODEL, ped)) as u32;
            let want = match cfg.ped_model.as_str() {
                "faith" | "faith2" => Some(crate::ladders::joaat(&cfg.ped_model)),
                _ => game_model,
            };
            match want {
                Some(h) if h == current => model_job = None,
                Some(h) if !ret_bool(native!(IS_MODEL_IN_CDIMAGE, h)) => {
                    log!("model: {} isn't installed (add-on pack faithcatalyst)", cfg.ped_model);
                    menu.note("Faith's ped isn't installed (add-on pack faithcatalyst)");
                    cfg.ped_model = "game".into();
                }
                Some(h) => {
                    if !is_faith_model(current) {
                        game_model = Some(current);
                    }
                    model_job = Some((h, Instant::now()));
                }
                None => {}
            }
        }
        if let Some((h, since)) = model_job {
            native!(REQUEST_MODEL, h);
            if ret_bool(native!(HAS_MODEL_LOADED, h)) {
                model_job = None;
                model_swap = Some(h);
            } else if since.elapsed().as_secs() > 10 {
                model_job = None;
                log!("model: {h:#x} didn't load in 10 s");
                menu.note("Couldn't load the character model");
            }
        }
        if let Some(h) = model_swap {
            if matches!(mode, Mode::On(_)) {
                // Faith mode lets go of the old character first (this frame), then the change.
                pressed.toggle = true;
                resume_faith = true;
            } else if matches!(mode, Mode::Off) {
                // (One native call at a time: a native! inside another's arguments clobbers it.)
                let player = ret_i32(native!(PLAYER_ID));
                native!(SET_PLAYER_MODEL, player, h);
                let ped = ret_i32(native!(PLAYER_PED_ID));
                native!(SET_PED_DEFAULT_COMPONENT_VARIATION, ped);
                native!(SET_MODEL_AS_NO_LONGER_NEEDED, h);
                log!("model: the player is now {h:#010x} ({})", cfg.ped_model);
                model_swap = None;
                continue;
            }
        } else if resume_faith && matches!(mode, Mode::Off) {
            resume_faith = false;
            pressed.toggle = true;
        }
        FAITH_PED.store(ped != 0 && is_faith_model(ret_i32(native!(GET_ENTITY_MODEL, ped)) as u32), std::sync::atomic::Ordering::Relaxed);
        let paused = ret_bool(native!(IS_PAUSE_MENU_ACTIVE));
        if ped != 0 {
            shared.lock().unwrap().player = ret_vec3(native!(GET_ENTITY_COORDS, ped, true));
        }

        mode = match mode {
            Mode::Off => {
                let skating = pressed.toggle
                    && !unsafe { win::GetModuleHandleA(c"SkateVLegacy.asi".as_ptr()) }.is_null()
                    && crate::posehook::taken_by_other(unsafe { (crate::shv::shv().handle_address)(ped) } as usize);
                if skating {
                    say("Leave skate mode first", 2500);
                    log!("faith: not started: SK8V's skate mode is on");
                    Mode::Off
                } else if pressed.toggle && player_ok(ped) {
                    let origin = ret_vec3(native!(GET_ENTITY_COORDS, ped, true));
                    let mut s = shared.lock().unwrap();
                    s.active = true;
                    s.origin = origin;
                    s.world = None;
                    let msg = if s.level.is_some() { "Faith mode: reading the area..." } else { "Faith mode: finding GTA's collision..." };
                    drop(s);
                    say(msg, 2000);
                    log!("faith: requested at ({:.1}, {:.1}, {:.1})", origin.x, origin.y, origin.z);
                    Mode::Waiting { since: Instant::now(), origin }
                } else {
                    Mode::Off
                }
            }
            Mode::Waiting { since, origin } => {
                let ready = shared.lock().unwrap().world.as_ref().is_some_and(|(_, o)| *o == origin);
                if pressed.toggle || !player_ok(ped) {
                    shared.lock().unwrap().active = false;
                    say("Faith mode cancelled", 1500);
                    Mode::Off
                } else if ready {
                    Mode::On(Box::new(enter(ped, origin, &cfg, if want_body { loaded_body() } else { None })))
                } else if since.elapsed().as_secs_f32() > 20.0 {
                    shared.lock().unwrap().active = false;
                    say("Faith mode: couldn't read GTA's collision here (see FaithRunnerV.log)", 4000);
                    log!("faith: no collision after 20 s");
                    Mode::Off
                } else {
                    Mode::Waiting { since, origin }
                }
            }
            Mode::On(mut f) => {
                let mut return_off = false;
                let current = ret_i32(native!(PLAYER_PED_ID));
                let fell = to_gta(f.ctrl.feet, f.origin).z < f.origin.z - 300.0;
                // Moved by something else (a map switch, a trainer's teleport): the collision
                // there isn't loaded yet, so Faith mode steps aside.
                let expected = to_gta(f.ctrl.feet, f.origin) + Vec3::Z * cfg.ped_z_offset;
                let now_at = ret_vec3(native!(GET_ENTITY_COORDS, f.ped, true));
                let moved = !paused && (now_at - expected).length() > 25.0;
                if moved {
                    log!("faith: the character was moved {:.0} m by something else (map switch or teleport)", (now_at - expected).length());
                }
                if pressed.toggle || current != f.ped || !player_ok(f.ped) || fell || moved {
                    leave(&f, &cfg, &shared);
                    if let Some(s) = &sound {
                        let _ = s.tx.send(Msg::StopAll);
                    }
                    say(if moved { "Faith mode off (you were moved). Tap F8 once you've landed." } else { "Faith mode off" }, if moved { 4000 } else { 1500 });
                    Mode::Off
                } else {
                    // Body or head changed in the menu: set first person up again with it.
                    if menu_action == crate::menu::Action::View {
                        f.body_mode = cfg.body.clone();
                        f.own_body = cfg.body == "gta";
                        f.head_hide = cfg.head_hide.clone();
                        if f.cam != 0 || f.gta_fp {
                            third_on(&mut f);
                            first_on(&mut f);
                        }
                    }
                    gta_tuning(&mut f.ctrl.tuning, &cfg);
                    let world = shared.lock().unwrap().world.as_ref().filter(|(_, o)| *o == f.origin).map(|(w, _)| w.clone());
                    for c in TAKEN {
                        native!(DISABLE_CONTROL_ACTION, 0, c, true);
                    }
                    // GTA's camera button stays off (first person only; third person was dropped).
                    native!(DISABLE_CONTROL_ACTION, 0, 0, true);
                    if f.cam != 0 {
                        for c in TAKEN_FIRST_PERSON {
                            native!(DISABLE_CONTROL_ACTION, 0, c, true);
                        }
                    }
                    if paused {
                        render::publish(None);
                    } else if let Some(world) = world
                        && tick(&mut f, &world, &shared, axis, held, pressed, &cfg, &mut clips, sound.as_mut())
                    {
                        // A deadly fall: out of Faith mode, GTA's ragdoll takes over.
                        leave(&f, &cfg, &shared);
                        if let Some(s) = &sound {
                            let _ = s.tx.send(Msg::StopAll);
                        }
                        native!(SET_PED_TO_RAGDOLL, f.ped, 3000, 3000, 0, false, false, false);
                        say("Deadly fall", 2000);
                        log!("faith: deadly fall");
                        return_off = true;
                    }
                    if return_off { Mode::Off } else { Mode::On(f) }
                }
            }
        };
    }
}

fn enter(ped: i32, origin: Vec3, cfg: &Config, loaded: Option<Arc<Loaded>>) -> Faith {
    let p = ret_vec3(native!(GET_ENTITY_COORDS, ped, true));
    let heading = ret_f32(native!(GET_ENTITY_HEADING, ped));
    let vel = ret_vec3(native!(GET_ENTITY_VELOCITY, ped));
    native!(CLEAR_PED_TASKS_IMMEDIATELY, ped);
    hold(ped, true);
    let feet = to_faith(p - Vec3::Z * cfg.ped_z_offset, origin) + Vec3::Y * 0.05;
    let mut ctrl = Controller::new(Tuning::default(), feet, heading.to_radians());
    ctrl.vel = dir_to_faith(vel);
    log!("faith: on at ({:.1}, {:.1}, {:.1}) heading {:.0}", p.x, p.y, p.z, heading);
    say("Faith mode on", 1500);
    let body = loaded.map(|l| Body::new(l, &mut ctrl));
    // GTA's walls have sills and trims that Mirror's Edge's don't: climb round them (and more).
    gta_tuning(&mut ctrl.tuning, cfg);
    // With the body, the camera's bob and tilt come from the animations (as in Faith Runner).
    let fx = if body.is_some() { CameraFx::new(CameraFxSettings::animation_driven()) } else { CameraFx::default() };
    if body.is_some() {
        log!("faith: with Faith's arms and legs");
    }
    let mut f = Faith {
        ped,
        ctrl,
        origin,
        pose: None,
        started: None,
        last_state: "",
        cam: 0,
        fx,
        look: LookLimiter::default(),
        dims: Dimensions::default(),
        near_bodies: 0,
        body,
        third: None,
        own_body: cfg.body == "gta",
        own: None,
        head_hide: cfg.head_hide.clone(),
        body_mode: cfg.body.clone(),
        gta_fp: false,
        view_mode_before: ret_i32(native!(GET_FOLLOW_PED_CAM_VIEW_MODE)),
    };
    first_on(&mut f);
    f
}

fn leave(f: &Faith, cfg: &Config, shared: &collision::Handle) {
    shared.lock().unwrap().active = false;
    if f.own_body && f.head_hide != "shrink" {
        native!(SET_FOLLOW_PED_CAM_VIEW_MODE, f.view_mode_before);
    }
    render::publish(None);
    crate::posehook::uninstall();
    if ret_bool(native!(DOES_ENTITY_EXIST, f.ped)) {
        let p = to_gta(f.ctrl.feet, f.origin) + Vec3::Z * cfg.ped_z_offset;
        native!(STOP_ANIM_TASK_ANY, f.ped);
        hold(f.ped, false);
        native!(SET_ENTITY_COORDS_NO_OFFSET, f.ped, p.x, p.y, p.z, false, false, false);
        let v = dir_to_gta(f.ctrl.vel);
        native!(SET_ENTITY_VELOCITY, f.ped, v.x, v.y, v.z);
    }
    if f.cam != 0 {
        if ret_bool(native!(DOES_ENTITY_EXIST, f.ped)) {
            native!(SET_ENTITY_VISIBLE, f.ped, true, false);
        }
        native!(RENDER_SCRIPT_CAMS, false, false, 0, true, false, 0);
        if ret_bool(native!(DOES_CAM_EXIST, f.cam)) {
            native!(SET_CAM_ACTIVE, f.cam, false);
            native!(DESTROY_CAM, f.cam, false);
        }
        native!(SET_GAMEPLAY_CAM_RELATIVE_HEADING, 0.0f32);
    }
    log!("faith: off");
}

// Stopping our clip: clearing the ped's tasks (not immediately, so GTA blends out).
const STOP_ANIM_TASK_ANY: u64 = CLEAR_PED_TASKS;

#[allow(clippy::too_many_arguments)]
fn tick(
    f: &mut Faith,
    world: &Arc<MeshWorld>,
    shared: &collision::Handle,
    axis: Vec2,
    held: Buttons,
    pressed: Buttons,
    cfg: &Config,
    clips: &mut [(Pose, Clip)],
    mut sound: Option<&mut Sound>,
) -> bool {
    let mut died = false;
    let dt = ret_f32(native!(GET_FRAME_TIME)).clamp(0.0, 0.1);
    if dt <= 0.0 {
        return false;
    }
    let first_person = f.cam != 0;
    let (look, cam_yaw) = if first_person {
        // Mirror's Edge: the mouse / right stick turn Faith, and her eye is the camera.
        let lr = ret_f32(native!(GET_DISABLED_CONTROL_NORMAL, 0, 1));
        let ud = ret_f32(native!(GET_DISABLED_CONTROL_NORMAL, 0, 2));
        let scale = if ret_bool(native!(IS_USING_KEYBOARD_AND_MOUSE, 2)) { cfg.mouse_sensitivity } else { cfg.pad_sensitivity * dt };
        (Vec2::new(-lr, -ud) * scale, 0.0)
    } else {
        // GTA's camera is the view: Faith turns toward where it looks.
        let cam = ret_vec3(native!(GET_GAMEPLAY_CAM_ROT, 2));
        let (cam_yaw, cam_pitch) = (cam.z.to_radians(), cam.x.to_radians());
        (Vec2::new(wrap(cam_yaw - f.ctrl.yaw), (cam_pitch - f.ctrl.pitch).clamp(-1.0, 1.0)), cam_yaw)
    };
    let yaw_before = f.ctrl.yaw;

    // Cars and props near her, as a moving layer over the map.
    let here = to_gta(f.ctrl.feet, f.origin);
    let (moving, cars, near) = dynamic::moving_world(here, 15.0, f.origin, shared, &mut f.dims);
    if near != f.near_bodies && (near == 0 || f.near_bodies == 0 || near.abs_diff(f.near_bodies) >= 5) {
        log!("dynamic: {near} cars and props within 15 m");
        f.near_bodies = near;
    }
    let both = dynamic::GtaWorld { layered: faith_move::world::Layered { still: world.as_ref(), moving: &moving }, cars: &cars };
    let mut last_input = Input::default();
    let steps = (dt / (1.0 / 60.0)).ceil().clamp(1.0, 6.0) as usize;
    let h = dt / steps as f32;
    for i in 0..steps {
        let first = i == 0;
        let input = Input {
            move_axis: axis,
            look: if first { look } else { Vec2::ZERO },
            jump_pressed: first && pressed.jump,
            jump_held: held.jump,
            crouch_pressed: first && pressed.crouch,
            crouch_held: held.crouch,
            turn_pressed: first && pressed.turn,
            melee_pressed: first && pressed.melee,
            takedown_pressed: false,
            strafe_raw: 0.0,
        };
        let before = (f.ctrl.feet, f.ctrl.state.name());
        f.ctrl.step(h, &input, &both);
        died |= f.ctrl.events.iter().any(|e| matches!(e, faith_move::Event::Death));
        through_wall_guard(&mut f.ctrl, before, &both);
        if first {
            last_input = input;
        }
    }

    // Yaw the moves added themselves (180 turns, wall kick-offs): the camera turns with her.
    let extra = wrap(f.ctrl.yaw - yaw_before - look.x);
    let body_yaw = match f.ctrl.state {
        State::Slide { .. } => f.ctrl.slide_yaw,
        _ => f.ctrl.yaw,
    };
    if !first_person && extra.abs() > 0.01 {
        native!(SET_GAMEPLAY_CAM_RELATIVE_HEADING, wrap(cam_yaw + extra - body_yaw).to_degrees());
    }

    let p = to_gta(f.ctrl.feet, f.origin) + Vec3::Z * cfg.ped_z_offset;
    native!(SET_ENTITY_COORDS_NO_OFFSET, f.ped, p.x, p.y, p.z, false, false, false);
    // Third person: the body faces where she's going (or the wall she's on).
    let body_yaw = if first_person || f.gta_fp { body_yaw } else { facing(&f.ctrl).unwrap_or(body_yaw) };
    native!(SET_ENTITY_HEADING, f.ped, body_yaw.to_degrees());
    if f.gta_fp {
        // GTA's own first-person camera: the view is GTA's; the body wears Faith's first-person pose.
        native!(SET_FOLLOW_PED_CAM_VIEW_MODE, 4);
        f.look.apply(&mut f.ctrl, dt);
        let shot = f.fx.update(dt, &f.ctrl, &last_input);
        if let (Some(b), Some(o)) = (f.body.as_mut(), f.own.as_ref()) {
            let frame = b.update(dt, &f.ctrl, &shot, 90f32.to_radians(), 16.0 / 9.0, false);
            native!(SET_ENTITY_HEADING, f.ped, body_heading(&frame, body_yaw).to_degrees());
            let mut pose = crate::retarget::PoseOut::default();
            o.retarget.pose(&o.gta, b.globals(), false, &mut pose);
            crate::posehook::publish(Some(Arc::new(pose)));
        }
        if let Some(snd) = sound.as_deref_mut() {
            play_sounds(f, snd, dt, shot.step_phase);
        }
    } else if !first_person && let Some(d) = f.third.as_mut() {
        crate::posehook::publish(Some(Arc::new(d.update(dt, &f.ctrl))));
    }

    if first_person {
        // Faith Runner's own camera: look limits and swan neck, then bob, tilt and FOV.
        f.look.apply(&mut f.ctrl, dt);
        let shot = f.fx.update(dt, &f.ctrl, &last_input);
        let view = shot.view;
        let aspect = ret_f32(native!(GET_ASPECT_RATIO, false)).clamp(1.0, 4.0);
        let world_hfov = view.fov_deg.to_radians();
        // Faith's FOV is horizontal; GTA's camera takes the vertical one.
        let vfov = (2.0 * ((world_hfov * 0.5).tan() / aspect).atan()).to_degrees();
        let (eye, pitch, roll, yaw) = if let Some(b) = f.body.as_mut() {
            // The camera is the body's camera bone (Faith Runner's Rig), as in the game.
            let frame = b.update(dt, &f.ctrl, &shot, world_hfov, aspect, f.own.is_none() && f.body_mode == "faith");
            let (pitch, roll, yaw) = gta_angles(frame.cam_rot);
            match f.own.as_ref() {
                Some(o) => {
                    let body_yaw = body_heading(&frame, body_yaw);
                    native!(SET_ENTITY_HEADING, f.ped, body_yaw.to_degrees());
                    // The character wears Faith's first-person pose, its head shrunk away; the
                    // camera sits at its head (so its arms and legs meet the view as hers do).
                    let mut pose = crate::retarget::PoseOut::default();
                    // HeadHide = gta: tell GTA it's in its first-person view, so it may hide the
                    // head itself (in the view only, keeping it in the shadow); shrink: do it here.
                    if f.head_hide == "gta" {
                        native!(SET_FOLLOW_PED_CAM_VIEW_MODE, 4);
                    }
                    o.retarget.pose(&o.gta, b.globals(), f.head_hide == "shrink", &mut pose);
                    let head = pose.objects[o.head].w_axis.truncate();
                    let turn = glam::Quat::from_rotation_z(body_yaw);
                    let along = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
                    // HeadHide = gta: where GTA's own first-person camera would be (it hid only
                    // part of the head with the camera at the head bone).
                    let eye = if f.head_hide == "gta" {
                        ret_vec3(native!(GET_GAMEPLAY_CAM_COORD))
                    } else {
                        p + turn * head + Vec3::Z * 0.07 + along * 0.08
                    };
                    crate::posehook::publish(Some(Arc::new(pose)));
                    (eye, pitch, roll, yaw)
                }
                None => (to_gta(frame.cam_pos, f.origin), pitch, roll, yaw),
            }
        } else {
            (to_gta(view.eye, f.origin), view.pitch, -view.roll, view.yaw)
        };
        let roll = if cfg.invert_roll { -roll } else { roll };
        native!(SET_CAM_COORD, f.cam, eye.x, eye.y, eye.z);
        native!(SET_CAM_ROT, f.cam, pitch.to_degrees(), roll.to_degrees(), yaw.to_degrees(), 2);
        native!(SET_CAM_FOV, f.cam, vfov.clamp(20.0, 120.0));

        // Faith's sounds: the animations' own cues, landings, slides, wind.
        if let Some(snd) = sound.as_deref_mut() {
            play_sounds(f, snd, dt, shot.step_phase);
        }
    }

    let name = f.ctrl.state.name();
    if name != f.last_state {
        log!("faith: {name} at ({:.1}, {:.1}, {:.1}) speed {:.1}", p.x, p.y, p.z, f.ctrl.vel.length());
        // Finding ladders: which GTA materials are in front of her when she climbs or hangs.
        if matches!(f.ctrl.state, State::WallClimb { .. } | State::LedgeHang { .. }) {
            log_wall_materials(f, world, shared);
        }
        f.last_state = name;
    }
    if died {
        return true;
    }

    // The plain GTA clip for this move (third person only: in first person the body is hidden,
    // and the clips didn't play on the held character in the first test anyway).
    if first_person || f.gta_fp || f.third.is_some() {
        return false;
    }
    let pose = pose_of(&f.ctrl);
    if pose == Pose::Idle {
        if f.pose.is_some_and(|p| p != Pose::Idle) {
            native!(STOP_ANIM_TASK_ANY, f.ped);
        }
        f.pose = Some(Pose::Idle);
        return false;
    }
    if let Some((_, clip)) = clips.iter_mut().find(|(p, _)| *p == pose)
        && let Some((d, c)) = clip.ready()
    {
        let playing = ret_bool(native!(IS_ENTITY_PLAYING_ANIM, f.ped, d.as_ptr(), c.as_ptr(), 3));
        if f.pose == Some(pose) && !playing && f.started.is_some_and(|t| t.elapsed().as_secs_f32() > 0.5) {
            // Started half a second ago and still not playing: not a real clip.
            clip.reject();
            native!(STOP_ANIM_TASK_ANY, f.ped);
            f.pose = None;
            f.started = None;
            return false;
        }
        if f.pose != Some(pose) || (!playing && f.started.is_none()) {
            native!(TASK_PLAY_ANIM, f.ped, d.as_ptr(), c.as_ptr(), 4.0f32, -4.0f32, -1, 1, 0.0f32, false, false, false);
            f.pose = Some(pose);
            f.started = Some(Instant::now());
        }
        if playing {
            f.started = None;
        }
        if pose == Pose::Run {
            let rate = (f.ctrl.horizontal_speed() / 4.0).clamp(0.6, 1.8);
            native!(SET_ENTITY_ANIM_SPEED, f.ped, d.as_ptr(), c.as_ptr(), rate);
        }
    }
    false
}

/// GTA camera angles (pitch, roll, heading; radians) for a camera rotation in Faith's
/// space (looking down -Z, Y up).
fn gta_angles(rot: glam::Quat) -> (f32, f32, f32) {
    let f = dir_to_gta(rot * Vec3::NEG_Z).normalize_or_zero();
    let u = dir_to_gta(rot * Vec3::Y).normalize_or_zero();
    let heading = (-f.x).atan2(f.y);
    let pitch = f.z.clamp(-1.0, 1.0).asin();
    let right = f.cross(u);
    let level = f.cross(Vec3::Z).normalize_or_zero();
    let roll = level.cross(right).dot(f).atan2(level.dot(right));
    (pitch, roll, heading)
}

/// Which way Faith's body faces (GTA heading, radians): her rig keeps it on the wall or ladder
/// while she hangs or climbs, and lets only the view turn.
fn body_heading(frame: &faith_anim::RigFrame, fallback: f32) -> f32 {
    let fw = frame.body_rot * Vec3::NEG_Z;
    if fw.x * fw.x + fw.z * fw.z > 0.01 { (-fw.x).atan2(-fw.z) } else { fallback }
}

/// Third person: which way the body faces (Faith's space yaw), when not just the view's.
fn facing(c: &Controller) -> Option<f32> {
    let yaw_of = |d: Vec3| (-d.x).atan2(-d.z);
    let flat = Vec3::new(c.vel.x, 0.0, c.vel.z);
    match c.state {
        State::WallClimb { normal, .. } | State::WallClimbTurned { normal, .. } | State::LedgeHang { normal, .. } => Some(yaw_of(-normal)),
        State::Slide { .. } => Some(c.slide_yaw),
        _ if flat.length() > 0.5 => Some(yaw_of(flat)),
        _ => None,
    }
}

/// The GTA settings of Faith's movement (Mirror's Edge's own values elsewhere).
fn gta_tuning(t: &mut Tuning, cfg: &Config) {
    t.overhang_step_out = cfg.overhang_step_out;
    t.step_height = cfg.step_height;
    t.auto_step_up_max = cfg.step_up.max(cfg.step_height + 0.05);
    t.lethal_fall_height = cfg.lethal_fall;
    t.ladder_climb_speed = cfg.ladder_speed;
    t.step_up_keep = 0.9;
    // GTA's own consequences instead of a respawn at the checkpoint.
    t.respawn_on_death = false;
    // GTA's steps come in all heights, and walls rise behind them: springboards work off both.
    t.springboard_into_walls = true;
    // ...and a roof's parapet launches you on over the gap instead of being vaulted.
    t.springboard_into_air = true;
    t.springboard_slack = 0.3;
}

/// The player is Faith's add-on ped (her voice fits whatever body is drawn).
static FAITH_PED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn is_faith_model(h: u32) -> bool {
    h == crate::ladders::joaat("faith") || h == crate::ladders::joaat("faith2")
}

fn play_sounds(f: &Faith, snd: &mut Sound, dt: f32, step_phase: f32) {
    let Some(director) = snd.director.as_mut() else { return };
    let mut out: Vec<SoundCmd> = vec![];
    let notifies = f.body.as_ref().map(|b| b.rig.driver.notifies().to_vec()).unwrap_or_default();
    director.update(dt, &f.ctrl, &notifies, f.body.is_some(), step_phase, &|_| Surface::Concrete, &mut out);
    for cmd in out {
        // Faith's female voice (breathing, effort) only with Faith's own body.
        if f.body_mode != "faith" && !FAITH_PED.load(std::sync::atomic::Ordering::Relaxed) && matches!(&cmd, SoundCmd::Play { cue, .. } if cue.starts_with("a_character_female")) {
            continue;
        }
        let _ = snd.tx.send(Msg::Cmd(cmd));
    }
}

/// Logs the GTA materials of the map triangles just in front of her chest (to find which
/// material GTA's ladders use).
fn log_wall_materials(f: &Faith, world: &Arc<MeshWorld>, shared: &collision::Handle) {
    let Some(mats) = shared.lock().unwrap().materials.clone() else { return };
    if mats.len() != world.tris.len() {
        return;
    }
    let p = f.ctrl.feet + Vec3::Y * 1.2 + f.ctrl.forward() * 0.4;
    let mut count = std::collections::BTreeMap::<u8, usize>::new();
    for (t, m) in world.tris.iter().zip(mats.iter()) {
        let c = (t[0] + t[1] + t[2]) / 3.0;
        if (c - p).length() < 1.0 {
            *count.entry(*m).or_default() += 1;
        }
    }
    log!("faith: wall materials here (material: triangles): {count:?}");
}

/// The safety net against ending up inside a building: GTA's buildings are hollow shells, so
/// a scripted move (vault, pull-up, step-up, springboard...) whose "is there room?" check found
/// the empty inside could carry her through the wall. If her head's path this step (1.5 m above
/// the feet) crosses a wall, the step is undone and she drops where she was.
pub(crate) fn through_wall_guard(c: &mut Controller, before: (Vec3, &'static str), world: &dyn faith_move::World) -> bool {
    let (from, state) = before;
    // Ladders go round the wall's edge at the top by design.
    if matches!(state, "Onto ladder" | "Ladder" | "Off ladder") || matches!(c.state, State::IntoClimb { .. } | State::Climb { .. } | State::ClimbExit { .. }) {
        return false;
    }
    let to = c.feet;
    let d = to - from;
    if d.length_squared() < 1e-6 {
        return false;
    }
    let half = Vec3::splat(0.04);
    let a = from + Vec3::Y * 1.5;
    if let Some(hit) = world.sweep(half, a, d)
        && hit.normal.y.abs() < 0.5
    {
        log!("faith: stopped going through a wall ({state} -> {}) at ({:.1}, {:.1}, {:.1})", c.state.name(), to.x, to.y, to.z);
        c.feet = from;
        c.vel = Vec3::new(0.0, c.vel.y.min(0.0), 0.0);
        if !matches!(c.state, State::Ground | State::Air) {
            c.state = State::Air;
        }
        return true;
    }
    false
}
