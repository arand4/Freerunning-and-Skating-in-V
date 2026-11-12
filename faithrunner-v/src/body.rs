//! Faith's first-person body (arms and legs) from the player's own Mirror's Edge
//! install, animated by Faith Runner's `faith_anim` (the game's own animation
//! choices, body placement and camera), skinned on the CPU each frame and
//! handed to the renderer. Mirror's Edge's first person has only arms and legs.
//!
//! Later this is where the active GTA character's body would go instead
//! (its head and torso hidden): `Body = gta` in the settings, not built yet.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use faith_anim::{Rig, RigFrame};
use faith_move::{Controller, Shot};
use glam::{Mat4, Quat, Vec3};
use me_assets::pose::section_indices;
use me_assets::{FaithArms, Skinner};

use crate::log;
use crate::render::{self, PartFrame, PartStatic, Scene, Section, Vertex};

/// TdPawn.Model1pFOV: Mirror's Edge draws the first-person body at 100° (the world at 90°).
const MODEL_1P_FOV_DEG: f32 = 100.0;

/// The Mirror's Edge files: the ini's path (relative paths are from the GTA V folder), the copy
/// the launcher makes in FaithRunnerV\MirrorsEdge, FAITH_ME_DIR, then common install folders.
pub fn find_install(configured: &str) -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = vec![];
    if !configured.trim().is_empty() {
        c.push(crate::module_dir().join(configured.trim()));
    }
    c.push(crate::module_dir().join("FaithRunnerV").join("MirrorsEdge"));
    if let Some(p) = std::env::var_os("FAITH_ME_DIR") {
        c.push(PathBuf::from(p));
    }
    for p in [
        r"C:\Program Files (x86)\Steam\steamapps\common\mirrors edge",
        r"C:\Program Files\EA Games\Mirror's Edge",
        r"C:\Program Files (x86)\EA Games\Mirror's Edge",
        r"C:\Program Files (x86)\Origin Games\Mirror's Edge",
        r"C:\Games\Mirror's Edge",
    ] {
        c.push(PathBuf::from(p));
    }
    c.into_iter().find(|d| me_assets::cooked_pc(d).is_some())
}

/// Loaded once, in the background (reading the packages takes a moment).
pub static LOADED: Mutex<Option<Result<Arc<Loaded>, String>>> = Mutex::new(None);

pub struct Loaded {
    pub arms: FaithArms,
    pub scene: Arc<Scene>,
}

pub fn load_in_background(dir: PathBuf) {
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let r = std::panic::catch_unwind(|| load(&dir)).unwrap_or_else(|_| Err("the reader panicked".into()));
        match &r {
            Ok(l) => log!(
                "body: Faith's arms{} loaded from {} in {:.1} s",
                if l.arms.legs.is_some() { " and legs" } else { "" },
                dir.display(),
                t0.elapsed().as_secs_f32()
            ),
            Err(e) => log!("body: couldn't load Faith's body from {}: {e}", dir.display()),
        }
        *LOADED.lock().unwrap() = Some(r.map(Arc::new));
    });
}

fn load(dir: &Path) -> Result<Loaded, String> {
    let arms = FaithArms::load(dir, 1024).map_err(|e| e.to_string())?;
    let mut parts = vec![part_static(&arms.mesh, &arms.materials, false)];
    if let Some((legs, slots)) = &arms.legs {
        parts.push(part_static(legs, slots, true));
    }
    Ok(Loaded { arms, scene: Arc::new(Scene { parts }) })
}

fn part_static(mesh: &me_assets::SkeletalMesh, slots: &[me_assets::MaterialSlot], legs: bool) -> PartStatic {
    let uvs = mesh.vertices.iter().map(|v| v.uv).collect();
    let sections = mesh
        .sections
        .iter()
        .enumerate()
        .map(|(k, sec)| Section {
            indices: section_indices(mesh, k),
            texture: slots.get(sec.material as usize).and_then(|s| s.diffuse.clone()),
        })
        .collect();
    PartStatic { uvs, sections, legs }
}

/// One Faith mode session's body: the rig and the skinning scratch.
pub struct Body {
    pub loaded: Arc<Loaded>,
    pub rig: Rig,
    skinners: Vec<Skinner>,
    morph: Vec<Vec3>,
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
}

impl Body {
    /// This frame's posed bones of Faith's first-person skeleton (Mirror's Edge mesh space).
    pub fn globals(&self) -> &[Mat4] {
        &self.rig.driver.globals
    }

    /// Her first-person skeleton at rest (for fitting a GTA skeleton to it).
    pub fn rest_globals(&self) -> Vec<Mat4> {
        let mut g = vec![];
        me_assets::pose::globals(&self.loaded.arms.mesh, &me_assets::pose::Pose::rest(&self.loaded.arms.mesh), &mut g);
        g
    }

    pub fn new(loaded: Arc<Loaded>, ctrl: &mut Controller) -> Body {
        let rig = Rig::new(&loaded.arms, ctrl.yaw);
        // Let the move timings follow the animations they play (as Faith Runner does).
        rig.tune(&loaded.arms, &mut ctrl.tuning);
        let mut skinners = vec![Skinner::new(&loaded.arms.mesh)];
        if let Some((legs, _)) = &loaded.arms.legs {
            skinners.push(Skinner::new(legs));
        }
        Body { loaded, rig, skinners, morph: vec![], pos: vec![], nrm: vec![] }
    }

