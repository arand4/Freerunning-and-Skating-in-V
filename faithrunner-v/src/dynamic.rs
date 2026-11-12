//! Cars and props (things that can move) as a second collision layer, rebuilt
//! every frame around Faith. Each model's shape is read once from GTA's physics
//! level (SK8V's `Level::model_at`, on the collision thread). Only what blocks a
//! person counts (the same filter as the map): signs, neon and other decoration
//! have no collision for peds, so none here either (a stand-in box there made
//! invisible walls on the strip club roof). Cars use their box until their shape reads.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use faith_move::MeshWorld;
use glam::Vec3;

use crate::collision::{Handle, to_faith};
use crate::natives::*;
use crate::shv::{ret_bool, ret_i32, ret_vec3};
use crate::{live, log, native, shv};

/// A model's collision, in its own space (GTA axes: x right, y forward, z up).
#[derive(Clone)]
pub enum Shape {
    Tris(Arc<Vec<[Vec3; 3]>>),
    /// A car whose shape couldn't be read: its box is used.
    Box,
    /// Nothing a person bumps into.
    Nothing,
}

/// Reads from the collision thread: models wanted, and what was found.
#[derive(Default)]
pub struct Models {
    /// Model -> where one stands now (position, scaled axes) and whether it's a car, for `model_at`.
    pub wanted: HashMap<u32, (Vec3, [Vec3; 3], bool)>,
    pub shapes: HashMap<u32, Shape>,
    tries: HashMap<u32, (u8, Instant)>,
}

impl Models {
    /// Collision thread: read a few wanted models (each `model_at` walks the whole table).
    pub fn resolve(&mut self, level: &live::Level) {
        let due: Vec<u32> = self
            .wanted
            .keys()
            .copied()
            .filter(|m| self.tries.get(m).is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(2)))
            .take(4)
            .collect();
        for model in due {
            let Some((pos, axes, car)) = self.wanted.get(&model).copied() else { continue };
            match level.model_at(pos, axes) {
                Ok(tris) => {
                    let tris: Vec<[Vec3; 3]> = tris.iter().map(|t| t.v.map(Vec3::from_array)).collect();
                    log!("dynamic: model {model:08x}: {} triangles read from GTA", tris.len());
                    self.shapes.insert(model, Shape::Tris(Arc::new(tris)));
                    self.wanted.remove(&model);
                    self.tries.remove(&model);
                }
                Err(e) if e.contains("no skater collision") => {
                    // Its collision doesn't block people (decoration, signs): walk through it.
                    log!("dynamic: model {model:08x}: nothing that blocks a person ({e})");
                    self.shapes.insert(model, Shape::Nothing);
                    self.wanted.remove(&model);
                    self.tries.remove(&model);
                }
                Err(e) => {
                    let n = self.tries.get(&model).map_or(1, |(n, _)| n + 1);
                    if n >= 10 {
                        let shape = if car { Shape::Box } else { Shape::Nothing };
                        log!("dynamic: model {model:08x}: not readable ({e}); {}", if car { "using its box" } else { "no collision" });
                        self.shapes.insert(model, shape);
                        self.wanted.remove(&model);
                        self.tries.remove(&model);
                    } else {
                        self.tries.insert(model, (n, Instant::now()));
                    }
                }
            }
        }
    }
}

struct Placed {
    model: u32,
    car: bool,
    pos: Vec3,
    /// right, forward, up (scaled like the entity).
    axes: [Vec3; 3],
    lo: Vec3,
    hi: Vec3,
}

impl Placed {
    fn at(&self, p: Vec3) -> Vec3 {
        self.pos + self.axes[0] * p.x + self.axes[1] * p.y + self.axes[2] * p.z
    }

    /// The model's box as 12 triangles (GTA space).
    fn box_tris(&self) -> Vec<[Vec3; 3]> {
        let (l, h) = (self.lo, self.hi);
        let v: Vec<Vec3> = (0..8)
            .map(|i| self.at(Vec3::new(if i & 1 == 0 { l.x } else { h.x }, if i & 2 == 0 { l.y } else { h.y }, if i & 4 == 0 { l.z } else { h.z })))
            .collect();
        let faces = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
        faces.iter().flat_map(|f| [[v[f[0]], v[f[1]], v[f[2]]], [v[f[0]], v[f[2]], v[f[3]]]]).collect()
    }
}

/// Per-model dimensions, asked once.
#[derive(Default)]
pub struct Dimensions(HashMap<u32, (Vec3, Vec3)>);

