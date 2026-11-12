//! Faith's third-person animations (AS_F3P_Unarmed, from the player's install) chosen
//! from her movement state, and put on the GTA character's skeleton (retarget.rs,
//! posehook.rs). Mirror's Edge's own animation names; a simple chooser with crossfades
//! (the game's third-person animation tree, AT_C3P, is a later refinement).
use std::path::Path;
use std::sync::Arc;

use faith_move::{Controller, State, TraverseKind, VAULT_TYPES};
use glam::{Mat4, Vec3};
use me_assets::pose::{Pose, TrackMap, globals, sample};
use me_assets::{AnimSet, Package, SkeletalMesh};

use crate::log;
use crate::retarget::{GtaSkeleton, PoseOut, Retarget};

/// Loaded once, in the background.
pub static LOADED: std::sync::Mutex<Option<Arc<Faith3p>>> = std::sync::Mutex::new(None);

pub fn load_in_background(install: std::path::PathBuf) {
    std::thread::spawn(move || {
        match std::panic::catch_unwind(|| Faith3p::load(&install)).unwrap_or_else(|_| Err("the reader panicked".into())) {
            Ok(f) => {
                log!("third person: Faith's {} third-person animations loaded", f.set.sequences.len());
                *LOADED.lock().unwrap() = Some(Arc::new(f));
            }
            Err(e) => log!("third person: couldn't load Faith's third-person animations: {e}"),
        }
    });
}

pub struct Faith3p {
    mesh: SkeletalMesh,
    set: AnimSet,
    map: TrackMap,
    rest: Pose,
    rest_globals: Vec<Mat4>,
}

impl Faith3p {
    pub fn load(install: &Path) -> Result<Faith3p, String> {
        let cooked = me_assets::cooked_pc(install).ok_or("no TdGame\\CookedPC")?;
        let pkg = Package::open(&cooked.join("Characters").join("CH_TKY_Crim_Fixer.upk")).map_err(|e| e.to_string())?;
        let mesh = SkeletalMesh::read(&pkg, "SK_TKY_Crim_Fixer").map_err(|e| e.to_string())?;
        let set = AnimSet::read(&Package::open(&cooked.join("Animations").join("AS_F3P_Unarmed.upk")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let map = TrackMap::new(&mesh, &set);
        let rest = Pose::rest(&mesh);
        let mut rest_globals = vec![];
        globals(&mesh, &rest, &mut rest_globals);
        Ok(Faith3p { mesh, set, map, rest, rest_globals })
    }

    pub fn retarget(&self, gta: &GtaSkeleton) -> Result<Retarget, String> {
        Retarget::new(gta, &self.mesh, &self.rest_globals)
    }

    fn has(&self, name: &str) -> Option<String> {
        self.set.sequences.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned()
    }
}

/// What plays: a clip, looping or held at its end, at a time or a progress.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    clip: &'static str,
    looping: bool,
    /// Some: play at this fraction of the clip (moves timed by the controller).
    progress: Option<f32>,
    rate: f32,
}

fn choose(c: &Controller) -> Choice {
    let speed = c.horizontal_speed();
    let lp = |clip, rate| Choice { clip, looping: true, progress: None, rate };
    let at = |clip, p: f32| Choice { clip, looping: false, progress: Some(p.clamp(0.0, 1.0)), rate: 1.0 };
    let once = |clip| Choice { clip, looping: false, progress: None, rate: 1.0 };
    // Which side the wall is on, from its normal and where she's going.
    let wall_left = |n: Vec3| {
        let right = Vec3::new(-c.vel.z, 0.0, c.vel.x); // velocity turned right (Y up)
        n.dot(right) > 0.0
    };
    match &c.state {
        State::Ground if c.crouched => if speed > 0.5 { lp("crouchfwd", (speed / 2.0).clamp(0.6, 1.6)) } else { lp("crouchstill", 1.0) },
        State::Ground if speed < 0.3 => lp("Stand", 1.0),
        State::Ground if speed < 2.2 => lp("walkfwd", (speed / 1.5).clamp(0.6, 1.5)),
        State::Ground if speed < 5.0 => lp("runfwd", (speed / 4.0).clamp(0.7, 1.4)),
        State::Ground => lp("SprintFwd", (speed / 6.3).clamp(0.8, 1.3)),
        State::Air if c.vel.y < -14.0 => lp("fallinguncontrolled", 1.0),
        State::Air => once("JumpSlow"),
        State::Slide { .. } => lp("CrouchSlide", 1.0),
        State::Roll { t } => at("fallinglandroll", *t / 1.3),
        State::Stunned { t } => at("fallinglandhard", *t / 1.8),
        State::WallRun { normal, .. } => if wall_left(*normal) { lp("WallrunLeft", 1.0) } else { lp("WallrunRight", 1.0) },
        State::WallClimb { .. } => lp("WallRunVertical", 1.0),
        State::WallClimbTurned { .. } => once("wallrunvertical180turn"),
        State::LedgeHang { turned: true, .. } => lp("HangTurnLeftIdle", 1.0),
        State::LedgeHang { .. } => match &c.shimmy {
            Some(s) if s.dir < 0.0 => lp("HangStrafeLeft", 1.0),
            Some(_) => lp("HangStrafeRight", 1.0),
            None => lp("Hang", 1.0),
        },
        State::Traverse(t) => match t.kind {
            TraverseKind::PullUp => at("HangHeaveUp", t.t / t.dur.max(0.01)),
            TraverseKind::SpringBoard => at("SpringBoardRightLeg", t.t / t.dur.max(0.01)),
            _ => at("VaultOver", t.t / t.dur.max(0.01)),
        },
        State::Vault(v) => {
            let name = VAULT_TYPES[v.kind].anim;
            let p = v.t / v.duration().max(0.01);
            // The 3P set has the same names as the 1P one for most vault types.
            Choice { clip: name, looping: false, progress: Some(p.clamp(0.0, 1.0)), rate: 1.0 }
        }
        State::LayOnGround { .. } => lp("jumpturnlandingidle", 1.0),
        State::Balance { .. } => lp("walkbalancefwd", 1.0),
        _ => lp("Stand", 1.0),
    }
}

/// One session's third-person animation for one GTA skeleton.
pub struct Driver {
    faith: Arc<Faith3p>,
    gta: GtaSkeleton,
    retarget: Retarget,
    current: Option<(String, Choice)>,
    time: f32,
    /// The pose being faded out of, and how far the fade is.
    from: Option<Pose>,
    fade: f32,
    pose: Pose,
    scratch: Pose,
    faith_globals: Vec<Mat4>,
}

const FADE: f32 = 0.15;

impl Driver {
    pub fn new(faith: Arc<Faith3p>, gta: GtaSkeleton) -> Result<Driver, String> {
        let retarget = faith.retarget(&gta)?;
        let pose = faith.rest.clone();
        Ok(Driver { scratch: pose.clone(), pose, faith, gta, retarget, current: None, time: 0.0, from: None, fade: 1.0, faith_globals: vec![] })
    }

