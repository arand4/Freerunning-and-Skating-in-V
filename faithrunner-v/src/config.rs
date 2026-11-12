//! FaithRunnerV.ini next to the ASI: `Key = value` lines, `;` comments.
use crate::{log, module_dir};

#[derive(Clone, Debug)]
pub struct Config {
    /// Keyboard key that switches Faith mode on and off (Windows virtual-key code).
    pub toggle_key: i32,
    /// A key that opens the settings menu with one press (for the Mod Hub; F21 by default).
    pub menu_key: i32,
    /// Controller: both sticks clicked together switch Faith mode.
    pub pad_toggle: bool,
    /// First person: Faith's own camera at her eye (Mirror's Edge). False: GTA's camera.
    pub first_person: bool,
    /// Mouse look: radians per unit of GTA's look input per frame.
    pub mouse_sensitivity: f32,
    /// Right stick look: radians per second at full tilt.
    pub pad_sensitivity: f32,
    /// Flip the camera's head tilt if it leans the wrong way.
    pub invert_roll: bool,
    /// First-person body: "faith" (her Mirror's Edge arms and legs) or "none".
    /// ("gta", the active GTA character with head and torso hidden, is planned.)
    pub body: String,
    /// The player's character model: "game" (whoever the game gives you), or Faith's add-on ped
    /// (alex189's Mirror's Edge Catalyst Faith, pack faithcatalyst): "faith" (game outfit),
    /// "faith2" (prologue outfit). Works in and out of Faith mode.
    pub ped_model: String,
    /// Body = gta, first person: how the head is kept out of view. "gta": GTA is told it's in
    /// its first-person view (its own head hiding), Faith's camera as usual; "gtacam": GTA's
    /// own first-person camera; "shrink": the head bone shrunk to a point.
    pub head_hide: String,
    /// Faith's sounds from Mirror's Edge (with her body).
    pub sounds: bool,
    pub sound_volume: f32,
    /// Volume per kind of sound (times SoundVolume).
    pub footstep_volume: f32,
    pub hand_volume: f32,
    pub voice_volume: f32,
    pub effects_volume: f32,
    /// The Mirror's Edge install folder.
    pub mirrors_edge: String,
    /// Wallclimbs step out round overhangs (sills, trims) up to this deep, metres. 0 = off (as in ME).
    pub overhang_step_out: f32,
    /// Highest thing she just runs over (kerbs, debris), and the highest she steps up onto
    /// without a vault (GTA has far more small clutter than Mirror's Edge), metres.
    pub step_height: f32,
    pub step_up: f32,
    /// A fall this high (metres) ends Faith mode with GTA's ragdoll (Mirror's Edge: 10 m, death).
    pub lethal_fall: f32,
    /// Ladder climbing speed up, times Mirror's Edge's.
    pub ladder_speed: f32,
    /// GTA ped origin above Faith's feet.
    pub ped_z_offset: f32,
    /// Half size of the collision area read around the player (metres).
    pub collision_radius: f32,
    /// Read the area again after moving this far from where it was read.
    pub rebuild_distance: f32,
    /// GTA clips, as "dictionary, clip" (several tried in order, separated by `|`).
    pub anim_run: String,
    pub anim_air: String,
    pub anim_wall: String,
    pub anim_hang: String,
    pub anim_slide: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            toggle_key: 0x77, // F8 (F7 is World Travel's teleport key)
            menu_key: 0x84,   // F21: no keyboard has it; the Mod Hub sends it
            pad_toggle: true,
            first_person: true,
            mouse_sensitivity: 0.12,
            pad_sensitivity: 3.2,
            invert_roll: false,
            body: "faith".into(),
            ped_model: "game".into(),
            head_hide: "shrink".into(),
            sounds: true,
            sound_volume: 1.0,
            footstep_volume: 0.35,
            hand_volume: 0.8,
            voice_volume: 1.0,
            effects_volume: 1.0,
            mirrors_edge: String::new(),
            overhang_step_out: 0.45,
            step_height: 0.45,
            step_up: 0.8,
            lethal_fall: 20.0,
            ladder_speed: 1.6,
            ped_z_offset: 1.0,
            collision_radius: 64.0,
            rebuild_distance: 20.0,
            anim_run: "move_m@jogger, run | move_m@generic, run".into(),
            anim_air: "move_fall, fall_med | skydive@base, free_idle".into(),
            anim_wall: "move_fall, fall_med | skydive@base, free_idle".into(),
            anim_hang: "move_fall, fall_med | skydive@base, free_idle".into(),
            anim_slide: "move_fall, fall_med | skydive@base, free_idle".into(),
        }
    }
}

