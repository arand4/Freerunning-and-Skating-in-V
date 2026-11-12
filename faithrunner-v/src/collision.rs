//! GTA's static map around the player, read on a background thread from GTA's
//! loaded collision (SK8V's live reader) and turned into Faith's triangle world.
//!
//! Faith's space: metres, Y up, relative to `origin` (a GTA point picked when
//! Faith mode starts, so the numbers stay small): faith = (x, z, -y) - origin.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use faith_move::MeshWorld;
use glam::Vec3;

use crate::{live, log};

pub fn to_faith(g: Vec3, origin: Vec3) -> Vec3 {
    let d = g - origin;
    Vec3::new(d.x, d.z, -d.y)
}

pub fn to_gta(f: Vec3, origin: Vec3) -> Vec3 {
    origin + Vec3::new(f.x, -f.z, f.y)
}

pub fn dir_to_gta(f: Vec3) -> Vec3 {
    Vec3::new(f.x, -f.z, f.y)
}

pub fn dir_to_faith(g: Vec3) -> Vec3 {
    Vec3::new(g.x, g.z, -g.y)
}

#[derive(Default)]
pub struct Shared {
    /// Where GTA's physics level is, once found.
    pub level: Option<live::Level>,
    /// Faith mode wants collision (else the builder idles).
    pub active: bool,
    pub origin: Vec3,
    /// The player's GTA position, updated every frame.
    pub player: Vec3,
    /// The latest built world and the origin it was built for.
    pub world: Option<(Arc<MeshWorld>, Vec3)>,
    /// GTA's material of each of the world's triangles (same order), for finding ladders.
    pub materials: Option<Arc<Vec<u8>>>,
    /// Consecutive failed reads (the table moved or is gone).
    pub failures: u32,
    /// Cars' and props' shapes (see dynamic.rs).
    pub models: crate::dynamic::Models,
    /// GTA's ladders near the player (GTA space), from the ladder watcher (ladders.rs).
    pub ladders: Arc<Vec<crate::ladders::GtaLadder>>,
}

pub type Handle = Arc<Mutex<Shared>>;

pub fn start(radius: f32, rebuild_distance: f32) -> Handle {
    let h: Handle = Arc::default();
    let shared = h.clone();
    std::thread::Builder::new()
        .name("faithrunner-collision".into())
        .spawn(move || builder(shared, radius, rebuild_distance))
        .expect("collision thread");
    h
}