    pub fn mapped(&self) -> usize {
        self.retarget.mapped()
    }

    /// This frame's GTA pose for the skeleton.
    pub fn update(&mut self, dt: f32, c: &Controller) -> PoseOut {
        let mut want = choose(c);
        let name = self
            .faith
            .has(want.clip)
            .or_else(|| {
                // A vault type the 3P set lacks: the plain one.
                want.clip = if matches!(c.state, State::Vault(v) if v.onto()) { "VaultOnto" } else { "VaultOver" };
                self.faith.has(want.clip)
            })
            .or_else(|| self.faith.has("Stand"))
            .unwrap_or_default();
        if self.current.as_ref().is_none_or(|(n, _)| *n != name) {
            self.from = Some(self.pose.clone());
            self.fade = 0.0;
            self.time = 0.0;
        }
        self.current = Some((name.clone(), want.clone()));
        self.time += dt * want.rate;
        self.fade = (self.fade + dt / FADE).min(1.0);
        if let Some(seq) = self.faith.set.sequences.get(&name) {
            let t = match want.progress {
                Some(p) => p * seq.length,
                None => self.time,
            };
            sample(seq, &self.faith.map, &self.faith.rest, t, want.looping, &mut self.scratch);
        }
        match &self.from {
            Some(from) if self.fade < 1.0 => {
                self.pose = from.clone();
                self.pose.blend_toward(&self.scratch, self.fade);
            }
            _ => {
                self.from = None;
                self.pose.rot.clone_from(&self.scratch.rot);
                self.pose.pos.clone_from(&self.scratch.pos);
            }
        }
        globals(&self.faith.mesh, &self.pose, &mut self.faith_globals);
        let mut out = PoseOut::default();
        self.retarget.pose(&self.gta, &self.faith_globals, false, &mut out);
        out
    }

    pub fn bones(&self) -> usize {
        self.gta.names.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faith_move::{Input, MeshWorld, Tuning};
    use glam::Vec2;

    /// Run, jump at a wall, climb: Faith's 3P animations drive Trevor's skeleton without
    /// NaNs, and the clips change with the moves.
    #[test]
    fn drives_trevor_through_moves() {
        let (Some(dir), Some(gta)) = (std::env::var_os("ME_INSTALL"), crate::retarget::tests::trevor()) else { return };
        let faith = Arc::new(Faith3p::load(Path::new(&dir)).expect("load"));
        let mut d = Driver::new(faith, gta.clone()).expect("driver");
        let mut tris = MeshWorld::oriented_box(Vec3::new(0., -0.5, 0.), Vec3::new(40., 0.5, 40.), 0.0);
        tris.extend(MeshWorld::oriented_box(Vec3::new(0., 5.0, -10.0), Vec3::new(10., 5.0, 1.0), 0.0));
        let world = MeshWorld::new(tris, vec![]);
        let mut c = Controller::new(Tuning::default(), Vec3::new(0., 0.05, 0.), 0.0);
        let mut clips: Vec<String> = vec![];
        let mut jumped = false;
        for _ in 0..240 {
            let jump = !jumped && c.feet.z < -7.6;
            jumped |= jump;
            c.step(1.0 / 60.0, &Input { move_axis: Vec2::Y, jump_pressed: jump, jump_held: jumped, ..Default::default() }, &world);
            let m = d.update(1.0 / 60.0, &c);
            assert_eq!(m.objects.len(), gta.names.len());
            assert!(m.objects.iter().chain(&m.locals).all(|x| x.is_finite()));
            let now = d.current.as_ref().unwrap().0.clone();
            if clips.last() != Some(&now) {
                clips.push(now);
            }
        }
        println!("clips: {clips:?}");
        assert!(clips.iter().any(|c| c.eq_ignore_ascii_case("WallRunVertical")), "{clips:?}");
    }
}