/// Writes the menu's settings into FaithRunnerV.ini (other lines and notes kept).
pub fn save(c: &Config) -> std::io::Result<()> {
    let path = module_dir().join("FaithRunnerV.ini");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let values: Vec<(&str, String)> = vec![
        ("Body", c.body.clone()),
        ("PedModel", c.ped_model.clone()),
        ("HeadHide", c.head_hide.clone()),
        ("Camera", (if c.first_person { "first" } else { "third" }).into()),
        ("SoundVolume", format!("{:.2}", c.sound_volume)),
        ("FootstepVolume", format!("{:.2}", c.footstep_volume)),
        ("HandVolume", format!("{:.2}", c.hand_volume)),
        ("VoiceVolume", format!("{:.2}", c.voice_volume)),
        ("EffectsVolume", format!("{:.2}", c.effects_volume)),
        ("MouseSensitivity", format!("{:.2}", c.mouse_sensitivity)),
        ("PadSensitivity", format!("{:.1}", c.pad_sensitivity)),
        ("InvertRoll", (if c.invert_roll { "1" } else { "0" }).into()),
        ("OverhangStepOut", format!("{:.2}", c.overhang_step_out)),
        ("StepHeight", format!("{:.2}", c.step_height)),
        ("StepUp", format!("{:.2}", c.step_up)),
        ("LethalFall", format!("{:.0}", c.lethal_fall)),
        ("LadderSpeed", format!("{:.2}", c.ladder_speed)),
    ];
    let mut done = vec![false; values.len()];
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            let key = line.split(';').next().unwrap_or("").split('=').next().unwrap_or("").trim();
            match values.iter().position(|(k, _)| k.eq_ignore_ascii_case(key) && line.contains('=')) {
                Some(i) => {
                    done[i] = true;
                    format!("{} = {}", values[i].0, values[i].1)
                }
                None => line.to_string(),
            }
        })
        .collect();
    for (i, (k, v)) in values.iter().enumerate() {
        if !done[i] {
            out.push(format!("{k} = {v}"));
        }
    }
    std::fs::write(&path, out.join("\r\n") + "\r\n")?;
    log!("config: saved to {}", path.display());
    Ok(())
}

pub fn load() -> Config {
    let mut c = Config::default();
    let path = module_dir().join("FaithRunnerV.ini");
    let Ok(text) = std::fs::read_to_string(&path) else {
        log!("config: {} not found, using the defaults", path.display());
        return c;
    };
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let f = |d: f32| v.parse().unwrap_or(d);
        match k.as_str() {
            "togglekey" => {
                c.toggle_key = v
                    .strip_prefix("0x")
                    .and_then(|h| i32::from_str_radix(h, 16).ok())
                    .or_else(|| v.parse().ok())
                    .unwrap_or(c.toggle_key)
            }
            "menukey" => {
                c.menu_key = v
                    .strip_prefix("0x")
                    .and_then(|h| i32::from_str_radix(h, 16).ok())
                    .or_else(|| v.parse().ok())
                    .unwrap_or(c.menu_key)
            }
            "padtoggle" => c.pad_toggle = v != "0",
            "camera" => c.first_person = !v.eq_ignore_ascii_case("third") && v != "3",
            "mousesensitivity" => c.mouse_sensitivity = f(c.mouse_sensitivity),
            "padsensitivity" => c.pad_sensitivity = f(c.pad_sensitivity),
            "invertroll" => c.invert_roll = v == "1",
            "body" => c.body = v.to_ascii_lowercase(),
            "pedmodel" => c.ped_model = v.to_ascii_lowercase(),
            "sounds" => c.sounds = v != "0",
            "headhide" => c.head_hide = v.to_ascii_lowercase(),
            "soundvolume" => c.sound_volume = f(c.sound_volume).clamp(0.0, 4.0),
            "footstepvolume" => c.footstep_volume = f(c.footstep_volume).clamp(0.0, 4.0),
            "handvolume" => c.hand_volume = f(c.hand_volume).clamp(0.0, 4.0),
            "voicevolume" => c.voice_volume = f(c.voice_volume).clamp(0.0, 4.0),
            "effectsvolume" => c.effects_volume = f(c.effects_volume).clamp(0.0, 4.0),
            "mirrorsedge" => c.mirrors_edge = v.into(),
            "stepheight" => c.step_height = f(c.step_height).clamp(0.1, 0.8),
            "stepup" => c.step_up = f(c.step_up).clamp(0.3, 1.2),
            "lethalfall" => c.lethal_fall = f(c.lethal_fall).clamp(5.0, 200.0),
            "ladderspeed" => c.ladder_speed = f(c.ladder_speed).clamp(0.5, 3.0),
            "overhangstepout" => c.overhang_step_out = f(c.overhang_step_out).clamp(0.0, 1.0),
            "pedzoffset" => c.ped_z_offset = f(c.ped_z_offset),
            "collisionradius" => c.collision_radius = f(c.collision_radius).clamp(16.0, 200.0),
            "rebuilddistance" => c.rebuild_distance = f(c.rebuild_distance).clamp(4.0, 100.0),
            "animrun" => c.anim_run = v.into(),
            "animair" => c.anim_air = v.into(),
            "animwall" => c.anim_wall = v.into(),
            "animhang" => c.anim_hang = v.into(),
            "animslide" => c.anim_slide = v.into(),
            _ => {}
        }
    }
    log!("config: {c:?}");
    c
}