fn builder(shared: Handle, radius: f32, rebuild_distance: f32) {
    let mut built: Option<(Vec3, Vec3, Instant)> = None; // centre, origin, when
    let mut logged = 0;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let (level, active, origin, player) = {
            let s = shared.lock().unwrap();
            (s.level, s.active, s.origin, s.player)
        };
        let (Some(level), true) = (level, active) else {
            built = None;
            continue;
        };
        // Cars' and props' shapes asked for by the script thread.
        let mut models = std::mem::take(&mut shared.lock().unwrap().models);
        if !models.wanted.is_empty() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| models.resolve(&level)));
        }
        {
            let mut s = shared.lock().unwrap();
            // Keep wants the script thread added meanwhile.
            let added = std::mem::take(&mut s.models.wanted);
            models.wanted.extend(added.into_iter().filter(|(m, _)| !models.shapes.contains_key(m)));
            s.models = models;
        }
        let due = match built {
            None => true,
            Some((c, o, at)) => {
                o != origin || (player - c).truncate().length() > rebuild_distance || at.elapsed() > Duration::from_secs(4)
            }
        };
        if !due {
            continue;
        }
        let t0 = Instant::now();
        let result = std::panic::catch_unwind(|| level.walk([player.x, player.y], radius));
        match result {
            Ok(Ok((tris, census, _))) => {
                let walked = t0.elapsed();
                let faith: Vec<[Vec3; 3]> = tris
                    .iter()
                    .map(|t| t.v.map(|p| to_faith(Vec3::from_array(p), origin)))
                    .collect();
                let n = faith.len();
                let materials: Arc<Vec<u8>> = Arc::new(tris.iter().map(|t| t.material).collect());
                let mut mesh = MeshWorld::new(faith, Vec::new());
                // GTA's ladders become Faith's (Mirror's Edge's ladder climb).
                let gta_ladders = shared.lock().unwrap().ladders.clone();
                mesh.fixtures = gta_ladders
                    .iter()
                    .filter(|l| (l.bottom - player).truncate().length() < radius + 10.0)
                    .map(|l| faith_move::Fixture::Ladder(crate::ladders::fit(l, origin, &mesh)))
                    .collect();
                let ladder_count = mesh.fixtures.len();
                if ladder_count > 0 {
                    // The ladders' own small parts out of the way (see ladders::in_climb_space).
                    let ladders: Vec<faith_move::climb::Ladder> = mesh
                        .fixtures
                        .iter()
                        .filter_map(|x| match x {
                            faith_move::Fixture::Ladder(l) => Some(*l),
                            _ => None,
                        })
                        .collect();
                    let fixtures = std::mem::take(&mut mesh.fixtures);
                    let tris: Vec<[Vec3; 3]> = std::mem::take(&mut mesh.tris)
                        .into_iter()
                        .filter(|t| {
                            let small = (t[0] - t[1]).length() < 1.5 && (t[1] - t[2]).length() < 1.5 && (t[2] - t[0]).length() < 1.5;
                            !(small && ladders.iter().any(|l| t.iter().all(|&p| crate::ladders::in_climb_space(l, p))))
                        })
                        .collect();
                    mesh = MeshWorld::new(tris, fixtures);
                }
                let world = Arc::new(mesh);
                if logged < 5 || logged % 50 == 0 {
                    log!(
                        "collision: {n} triangles and {ladder_count} ladders around ({:.0}, {:.0}, {:.0}), read {:.0} ms, built {:.0} ms; {census}",
                        player.x,
                        player.y,
                        player.z,
                        walked.as_secs_f64() * 1e3,
                        (t0.elapsed() - walked).as_secs_f64() * 1e3
                    );
                }
                logged += 1;
                let mut s = shared.lock().unwrap();
                s.world = Some((world, origin));
                s.materials = Some(materials);
                s.failures = 0;
                built = Some((player, origin, Instant::now()));
            }
            Ok(Err(e)) => {
                log!("collision: read failed: {e}");
                let mut s = shared.lock().unwrap();
                s.failures += 1;
                if s.failures >= 3 {
                    log!("collision: the physics level is gone; locating it again");
                    s.level = None;
                    s.failures = 0;
                }
                drop(s);
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(_) => {
                log!("collision: the reader panicked; trying again shortly");
                std::thread::sleep(Duration::from_millis(1000));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faith_move::{Controller, Input, State, Tuning};
    use glam::Vec2;

    /// A GTA-space quad (z up) as two triangles in Faith's space.
    fn quad(o: Vec3, a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Vec<[Vec3; 3]> {
        let f = |p: Vec3| to_faith(p, o);
        vec![[f(a), f(b), f(c)], [f(a), f(c), f(d)]]
    }

    /// Far from GTA's origin (as in Los Santos), floor at z = 30, a 10 m wall 8 m north.
    fn scene() -> (Vec3, MeshWorld) {
        let o = Vec3::new(-1200.0, -1500.0, 31.0);
        let g = |x: f32, y: f32, z: f32| Vec3::new(-1200.0 + x, -1500.0 + y, z);
        let mut t = quad(o, g(-20., -20., 30.), g(20., -20., 30.), g(20., 20., 30.), g(-20., 20., 30.));
        // A solid building: x -20..20, y 8..18, z 30..40 (four walls and a roof).
        let (x0, x1, y0, y1, z0, z1) = (-20., 20., 8., 18., 30., 40.);
        t.extend(quad(o, g(x0, y0, z0), g(x1, y0, z0), g(x1, y0, z1), g(x0, y0, z1)));
        t.extend(quad(o, g(x0, y1, z0), g(x1, y1, z0), g(x1, y1, z1), g(x0, y1, z1)));
        t.extend(quad(o, g(x0, y0, z0), g(x0, y1, z0), g(x0, y1, z1), g(x0, y0, z1)));
        t.extend(quad(o, g(x1, y0, z0), g(x1, y1, z0), g(x1, y1, z1), g(x1, y0, z1)));
        t.extend(quad(o, g(x0, y0, z1), g(x1, y0, z1), g(x1, y1, z1), g(x0, y1, z1)));
        (o, MeshWorld::new(t, vec![]))
    }

    #[test]
    fn axes_round_trip() {
        let o = Vec3::new(10., 20., 30.);
        let p = Vec3::new(13., 25., 37.);
        assert!((to_gta(to_faith(p, o), o) - p).length() < 1e-5);
        assert_eq!(dir_to_faith(Vec3::Z), Vec3::Y, "GTA up is Faith up");
        assert_eq!(dir_to_faith(Vec3::Y), -Vec3::Z, "GTA north is Faith's yaw-0 forward");
    }

    #[test]
    fn stands_and_runs_north_into_the_wall() {
        let (o, world) = scene();
        let mut c = Controller::new(Tuning::default(), to_faith(Vec3::new(-1200., -1500., 30.05), o), 0.0);
        for _ in 0..60 {
            c.step(1.0 / 60.0, &Input::default(), &world);
        }
        let feet = to_gta(c.feet, o);
        assert!(matches!(c.state, State::Ground), "{:?}", c.state.name());
        assert!((feet.z - 30.0).abs() < 0.05, "feet at z {}", feet.z);
        // Heading 0 (GTA north): run forward until the wall stops her short of y = 8.
        let run = Input { move_axis: Vec2::Y, ..Default::default() };
        let mut max_y = f32::MIN;
        let mut states = vec![];
        for _ in 0..240 {
            c.step(1.0 / 60.0, &run, &world);
            let g = to_gta(c.feet, o);
            max_y = max_y.max(g.y + 1500.0);
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
            }
        }
        let g = to_gta(c.feet, o);
        assert!(max_y > 6.5 && max_y < 8.0, "ran north to y {max_y}");
        assert!((g.x + 1200.0).abs() < 0.2, "stayed on line: x {}", g.x + 1200.0);
        println!("states: {states:?}");
    }

    #[test]
    fn jump_at_wall_climbs_it() {
        let (o, world) = scene();
        let mut c = Controller::new(Tuning::default(), to_faith(Vec3::new(-1200., -1500., 30.05), o), 0.0);
        let mut states = vec![];
        let mut top = f32::MIN;
        let mut jumped: Option<usize> = None;
        for i in 0..360 {
            // Jump once within 1.2 m of the wall (y = 8), and hold it.
            if jumped.is_none() && to_gta(c.feet, o).y + 1500.0 > 6.8 {
                jumped = Some(i);
            }
            let input = Input {
                move_axis: Vec2::Y,
                jump_pressed: jumped == Some(i),
                jump_held: jumped.is_some_and(|j| i < j + 60),
                ..Default::default()
            };
            c.step(1.0 / 60.0, &input, &world);
            top = top.max(to_gta(c.feet, o).z);
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
            }
        }
        println!("states: {states:?}, highest feet z {top}, end {:?}", to_gta(c.feet, o) - Vec3::new(-1200., -1500., 0.));
        assert!(states.iter().any(|s| s.starts_with("Wallclimb")), "{states:?}");
    }

    /// The building with a window sill sticking out 20 cm from its south face (y = 8),
    /// its underside 2.3 m above the street.
    fn scene_with_sill() -> (Vec3, MeshWorld) {
        let o = Vec3::new(-1200.0, -1500.0, 31.0);
        let g = |x: f32, y: f32, z: f32| Vec3::new(-1200.0 + x, -1500.0 + y, z);
        let mut t = quad(o, g(-20., -20., 30.), g(20., -20., 30.), g(20., 20., 30.), g(-20., 20., 30.));
        let mut solid = |x0: f32, x1: f32, y0: f32, y1: f32, z0: f32, z1: f32| {
            t.extend(quad(o, g(x0, y0, z0), g(x1, y0, z0), g(x1, y0, z1), g(x0, y0, z1)));
            t.extend(quad(o, g(x0, y1, z0), g(x1, y1, z0), g(x1, y1, z1), g(x0, y1, z1)));
            t.extend(quad(o, g(x0, y0, z0), g(x0, y1, z0), g(x0, y1, z1), g(x0, y0, z1)));
            t.extend(quad(o, g(x1, y0, z0), g(x1, y1, z0), g(x1, y1, z1), g(x1, y0, z1)));
            t.extend(quad(o, g(x0, y0, z1), g(x1, y0, z1), g(x1, y1, z1), g(x0, y1, z1)));
            t.extend(quad(o, g(x0, y0, z0), g(x1, y0, z0), g(x1, y1, z0), g(x0, y1, z0)));
        };
        solid(-20., 20., 8., 18., 30., 40.);
        solid(-20., 20., 7.8, 8.0, 32.3, 32.45);
        (o, MeshWorld::new(t, vec![]))
    }

    /// Run at the wall, jump close to it, keep holding forward and jump: (states, highest
    /// feet, whether she ever got inside the building).
    fn climb_at_sill(step_out: f32) -> (Vec<&'static str>, f32, bool) {
        let (o, world) = scene_with_sill();
        let mut tuning = Tuning::default();
        tuning.overhang_step_out = step_out;
        let mut c = Controller::new(tuning, to_faith(Vec3::new(-1200., -1500., 30.05), o), 0.0);
        let (mut states, mut top, mut inside) = (vec![], f32::MIN, false);
        let mut jumped: Option<usize> = None;
        for i in 0..600 {
            let g = to_gta(c.feet, o);
            if jumped.is_none() && g.y + 1500.0 > 6.6 {
                jumped = Some(i);
            }
            let input = Input {
                move_axis: Vec2::Y,
                jump_pressed: jumped == Some(i) || jumped.is_some_and(|j| i == j + 50),
                jump_held: jumped.is_some(),
                ..Default::default()
            };
            let before = (c.feet, c.state.name());
            c.step(1.0 / 60.0, &input, &world);
            assert!(!crate::mode::through_wall_guard(&mut c, before, &world), "guard stopped a real move");
            let g = to_gta(c.feet, o);
            top = top.max(g.z);
            inside |= g.y + 1500.0 > 8.05 && g.z < 39.9;
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
                if std::env::var_os("TRACE").is_some() {
                    println!("  {} at y {:.2} z {:.2}", c.state.name(), g.y + 1500.0, g.z);
                }
            }
        }
        (states, top, inside)
    }

    #[test]
    fn wallclimb_round_a_sill_never_inside() {
        let (plain, top_plain, in_plain) = climb_at_sill(0.0);
        println!("as in ME: {plain:?}, feet up to z {top_plain:.2}, inside {in_plain}");
        let (gta, top_gta, in_gta) = climb_at_sill(0.45);
        println!("stepping out: {gta:?}, feet up to z {top_gta:.2}, inside {in_gta}");
        assert!(!in_plain && !in_gta, "never inside the building");
        assert!(top_gta > top_plain + 0.3, "climbs past the sill");
    }

    /// A 50 cm step 60 cm deep at the foot of a tall wall: jumping at it from a run, with the
    /// GTA settings she plants a foot on the step and wallclimbs from there, higher than from
    /// the floor.
    fn springboard_at_wall(into_walls: bool) -> (Vec<&'static str>, f32) {
        let o = Vec3::new(0.0, 0.0, 30.0);
        let g = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let mut t = quad(o, g(-20., -20., 30.), g(20., -20., 30.), g(20., 20., 30.), g(-20., 20., 30.));
        let mut solid = |x0: f32, x1: f32, y0: f32, y1: f32, z0: f32, z1: f32| {
            t.extend(quad(o, g(x0, y0, z0), g(x1, y0, z0), g(x1, y0, z1), g(x0, y0, z1)));
            t.extend(quad(o, g(x0, y1, z0), g(x1, y1, z0), g(x1, y1, z1), g(x0, y1, z1)));
            t.extend(quad(o, g(x0, y0, z0), g(x0, y1, z0), g(x0, y1, z1), g(x0, y0, z1)));
            t.extend(quad(o, g(x1, y0, z0), g(x1, y1, z0), g(x1, y1, z1), g(x1, y0, z1)));
            t.extend(quad(o, g(x0, y0, z1), g(x1, y0, z1), g(x1, y1, z1), g(x0, y1, z1)));
        };
        solid(-20., 20., 10.0, 18., 30., 40.); // the wall, south face at y = 10
        solid(-20., 20., 9.4, 10.0, 30., 30.5); // the step in front of it
        let world = MeshWorld::new(t, vec![]);
        let mut tu = Tuning::default();
        tu.springboard_into_walls = into_walls;
        tu.springboard_slack = 0.3;
        let mut c = Controller::new(tu, to_faith(Vec3::new(0., 0., 30.05), o), 0.0);
        let (mut states, mut top) = (vec![], f32::MIN);
        let mut jumped = false;
        for _ in 0..500 {
            let y = to_gta(c.feet, o).y;
            let jump = !jumped && y > 7.0;
            jumped |= jump;
            let before = (c.feet, c.state.name());
            c.step(1.0 / 60.0, &Input { move_axis: Vec2::Y, jump_pressed: jump, jump_held: jumped, ..Default::default() }, &world);
            assert!(!crate::mode::through_wall_guard(&mut c, before, &world), "guard stopped a real move");
            top = top.max(to_gta(c.feet, o).z);
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
            }
        }
        (states, top - 30.0)
    }

    #[test]
    fn springboard_into_a_wall() {
        let (me, top_me) = springboard_at_wall(false);
        println!("ME rules: {me:?}, feet up to {top_me:.2} m");
        let (gta, top_gta) = springboard_at_wall(true);
        println!("GTA rules: {gta:?}, feet up to {top_gta:.2} m");
        assert!(gta.contains(&"Springboard") && gta.contains(&"Wallclimb"), "{gta:?}");
        assert!(top_gta > top_me + 0.3, "higher than from the floor");
    }

    /// A 40 cm block 1 m deep across her path: with GTA's step settings she runs up and over it.
    #[test]
    fn runs_over_a_low_block() {
        let o = Vec3::new(0.0, 0.0, 30.0);
        let g = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let mut t = quad(o, g(-20., -20., 30.), g(20., -20., 30.), g(20., 20., 30.), g(-20., 20., 30.));
        let (x0, x1, y0, y1, z0, z1) = (-5., 5., 6.0, 7.0, 30.0, 30.4);
        t.extend(quad(o, g(x0, y0, z0), g(x1, y0, z0), g(x1, y0, z1), g(x0, y0, z1)));
        t.extend(quad(o, g(x0, y1, z0), g(x1, y1, z0), g(x1, y1, z1), g(x0, y1, z1)));
        t.extend(quad(o, g(x0, y0, z1), g(x1, y0, z1), g(x1, y1, z1), g(x0, y1, z1)));
        let world = MeshWorld::new(t, vec![]);
        let mut tu = Tuning::default();
        tu.step_height = 0.45;
        tu.auto_step_up_max = 0.8;
        let mut c = Controller::new(tu, to_faith(Vec3::new(0., 0., 30.05), o), 0.0);
        let mut states = vec![];
        for _ in 0..240 {
            let before = (c.feet, c.state.name());
            c.step(1.0 / 60.0, &Input { move_axis: Vec2::Y, ..Default::default() }, &world);
            assert!(!crate::mode::through_wall_guard(&mut c, before, &world), "guard stopped a real move");
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
            }
        }
        let y = to_gta(c.feet, o).y;
        println!("states {states:?}, ends at y {y:.1}, speed {:.1}", c.horizontal_speed());
        assert!(y > 9.0, "got past the block");
        assert!(!states.contains(&"Vault"));
    }
}