/// Script thread: the cars and props within `reach` of `centre` (GTA space) as Faith's
/// moving layer. Unknown models are asked for on the collision thread.
pub fn moving_world(centre: Vec3, reach: f32, origin: Vec3, shared: &Handle, dims: &mut Dimensions) -> (MeshWorld, Vec<CarBox>, usize) {
    let s = shv::shv();
    let mut placed = Vec::new();
    let mut h = vec![0i32; 1024];
    for (get, is_object) in [(s.all_vehicles, false), (s.all_objects, true)] {
        let n = unsafe { get(h.as_mut_ptr(), h.len() as i32) }.clamp(0, 1024) as usize;
        for &e in &h[..n] {
            if placed.len() >= 96 {
                break;
            }
            let p = ret_vec3(native!(GET_ENTITY_COORDS, e, true));
            if (p - centre).length() > reach || !ret_bool(native!(IS_ENTITY_VISIBLE, e)) {
                continue;
            }
            // Props held or attached (weapons, carried things) are not in the way.
            if is_object && ret_bool(native!(IS_ENTITY_ATTACHED, e)) {
                continue;
            }
            if ret_bool(native!(GET_ENTITY_COLLISION_DISABLED, e)) {
                continue;
            }
            let model = ret_i32(native!(GET_ENTITY_MODEL, e)) as u32;
            let (lo, hi) = *dims.0.entry(model).or_insert_with(|| {
                let (mut lo, mut hi) = ([0u64; 3], [0u64; 3]);
                native!(GET_MODEL_DIMENSIONS, model, lo.as_mut_ptr() as *const i8, hi.as_mut_ptr() as *const i8);
                (shv::ret_vec3(lo.as_mut_ptr()), shv::ret_vec3(hi.as_mut_ptr()))
            });
            // Cans, bottles and other small clutter: run straight over them.
            let size = hi - lo;
            if is_object && size.z < 0.3 && size.x.max(size.y) < 0.8 {
                continue;
            }
            let mut m = [[0u64; 3]; 4]; // forward, right, up, position
            native!(
                GET_ENTITY_MATRIX,
                e,
                m[0].as_mut_ptr() as *const i8,
                m[1].as_mut_ptr() as *const i8,
                m[2].as_mut_ptr() as *const i8,
                m[3].as_mut_ptr() as *const i8
            );
            let [fwd, right, up, pos] = m.map(|mut v| shv::ret_vec3(v.as_mut_ptr()));
            if !(pos.is_finite() && fwd.is_finite() && right.is_finite() && up.is_finite()) {
                continue;
            }
            placed.push(Placed { model, car: !is_object, pos, axes: [right, fwd, up], lo, hi });
        }
    }
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    {
        let mut sh = shared.lock().unwrap();
        for b in &placed {
            match sh.models.shapes.get(&b.model) {
                Some(Shape::Tris(t)) => tris.extend(t.iter().map(|t| t.map(|p| b.at(p)))),
                Some(Shape::Box) => tris.extend(b.box_tris()),
                Some(Shape::Nothing) => {}
                None => {
                    sh.models.wanted.insert(b.model, (b.pos, b.axes, b.car));
                    // Until it reads: a car's box; a prop nothing (it may be decoration).
                    if b.car {
                        tris.extend(b.box_tris());
                    }
                }
            }
        }
    }
    let count = placed.len();
    let faith = tris.into_iter().map(|t| t.map(|p| to_faith(p, origin))).collect();
    let cars = placed.iter().filter(|b| b.car).map(|b| CarBox::of(b, origin)).collect();
    (MeshWorld::new(faith, Vec::new()), cars, count)
}

/// A car's box in Faith's space: what she can vault right across.
#[derive(Clone, Copy, Debug)]
pub struct CarBox {
    pub centre: Vec3,
    /// Unit axes (right, forward, up) and half sizes along them.
    pub axes: [Vec3; 3],
    pub half: Vec3,
}

/// Longest car side Faith vaults right across (most cars are 1.7-2.1 m wide).
pub const VAULT_ACROSS_MAX: f32 = 2.6;

impl CarBox {
    fn of(b: &Placed, origin: Vec3) -> CarBox {
        let c = (b.lo + b.hi) * 0.5;
        let scale = b.axes.map(|a| a.length().max(1e-4));
        CarBox {
            centre: to_faith(b.at(c), origin),
            axes: [0, 1, 2].map(|k| crate::collision::dir_to_faith(b.axes[k] / scale[k])),
            half: (b.hi - b.lo) * 0.5 * Vec3::from_array(scale),
        }
    }