    /// Animate, place and skin the body; publish it for drawing from the camera
    /// `frame.cam_pos` / `cam_rot` (Faith's space). `world_hfov` is the world's
    /// horizontal FOV, `aspect` the screen's.
    /// `draw`: publish Faith's arms and legs for drawing (false: the GTA character's own body
    /// wears the pose instead, see `globals`).
    pub fn update(&mut self, dt: f32, ctrl: &Controller, shot: &Shot, world_hfov: f32, aspect: f32, draw: bool) -> RigFrame {
        let arms = &self.loaded.arms;
        let frame = self.rig.update(dt, ctrl, shot, arms);
        if !draw {
            return frame;
        }
        let view = Mat4::look_to_rh(Vec3::ZERO, frame.cam_rot * Vec3::NEG_Z, frame.cam_rot * Vec3::Y);
        // Looking down, the arms' FOV blends to the world's so torso and legs meet (as in Faith Runner).
        let t = ((-ctrl.pitch - 0.35) / 0.55).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let arms_h = MODEL_1P_FOV_DEG.to_radians() * (1.0 - t) + world_hfov * t;
        let vfov = |h: f32| 2.0 * ((h * 0.5).tan() / aspect).atan();
        let mut parts = Vec::with_capacity(self.skinners.len());
        for (i, skinner) in self.skinners.iter().enumerate() {
            let legs = i == 1;
            let mesh = if legs { &arms.legs.as_ref().unwrap().0 } else { &arms.mesh };
            if legs || arms.morphs.is_empty() {
                skinner.skin_full(mesh, &self.rig.driver.globals, Mat4::IDENTITY, &mut self.pos, &mut self.nrm, None);
            } else {
                self.rig.driver.forearm_morphs(arms, &mut self.morph);
                skinner.skin_morphed(mesh, &self.rig.driver.globals, Mat4::IDENTITY, &self.morph, &mut self.pos, &mut self.nrm, None);
            }
            let rot: Quat = if legs { frame.legs_rot } else { frame.body_rot };
            let uvs = &self.loaded.scene.parts[i].uvs;
            let verts = self
                .pos
                .iter()
                .zip(&self.nrm)
                .zip(uvs)
                .map(|((p, n), uv)| Vertex {
                    // Relative to the camera, so the numbers stay small.
                    p: (frame.origin + rot * Vec3::from_array(*p) - frame.cam_pos).to_array(),
                    n: (rot * Vec3::from_array(*n)).to_array(),
                    uv: *uv,
                })
                .collect();
            let h = if legs { world_hfov } else { arms_h };
            let proj = Mat4::perspective_rh(vfov(h), aspect, 0.01, 50.0);
            parts.push(PartFrame { verts, view_proj: proj * view });
        }
        render::publish(Some((self.loaded.scene.clone(), parts)));
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faith_move::{CameraFx, CameraFxSettings, Input, MeshWorld, Tuning};
    use glam::Vec2;

    /// Needs ME_INSTALL (the Mirror's Edge folder) and writes pictures to FAITH_SHOTS (a folder).
    #[test]
    fn faith_body_draws() {
        let Some(dir) = std::env::var_os("ME_INSTALL") else { return };
        let shots = std::path::PathBuf::from(std::env::var_os("FAITH_SHOTS").expect("FAITH_SHOTS"));
        let loaded = Arc::new(load(Path::new(&dir)).expect("load"));
        println!("parts: {}, vertices {:?}", loaded.scene.parts.len(), loaded.scene.parts.iter().map(|p| p.uvs.len()).collect::<Vec<_>>());
        let floor = MeshWorld::new(MeshWorld::oriented_box(Vec3::new(0., -0.5, 0.), Vec3::new(50., 0.5, 50.), 0.0), vec![]);
        let mut ctrl = Controller::new(Tuning::default(), Vec3::new(0., 0.05, 0.), 0.0);
        let mut body = Body::new(loaded.clone(), &mut ctrl);
        let mut fx = CameraFx::new(CameraFxSettings::animation_driven());
        let aspect = 16.0 / 9.0;
        let shoot = |name: &str, frames: &[PartFrame]| {
            let (changed, total) = render::tests::render_to_bmp(&loaded.scene, frames, &shots.join(name));
            println!("{name}: {changed} of {total} pixels show the body");
            changed
        };
        let mut drawn = vec![];
        for (frames_to_run, pitch, name) in [(30, 0.0, "stand.bmp"), (90, 0.0, "run.bmp"), (10, -1.2, "look_down.bmp")] {
            for _ in 0..frames_to_run {
                let input = Input { move_axis: if name == "stand.bmp" { Vec2::ZERO } else { Vec2::Y }, ..Default::default() };
                ctrl.step(1.0 / 60.0, &input, &floor);
                ctrl.pitch = pitch;
                let shot = fx.update(1.0 / 60.0, &ctrl, &input);
                body.update(1.0 / 60.0, &ctrl, &shot, 90f32.to_radians(), aspect, true);
            }
            let cur = crate::render::tests_take();
            assert!(cur.iter().all(|f| f.verts.iter().all(|v| v.p.iter().all(|x| x.is_finite() && x.abs() < 3.0))), "body within 3 m of the eye");
            drawn.push(shoot(name, &cur));
        }
        // Standing still her arms hang out of view, as in Mirror's Edge; running and looking down show them.
        assert!(drawn[1..].iter().all(|&n| n > 1000), "the body shows: {drawn:?}");
    }
}
