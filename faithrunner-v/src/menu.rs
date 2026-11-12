//! The in-game settings menu: hold F8 (or both sticks clicked) for half a second.
//! Up/down picks a line, left/right changes it, Enter/A runs an action; tap F8 or B closes.
//! Changes apply at once; "Save settings" writes them to FaithRunnerV.ini.
use std::ffi::CString;
use std::time::Instant;

use crate::config::Config;
use crate::natives::*;
use crate::{native, win};

/// What the game should do after a menu change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    /// Body or head setting changed: set the first-person view up again.
    View,
    /// A sound volume changed.
    Volumes,
    Save,
    /// Search memory for GTA's ladders near the player (log only, research).
    Ladders,
    /// The character model changed.
    Model,
}

#[derive(Clone, Copy)]
enum Item {
    Choice { label: &'static str, options: &'static [&'static str], get: fn(&Config) -> String, set: fn(&mut Config, &str), action: Action },
    Number { label: &'static str, min: f32, max: f32, step: f32, get: fn(&Config) -> f32, set: fn(&mut Config, f32), action: Action, percent: bool },
    Button { label: &'static str, action: Action },
    /// A group of settings: Enter / A / right opens it, Backspace / B goes back.
    Sub { label: &'static str, items: &'static [Item] },
}

const CHARACTER: &[Item] = &[
        Item::Choice {
            label: "Character",
            options: &["game", "faith", "faith2"],
            get: |c| c.ped_model.clone(),
            set: |c, v| c.ped_model = v.into(),
            action: Action::Model,
        },
        Item::Choice {
            label: "First-person body",
            options: &["faith", "gta", "none"],
            get: |c| c.body.clone(),
            set: |c, v| c.body = v.into(),
            action: Action::View,
        },
        Item::Choice {
            label: "Hide head (GTA body)",
            options: &["gta", "gtacam", "shrink"],
            get: |c| c.head_hide.clone(),
            set: |c, v| c.head_hide = v.into(),
            action: Action::View,
        },
];

const MOVEMENT: &[Item] = &[
        Item::Number { label: "Climb round sills (m)", min: 0.0, max: 1.0, step: 0.05, get: |c| c.overhang_step_out, set: |c, v| c.overhang_step_out = v, action: Action::None, percent: false },
        Item::Number { label: "Run over things up to (m)", min: 0.1, max: 0.8, step: 0.05, get: |c| c.step_height, set: |c, v| c.step_height = v, action: Action::None, percent: false },
        Item::Number { label: "Step up onto (m)", min: 0.3, max: 1.2, step: 0.05, get: |c| c.step_up, set: |c, v| c.step_up = v, action: Action::None, percent: false },
        Item::Number { label: "Ladder climb speed", min: 0.5, max: 3.0, step: 0.1, get: |c| c.ladder_speed, set: |c, v| c.ladder_speed = v, action: Action::None, percent: true },
        Item::Number { label: "Deadly fall (m)", min: 5.0, max: 200.0, step: 1.0, get: |c| c.lethal_fall, set: |c, v| c.lethal_fall = v, action: Action::None, percent: false },
];

const SOUND: &[Item] = &[
        Item::Number { label: "All sounds", min: 0.0, max: 2.0, step: 0.05, get: |c| c.sound_volume, set: |c, v| c.sound_volume = v, action: Action::Volumes, percent: true },
        Item::Number { label: "Footsteps", min: 0.0, max: 2.0, step: 0.05, get: |c| c.footstep_volume, set: |c, v| c.footstep_volume = v, action: Action::Volumes, percent: true },
        Item::Number { label: "Hand grabs", min: 0.0, max: 2.0, step: 0.05, get: |c| c.hand_volume, set: |c, v| c.hand_volume = v, action: Action::Volumes, percent: true },
        Item::Number { label: "Voice", min: 0.0, max: 2.0, step: 0.05, get: |c| c.voice_volume, set: |c, v| c.voice_volume = v, action: Action::Volumes, percent: true },
        Item::Number { label: "Effects", min: 0.0, max: 2.0, step: 0.05, get: |c| c.effects_volume, set: |c, v| c.effects_volume = v, action: Action::Volumes, percent: true },
];

const CONTROLS: &[Item] = &[
        Item::Number { label: "Mouse look speed", min: 0.02, max: 0.5, step: 0.01, get: |c| c.mouse_sensitivity, set: |c, v| c.mouse_sensitivity = v, action: Action::None, percent: false },
        Item::Number { label: "Stick look speed", min: 0.5, max: 8.0, step: 0.1, get: |c| c.pad_sensitivity, set: |c, v| c.pad_sensitivity = v, action: Action::None, percent: false },
        Item::Choice {
            label: "Wallrun tilt",
            options: &["normal", "flipped"],
            get: |c| (if c.invert_roll { "flipped" } else { "normal" }).into(),
            set: |c, v| c.invert_roll = v == "flipped",
            action: Action::None,
        },
];

const TOOLS: &[Item] = &[
        Item::Button { label: "Search for ladders (to the log)", action: Action::Ladders },
];

const ITEMS: &[Item] = &[
    Item::Sub { label: "Character & view", items: CHARACTER },
    Item::Sub { label: "Movement", items: MOVEMENT },
    Item::Sub { label: "Sound", items: SOUND },
    Item::Sub { label: "Controls", items: CONTROLS },
    Item::Sub { label: "Tools", items: TOOLS },
    Item::Button { label: "Save settings", action: Action::Save },
];

#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    ok: bool,
    back: bool,
}

pub struct Menu {
    pub open: bool,
    sel: usize,
    /// The groups opened, with the line each was opened from.
    path: Vec<(usize, &'static [Item], &'static str)>,
    prev: Keys,
    /// When left/right started being held (for repeat).
    held_since: Option<Instant>,
    last_repeat: Option<Instant>,
    note: Option<(String, Instant)>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { open: false, sel: 0, path: Vec::new(), prev: Keys::default(), held_since: None, last_repeat: None, note: None }
    }

    fn items(&self) -> &'static [Item] {
        self.path.last().map_or(ITEMS, |p| p.1)
    }

    pub fn note(&mut self, text: &str) {
        self.note = Some((text.into(), Instant::now()));
    }

    fn keys() -> Keys {
        let mut k = Keys::default();
        if let Some(p) = win::pad() {
            let b = p.buttons;
            k.up |= b & 0x1 != 0;
            k.down |= b & 0x2 != 0;
            k.left |= b & 0x4 != 0;
            k.right |= b & 0x8 != 0;
            k.ok |= b & 0x1000 != 0;
            k.back |= b & 0x2000 != 0;
        }
        if win::game_focused() {
            let d = win::key_down;
            k.up |= d(0x26);
            k.down |= d(0x28);
            k.left |= d(0x25);
            k.right |= d(0x27);
            k.ok |= d(0x0D);
            k.back |= d(0x08);
        }
        k
    }

    /// One frame with the menu open: input, then drawing. Returns what changed.
    pub fn tick(&mut self, cfg: &mut Config) -> Action {
        // The menu has the keys and buttons (look stays GTA's when not in Faith mode).
        native!(DISABLE_ALL_CONTROL_ACTIONS, 0);
        native!(ENABLE_CONTROL_ACTION, 0, 1, true);
        native!(ENABLE_CONTROL_ACTION, 0, 2, true);
        let k = Self::keys();
        let edge = |now: bool, before: bool| now && !before;
        let mut action = Action::None;
        let items = self.items();
        if edge(k.back, self.prev.back) {
            // Back up a level; at the top, close.
            match self.path.pop() {
                Some((from, _, _)) => self.sel = from,
                None => self.open = false,
            }
            self.prev = k;
            self.draw(cfg);
            return action;
        }
        if edge(k.up, self.prev.up) {
            self.sel = (self.sel + items.len() - 1) % items.len();
        }
        if edge(k.down, self.prev.down) {
            self.sel = (self.sel + 1) % items.len();
        }
        self.sel = self.sel.min(items.len() - 1);
        // Left/right: once, then repeating while held.
        let side = (k.right as i32) - (k.left as i32);
        let mut step = 0;
        if side != 0 {
            let now = Instant::now();
            match self.held_since {
                None => {
                    self.held_since = Some(now);
                    self.last_repeat = Some(now);
                    step = side;
                }
                Some(t) if now - t > std::time::Duration::from_millis(400)
                    && self.last_repeat.is_none_or(|r| now - r > std::time::Duration::from_millis(70)) =>
                {
                    self.last_repeat = Some(now);
                    step = side;
                }
                _ => {}
            }
        } else {
            self.held_since = None;
        }
        match items[self.sel] {
            Item::Sub { label, items: sub } if edge(k.ok, self.prev.ok) || (step > 0 && self.held_since.is_some_and(|t| t.elapsed().as_millis() < 50)) => {
                self.path.push((self.sel, sub, label));
                self.sel = 0;
            }
            Item::Choice { options, get, set, action: a, .. } if step != 0 => {
                let cur = get(cfg);
                let i = options.iter().position(|o| *o == cur).unwrap_or(0) as i32;
                let n = options.len() as i32;
                set(cfg, options[((i + step).rem_euclid(n)) as usize]);
                action = a;
            }
            Item::Number { min, max, step: s, get, set, action: a, .. } if step != 0 => {
                let v = ((get(cfg) + s * step as f32) / s).round() * s;
                set(cfg, v.clamp(min, max));
                action = a;
            }
            Item::Button { action: a, .. } if edge(k.ok, self.prev.ok) => action = a,
            _ => {}
        }
        self.prev = k;
        self.draw(cfg);
        action
    }

    fn draw(&self, cfg: &Config) {
        let (x, w, row) = (0.80f32, 0.30f32, 0.032f32);
        let top = 0.22f32;
        let items = self.items();
        let rows = items.len() as f32 + 2.0;
        native!(DRAW_RECT, x, top + row * rows * 0.5 - row * 0.5, w, row * rows, 10, 10, 14, 200, false);
        native!(DRAW_RECT, x, top, w, row, 200, 30, 30, 230, false);
        let title = match self.path.last() {
            Some((_, _, label)) => format!("Faith Runner V  >  {label}"),
            None => "Faith Runner V".into(),
        };
        text(&title, x - w * 0.47, top - row * 0.42, 255, 0.40);
        for (i, item) in items.iter().enumerate() {
            let y = top + row * (i as f32 + 1.0);
            if i == self.sel {
                native!(DRAW_RECT, x, y, w, row, 235, 235, 235, 230, false);
            }
            let shade = if i == self.sel { 20 } else { 235 };
            let (label, value) = match item {
                Item::Choice { label, get, .. } => (*label, format!("< {} >", get(cfg))),
                Item::Number { label, get, percent: true, .. } => (*label, format!("< {:.0}% >", get(cfg) * 100.0)),
                Item::Number { label, get, .. } => (*label, format!("< {:.2} >", get(cfg))),
                Item::Button { label, .. } => (*label, String::new()),
                Item::Sub { label, .. } => (*label, ">".into()),
            };
            text_rgb(label, x - w * 0.47, y - row * 0.42, shade, 0.34);
            text_rgb(&value, x + w * 0.08, y - row * 0.42, shade, 0.34);
        }
        let help = match &self.note {
            Some((n, t)) if t.elapsed().as_secs_f32() < 3.0 => n.clone(),
            _ if self.path.is_empty() => "Enter / A: open   Arrows: choose   Tap F8: close".into(),
            _ => "Arrows / D-pad: change   Backspace / B: back   Tap F8: close".into(),
        };
        let y = top + row * (items.len() as f32 + 1.0);
        text_rgb(&help, x - w * 0.47, y - row * 0.38, 200, 0.26);
    }
}

fn text(s: &str, x: f32, y: f32, shade: i32, scale: f32) {
    text_rgb(s, x, y, shade, scale)
}

fn text_rgb(s: &str, x: f32, y: f32, shade: i32, scale: f32) {
    let c = CString::new(s).unwrap_or_default();
    native!(SET_TEXT_FONT, 0);
    native!(SET_TEXT_SCALE, scale, scale);
    native!(SET_TEXT_COLOUR, shade, shade, shade, 255);
    native!(BEGIN_TEXT_COMMAND_DISPLAY_TEXT, c"STRING".as_ptr());
    native!(ADD_TEXT_COMPONENT_SUBSTRING_PLAYER_NAME, c.as_ptr());
    native!(END_TEXT_COMMAND_DISPLAY_TEXT, x, y, 0);
}