    /// If `at` is in (or just at) this box: how far along `dir` its far side is.
    pub fn across(&self, at: Vec3, dir: Vec3) -> Option<f32> {
        let d = at - self.centre;
        let p = Vec3::new(d.dot(self.axes[0]), d.dot(self.axes[1]), d.dot(self.axes[2]));
        let v = Vec3::new(dir.dot(self.axes[0]), dir.dot(self.axes[1]), dir.dot(self.axes[2]));
        let margin = 0.25;
        if (0..3).any(|k| p[k].abs() > self.half[k] + margin) {
            return None;
        }
        // Where the ray from p along v leaves the box (slab exits).
        let mut t = f32::MAX;
        for k in 0..3 {
            if v[k].abs() > 1e-5 {
                let exit = if v[k] > 0.0 { self.half[k] - p[k] } else { -self.half[k] - p[k] } / v[k];
                t = t.min(exit);
            }
        }
        (t.is_finite() && t > 0.0).then_some(t)
    }
}

/// The map and the moving layer as one world, and cars to vault across.
pub struct GtaWorld<'a> {
    pub layered: faith_move::world::Layered<'a>,
    pub cars: &'a [CarBox],
}

impl faith_move::World for GtaWorld<'_> {
    fn sweep(&self, half: Vec3, start: Vec3, delta: Vec3) -> Option<faith_move::SweepHit> {
        self.layered.sweep(half, start, delta)
    }
    fn overlaps(&self, region: &faith_move::Aabb) -> bool {
        self.layered.overlaps(region)
    }
    fn fixtures(&self) -> &[faith_move::Fixture] {
        self.layered.fixtures()
    }
    fn vault_across(&self, at: Vec3, dir: Vec3) -> Option<f32> {
        self.cars.iter().filter_map(|c| c.across(at, dir)).filter(|d| *d <= VAULT_ACROSS_MAX).reduce(f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faith_move::{Controller, Input, State, Tuning, World};
    use glam::Vec2;

    /// Sprint at the side of a car (1.9 m across, bonnet 1 m high), jump near it: with the
    /// car known she vaults right over and lands beyond; as a plain box she lands on top.
    fn run_at_car(know_car: bool) -> (Vec<&'static str>, Vec3) {
        let floor = MeshWorld::oriented_box(Vec3::new(0., -0.5, 0.), Vec3::new(60., 0.5, 60.), 0.0);
        let car = CarBox { centre: Vec3::new(0., 0.5, -12.0), axes: [Vec3::Z, Vec3::X, Vec3::Y], half: Vec3::new(0.95, 0.5, 2.25) };
        let car_tris = MeshWorld::oriented_box(car.centre, Vec3::new(2.25, 0.5, 0.95), 0.0);
        let still = MeshWorld::new(floor, vec![]);
        let moving = MeshWorld::new(car_tris, vec![]);
        let cars = if know_car { vec![car] } else { vec![] };
        let world = GtaWorld { layered: faith_move::world::Layered { still: &still, moving: &moving }, cars: &cars };
        let mut c = Controller::new(Tuning::default(), Vec3::new(0., 0.05, 0.), 0.0);
        let mut states = vec![];
        let mut jumped = false;
        for _ in 0..400 {
            let jump = !jumped && c.feet.z < -9.6;
            jumped |= jump;
            let before = (c.feet, c.state.name());
            c.step(1.0 / 60.0, &Input { move_axis: Vec2::Y, jump_pressed: jump, jump_held: jump, ..Default::default() }, &world as &dyn World);
            assert!(!crate::mode::through_wall_guard(&mut c, before, &world), "guard stopped a real move");
            if states.last() != Some(&c.state.name()) {
                states.push(c.state.name());
            }
            if c.feet.z < -15.0 {
                break;
            }
        }
        (states, c.feet)
    }

    #[test]
    fn vaults_across_a_car() {
        let (plain, at) = run_at_car(false);
        println!("as a box: {plain:?}, ends at {at:?}");
        let (car, at) = run_at_car(true);
        println!("as a car: {car:?}, ends at {at:?}");
        assert!(car.contains(&"Vault"), "{car:?}");
        assert!(at.z < -13.5 && at.y < 0.2, "landed beyond the car: {at:?}");
        let _ = State::Ground;
    }
}
