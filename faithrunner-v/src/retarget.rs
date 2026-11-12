//! Faith's third-person animations (Mirror's Edge's AS_F3P_Unarmed, read from the
//! player's install) on a GTA character's skeleton: any ped, the player's own or one
//! picked with a trainer. The method is SK8V's (`ped.rs`, from the 2010 mashup's rig):
//! each mapped GTA bone wears its Faith bone's rotation, carried through a bind fit
//! that swings the GTA bind segment onto Faith's; GTA keeps its own bone lengths.
//! Unmapped GTA bones (roll, helper, face bones) keep their bind pose relative to
//! their parent. The pelvis rises and drops with Faith's hips (scaled to the leg length).
use glam::{Mat3, Mat4, Quat, Vec3};
use me_assets::SkeletalMesh;

/// A GTA skeleton's bind pose (crSkeletonData bones: parent-first order).
#[derive(Clone, Debug)]
pub struct GtaSkeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    /// Bind local rotation and translation (metres) in the parent's frame.
    pub rot: Vec<Quat>,
    pub pos: Vec<Vec3>,
}

impl GtaSkeleton {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    /// Bind globals (model space): rotation, position.
    fn globals(&self) -> (Vec<Quat>, Vec<Vec3>) {
        let (mut r, mut p) = (Vec::with_capacity(self.rot.len()), Vec::with_capacity(self.rot.len()));
        for i in 0..self.rot.len() {
            let parent = self.parents[i];
            if parent < 0 || parent as usize >= i {
                r.push(self.rot[i]);
                p.push(self.pos[i]);
            } else {
                let q = parent as usize;
                r.push((r[q] * self.rot[i]).normalize());
                p.push(p[q] + r[q] * self.pos[i]);
            }
        }
        (r, p)
    }
}

/// (GTA bone, Faith bone, bind segment (GTA child, Faith child)). No segment: the
/// fit of the bone named in `FOLLOW` (chain ends) or none (the pelvis).
type Seg = Option<(&'static str, &'static str)>;
const MAP: &[(&str, &str, Seg)] = &[
    ("SKEL_Pelvis", "Hips", None),
    ("SKEL_Spine0", "Spine", Some(("SKEL_Spine1", "Spine1"))),
    ("SKEL_Spine1", "Spine1", Some(("SKEL_Spine2", "Spine2|SpineX"))),
    // Faith has three spine bones, GTA four: GTA's Spine3 rides along unmapped. (Her
    // first-person skeleton calls the third one SpineX.)
    ("SKEL_Spine2", "Spine2|SpineX", Some(("SKEL_Neck_1", "Neck"))),
    ("SKEL_Neck_1", "Neck", Some(("SKEL_Head", "Head"))),
    ("SKEL_Head", "Head", None),
    ("SKEL_L_Clavicle", "LeftShoulder", Some(("SKEL_L_UpperArm", "LeftArm"))),
    ("SKEL_L_UpperArm", "LeftArm", Some(("SKEL_L_Forearm", "LeftForeArm"))),
    ("SKEL_L_Forearm", "LeftForeArm", Some(("SKEL_L_Hand", "LeftHand"))),
    ("SKEL_L_Hand", "LeftHand", Some(("SKEL_L_Finger20", "LeftHandMiddle1"))),
    ("SKEL_R_Clavicle", "RightShoulder", Some(("SKEL_R_UpperArm", "RightArm"))),
    ("SKEL_R_UpperArm", "RightArm", Some(("SKEL_R_Forearm", "RightForeArm"))),
    ("SKEL_R_Forearm", "RightForeArm", Some(("SKEL_R_Hand", "RightHand"))),
    ("SKEL_R_Hand", "RightHand", Some(("SKEL_R_Finger20", "RightHandMiddle1"))),
    ("SKEL_L_Thigh", "LeftUpLeg", Some(("SKEL_L_Calf", "LeftLeg"))),
    ("SKEL_L_Calf", "LeftLeg", Some(("SKEL_L_Foot", "LeftFoot"))),
    ("SKEL_L_Foot", "LeftFoot", Some(("SKEL_L_Toe0", "LeftToeBase"))),
    ("SKEL_L_Toe0", "LeftToeBase", None),
    ("SKEL_R_Thigh", "RightUpLeg", Some(("SKEL_R_Calf", "RightLeg"))),
    ("SKEL_R_Calf", "RightLeg", Some(("SKEL_R_Foot", "RightFoot"))),
    ("SKEL_R_Foot", "RightFoot", Some(("SKEL_R_Toe0", "RightToeBase"))),
    ("SKEL_R_Toe0", "RightToeBase", None),
];
const FOLLOW: &[(&str, &str)] = &[
    ("SKEL_Head", "SKEL_Neck_1"),
    ("SKEL_L_Toe0", "SKEL_L_Foot"),
    ("SKEL_R_Toe0", "SKEL_R_Foot"),
];
/// Feet are fitted about the vertical only (both binds stand on flat soles; SK8V).
const GROUND_FIT: &[&str] = &["SKEL_L_Foot", "SKEL_R_Foot"];
/// Fingers: GTA FingerN0..N2 on Faith's <Finger>1..3, per side.
const FINGERS: &[(&str, &str)] = &[("0", "Thumb"), ("1", "Index"), ("2", "Middle"), ("3", "Ring"), ("4", "Pinky")];

/// A body frame from bone positions: right, forward, up (columns), orthonormal; mirrored
/// when the skeleton's own handedness says so (Mirror's Edge's Unreal space is left-handed).
fn frame(head: Vec3, l_foot: Vec3, r_foot: Vec3, l_toe: Vec3, l_arm: Vec3, r_arm: Vec3) -> Mat3 {
    let up = (head - (l_foot + r_foot) * 0.5).normalize();
    let toe = l_toe - l_foot;
    let fwd = (toe - up * toe.dot(up)).normalize();
    let right = fwd.cross(up);
    let right = if right.dot(r_arm - l_arm) < 0.0 { -right } else { right };
    Mat3::from_cols(right, fwd, up)
}

/// A pose for GTA: object (ped-space) and local (parent-space) matrices per bone.
#[derive(Clone, Default)]
pub struct PoseOut {
    pub objects: Vec<Mat4>,
    pub locals: Vec<Mat4>,
}

pub struct Retarget {
    /// Per GTA bone: the Faith mesh bone it wears.
    faith_of: Vec<Option<usize>>,
    /// Per GTA bone: the bind fit (GTA model space).
    fit: Vec<Quat>,
    /// Mirror's Edge mesh space -> GTA model space (may mirror).
    conv: Mat3,
    /// Metres per Mirror's Edge unit, scaled to the GTA legs.
    scale: f32,
    gta_rot: Vec<Quat>,
    gta_pos: Vec<Vec3>,
    /// Faith's rest pose, converted: rotations and the hips relative to the root.
    faith_rest_rot: Vec<Quat>,
    faith_rest_hips: Vec3,
    hips: usize,
    root: usize,
}

/// A Faith bone by name; `a|b` takes the first that exists.
fn named(mesh: &SkeletalMesh, name: &str) -> Option<usize> {
    name.split('|').find_map(|n| mesh.bones.iter().position(|b| b.name.eq_ignore_ascii_case(n)))
}

impl Retarget {
    /// `rest`: Faith's rest-pose globals (me_assets::pose::globals of Pose::rest).
    pub fn new(gta: &GtaSkeleton, faith: &SkeletalMesh, rest: &[Mat4]) -> Result<Retarget, String> {
        let (gr, gp) = gta.globals();
        let g = |n: &str| gta.index(n).ok_or(format!("GTA bone {n} missing"));
        let f = |n: &str| named(faith, n).ok_or(format!("Faith bone {n} missing"));
        let fp = |i: usize| rest[i].w_axis.truncate();
        let gf = frame(gp[g("SKEL_Head")?], gp[g("SKEL_L_Foot")?], gp[g("SKEL_R_Foot")?], gp[g("SKEL_L_Toe0")?], gp[g("SKEL_L_UpperArm")?], gp[g("SKEL_R_UpperArm")?]);
        let ff = frame(fp(f("Head")?), fp(f("LeftFoot")?), fp(f("RightFoot")?), fp(f("LeftToeBase")?), fp(f("LeftArm")?), fp(f("RightArm")?));
        let conv = gf * ff.transpose();
        let gleg = (gp[g("SKEL_L_Thigh")?] - gp[g("SKEL_L_Foot")?]).length();
        let fleg = (fp(f("LeftUpLeg")?) - fp(f("LeftFoot")?)).length();
        let scale = gleg / fleg.max(1e-3);
        let to_g = |m: &Mat4| Quat::from_mat3(&(conv * Mat3::from_mat4(*m) * conv.transpose())).normalize();
        let faith_rest_rot: Vec<Quat> = rest.iter().map(to_g).collect();

        let mut map: Vec<(String, String, Option<(String, String)>)> =
            MAP.iter().map(|(a, b, s)| (a.to_string(), b.to_string(), s.map(|(c, d)| (c.to_string(), d.to_string())))).collect();
        for side in ["L", "R"] {
            let fs = if side == "L" { "Left" } else { "Right" };
            for (n, finger) in FINGERS {
                for k in 0..3 {
                    let gname = format!("SKEL_{side}_Finger{n}{k}");
                    let fname = format!("{fs}Hand{finger}{}", k + 1);
                    let seg = (k < 2).then(|| (format!("SKEL_{side}_Finger{n}{}", k + 1), format!("{fs}Hand{finger}{}", k + 2)));
                    map.push((gname, fname, seg));
                }
            }
        }
        let n = gta.names.len();
        let mut faith_of = vec![None; n];
        let mut fit = vec![Quat::IDENTITY; n];
        let mut seg_fit: Vec<(usize, Quat)> = vec![];
        for (gname, fname, seg) in &map {
            let (Some(gi), Some(fi)) = (gta.index(gname), named(faith, fname)) else { continue };
            faith_of[gi] = Some(fi);
            if let Some((gc, fc)) = seg
                && let (Some(gci), Some(fci)) = (gta.index(gc), named(faith, fc))
            {
                let mut sg = gp[gci] - gp[gi];
                let mut sf = conv * (fp(fci) - fp(fi));
                if GROUND_FIT.iter().any(|x| x.eq_ignore_ascii_case(gname)) {
                    sg.z = 0.0;
                    sf.z = 0.0;
                }
                // Only segments that point roughly the same way: in Faith's first-person
                // skeleton the chest (SpineX) hangs off the root, not the spine, and fitting
                // GTA's spine to that "segment" bent the whole torso over.
                if sg.length() > 1e-4 && sf.length() > 1e-4 && sg.normalize().dot(sf.normalize()) > 0.6 {
                    // Rotation taking GTA's bind segment onto Faith's.
                    fit[gi] = Quat::from_rotation_arc(sg.normalize(), sf.normalize());
                    seg_fit.push((gi, fit[gi]));
                }
            }
        }
        // Chain ends follow the bone before them; the last finger joint its middle one.
        for (end, from) in FOLLOW {
            if let (Some(e), Some(fr)) = (gta.index(end), gta.index(from)) {
                fit[e] = fit[fr];
            }
        }
        for side in ["L", "R"] {
            for (n, _) in FINGERS {
                if let (Some(e), Some(fr)) = (gta.index(&format!("SKEL_{side}_Finger{n}2")), gta.index(&format!("SKEL_{side}_Finger{n}1"))) {
                    fit[e] = fit[fr];
                }
            }
        }
        let hips = f("Hips")?;
        let root = named(faith, "root").unwrap_or(0);
        Ok(Retarget {
            faith_of,
            fit,
            conv,
            scale,
            gta_rot: gr,
            gta_pos: gp,
            faith_rest_hips: conv * (fp(hips) - fp(root)),
            faith_rest_rot,
            hips,
            root,
        })
    }

    pub fn mapped(&self) -> usize {
        self.faith_of.iter().filter(|m| m.is_some()).count()
    }

    /// GTA object (ped-space) and local matrices for every bone from Faith's posed globals.
    /// `hide_head`: the head and everything on it (face, hair, hats) shrunk to a point, for
    /// first person with the character's own body.
    pub fn pose(&self, gta: &GtaSkeleton, faith: &[Mat4], hide_head: bool, out: &mut PoseOut) {
        let n = gta.names.len();
        let mut rot: Vec<Quat> = Vec::with_capacity(n);
        let mut pos: Vec<Vec3> = Vec::with_capacity(n);
        let hips_now = self.conv * (faith[self.hips].w_axis.truncate() - faith[self.root].w_axis.truncate());
        let mut lift = (hips_now - self.faith_rest_hips) * self.scale;
        // Root motion is the controller's: keep only a little of the hips' sideways travel.
        let flat = Vec3::new(lift.x, lift.y, 0.0);
        if flat.length() > 0.35 {
            let keep = flat.normalize() * 0.35;
            lift.x = keep.x;
            lift.y = keep.y;
        }
        for i in 0..n {
            let p = gta.parents[i];
            let (pr, pp) = if p < 0 || p as usize >= i { (Quat::IDENTITY, Vec3::ZERO) } else { (rot[p as usize], pos[p as usize]) };
            // Each bone's bind offset from its parent, carried by the parent as posed.
            let parent_bind = if p < 0 || p as usize >= i { Quat::IDENTITY } else { self.gta_rot[p as usize] };
            let offset = if p < 0 || p as usize >= i { self.gta_pos[i] } else { self.gta_pos[i] - self.gta_pos[p as usize] };
            let mut at = pp + pr * (parent_bind.inverse() * offset);
            let r = match self.faith_of[i] {
                Some(fi) => {
                    let posed = Quat::from_mat3(&(self.conv * Mat3::from_mat4(faith[fi]) * self.conv.transpose())).normalize();
                    (posed * self.faith_rest_rot[fi].inverse() * self.fit[i] * self.gta_rot[i]).normalize()
                }
                None => (pr * parent_bind.inverse() * self.gta_rot[i]).normalize(),
            };
            // The hips' rise and drop moves the whole body: in GTA's skeleton the spine and
            // the pelvis both hang off the root bone, so the root carries it.
            if p < 0 {
                at += lift;
            }
            rot.push(r);
            pos.push(at);
        }
        // Found in game: computed this way the body faced backwards. GTA's skeleton root carries
        // a half turn (its bind rotation) that the ped's own space doesn't: turn it back.
        let turn = Mat4::from_rotation_z(std::f32::consts::PI);
        let objects = &mut out.objects;
        objects.clear();
        objects.extend(rot.iter().zip(&pos).map(|(r, p)| turn * Mat4::from_rotation_translation(*r, *p)));
        let bind_local = |i: usize| Mat4::from_rotation_translation(gta.rot[i], gta.pos[i]);
        // Bones on the head (the head itself, then its children: face, hair, props).
        let head = gta.index("SKEL_Head");
        let mut on_head = vec![false; n];
        if hide_head && let Some(h) = head {
            objects[h] *= Mat4::from_scale(Vec3::splat(0.001));
            for i in 0..n {
                let p = gta.parents[i];
                if p >= 0 && (p as usize) < i && (p as usize == h || on_head[p as usize]) {
                    on_head[i] = true;
                    objects[i] = objects[p as usize] * bind_local(i);
                }
            }
        }
        out.locals.clear();
        for i in 0..n {
            let p = gta.parents[i];
            out.locals.push(if on_head[i] {
                bind_local(i)
            } else if p >= 0 && (p as usize) < i {
                out.objects[p as usize].inverse() * out.objects[i]
            } else {
                out.objects[i]
            });
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use me_assets::pose::{TrackMap, globals, sample, Pose};
    use me_assets::{AnimSet, Package};
    use std::path::Path;

    pub fn trevor() -> Option<GtaSkeleton> {
        let p = std::env::var_os("GTA_SKELETON")?;
        let text = std::fs::read_to_string(p).ok()?;
        // Minimal reader for SK8V's skeleton.json: name, parent, r, t per bone.
        let mut sk = GtaSkeleton { names: vec![], parents: vec![], rot: vec![], pos: vec![] };
        for chunk in text.split("\"name\"").skip(1) {
            let name = chunk.split('"').nth(1)?.to_string();
            let num = |key: &str, n: usize| -> Option<Vec<f32>> {
                let s = chunk.split(&format!("\"{key}\"")).nth(1)?;
                let s = s.trim_start_matches([':', ' ', '\n', '\r']);
                let body: String = if s.starts_with('[') { s[1..s.find(']')?].to_string() } else { s.split([',', '\n', '}']).next()?.to_string() };
                let v: Vec<f32> = body.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == n).then_some(v)
            };
            sk.names.push(name);
            sk.parents.push(num("parent", 1)?[0] as i32);
            let r = num("r", 4)?;
            sk.rot.push(Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize());
            let t = num("t", 3)?;
            sk.pos.push(Vec3::new(t[0], t[1], t[2]));
        }
        Some(sk)
    }

    fn line(img: &mut [u8], w: usize, h: usize, a: (f32, f32), b: (f32, f32), c: [u8; 3]) {
        let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for dx in 0..2 {
                for dy in 0..2 {
                    let (xi, yi) = (x as usize + dx, y as usize + dy);
                    if xi < w && yi < h {
                        let o = (yi * w + xi) * 3;
                        img[o..o + 3].copy_from_slice(&c);
                    }
                }
            }
        }
    }

    /// Front and side views of the GTA skeleton in each of Faith's poses, one row per clip.
    #[test]
    fn faith_poses_on_trevor() {
        let (Some(dir), Some(gta), Some(out)) = (std::env::var_os("ME_INSTALL"), trevor(), std::env::var_os("FAITH_SHOTS")) else { return };
        let cooked = Path::new(&dir).join("TdGame").join("CookedPC");
        let pkg = Package::open(&cooked.join(r"Characters\CH_TKY_Crim_Fixer.upk")).unwrap();
        let mesh = SkeletalMesh::read(&pkg, "SK_TKY_Crim_Fixer").unwrap();
        let set = AnimSet::read(&Package::open(&cooked.join(r"Animations\AS_F3P_Unarmed.upk")).unwrap()).unwrap();
        let map = TrackMap::new(&mesh, &set);
        let rest = Pose::rest(&mesh);
        let mut rg = vec![];
        globals(&mesh, &rest, &mut rg);
        let rt = Retarget::new(&gta, &mesh, &rg).expect("retarget");
        println!("mapped {} GTA bones; {} of Faith's mesh bones have tracks", rt.mapped(), map.track_for_bone.iter().filter(|t| t.is_some()).count());
        let clips = [("Stand", 0.5), ("runfwd", 0.2), ("WallrunLeft", 0.2), ("WallRunVertical", 0.2), ("Hang", 0.5), ("VaultOver", 0.3), ("CrouchSlide", 0.5), ("JumpSlow", 0.4)];
        let (cw, chh) = (180usize, 260usize);
        let (w, h) = (cw * clips.len(), chh * 2);
        let mut img = vec![235u8; w * h * 3];
        let mut pose = rest.clone();
        let mut fg = vec![];
        let mut po = PoseOut::default();
        let draw_bones = ["SKEL_Pelvis", "SKEL_Spine0", "SKEL_Spine1", "SKEL_Spine2", "SKEL_Spine3", "SKEL_Neck_1", "SKEL_Head",
            "SKEL_L_Clavicle", "SKEL_L_UpperArm", "SKEL_L_Forearm", "SKEL_L_Hand", "SKEL_R_Clavicle", "SKEL_R_UpperArm", "SKEL_R_Forearm", "SKEL_R_Hand",
            "SKEL_L_Thigh", "SKEL_L_Calf", "SKEL_L_Foot", "SKEL_L_Toe0", "SKEL_R_Thigh", "SKEL_R_Calf", "SKEL_R_Foot", "SKEL_R_Toe0",
            "SKEL_L_Finger21", "SKEL_R_Finger21"];
        for (k, (clip, t)) in clips.iter().enumerate() {
            let seq = set.sequences.iter().find(|(n, _)| n.eq_ignore_ascii_case(clip)).map(|(_, s)| s).expect(clip);
            sample(seq, &map, &rest, *t * seq.length, true, &mut pose);
            globals(&mesh, &pose, &mut fg);
            rt.pose(&gta, &fg, false, &mut po);
            let obj = &po.objects;
            assert!(obj.iter().all(|m| m.is_finite()), "{clip}: finite");
            let head = obj[gta.index("SKEL_Head").unwrap()].w_axis;
            let foot = obj[gta.index("SKEL_L_Foot").unwrap()].w_axis;
            println!("{clip}: head z {:.2}, left foot z {:.2}", head.z, foot.z);
            for (row, view) in [(0, 0usize), (1, 1usize)] {
                // view 0: front (x across), 1: side (y across); z up. 100 px per metre.
                let px = |p: Vec3| {
                    let across = if view == 0 { p.x } else { p.y };
                    ((k * cw) as f32 + cw as f32 * 0.5 + across * 100.0, (row * chh) as f32 + chh as f32 * 0.55 - p.z * 100.0)
                };
                for name in draw_bones {
                    let Some(i) = gta.index(name) else { continue };
                    let p = gta.parents[i];
                    if p < 0 || name == "SKEL_Pelvis" {
                        continue;
                    }
                    let c = if name.contains("_L_") { [200, 40, 40] } else if name.contains("_R_") { [40, 60, 200] } else { [20, 20, 20] };
                    line(&mut img, w, h, px(obj[p as usize].w_axis.truncate()), px(obj[i].w_axis.truncate()), c);
                }
            }
        }
        let mut bmp = vec![];
        let row = w * 3;
        let pad = (4 - row % 4) % 4;
        let size = 54 + (row + pad) * h;
        bmp.extend(b"BM");
        bmp.extend((size as u32).to_le_bytes());
        bmp.extend([0u8; 4]);
        bmp.extend(54u32.to_le_bytes());
        bmp.extend(40u32.to_le_bytes());
        bmp.extend((w as i32).to_le_bytes());
        bmp.extend((h as i32).to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        bmp.extend([0u8; 24]);
        for y in (0..h).rev() {
            for x in 0..w {
                let o = (y * w + x) * 3;
                bmp.extend([img[o + 2], img[o + 1], img[o]]);
            }
            bmp.extend(std::iter::repeat_n(0u8, pad));
        }
        std::fs::write(Path::new(&out).join("trevor_poses.bmp"), bmp).unwrap();
    }
}

#[cfg(test)]
mod own_body_tests {
    use super::*;
    use me_assets::pose::{TrackMap, globals, sample, Pose};
    use me_assets::FaithArms;
    use std::path::Path;

    /// Faith's first-person run on Trevor: the head shrunk to a point, hands ahead of the
    /// face, toes forward (+Y, the ped's forward).
    #[test]
    fn first_person_run_on_trevor() {
        let (Some(dir), Some(gta)) = (std::env::var_os("ME_INSTALL"), super::tests::trevor()) else { return };
        let arms = FaithArms::load(Path::new(&dir), 16).unwrap();
        let rest = Pose::rest(&arms.mesh);
        let mut rg = vec![];
        globals(&arms.mesh, &rest, &mut rg);
        let rt = Retarget::new(&gta, &arms.mesh, &rg).expect("retarget");
        println!("mapped {}", rt.mapped());
        let map = TrackMap::new(&arms.mesh, &arms.anims);
        let seq = arms.anims.sequences.iter().find(|(n, _)| n.eq_ignore_ascii_case("runfwd")).unwrap().1;
        let mut pose = rest.clone();
        sample(seq, &map, &rest, 0.2, true, &mut pose);
        let mut g = vec![];
        globals(&arms.mesh, &pose, &mut g);
        let idx = |n: &str| arms.mesh.bones.iter().position(|b| b.name == n).unwrap();
        for n in ["root", "Hips", "LeftFoot", "Head", "CameraJoint"] {
            println!("{n}: rest {:?} run {:?}", rg[idx(n)].w_axis.truncate(), g[idx(n)].w_axis.truncate());
        }
        let mut out = PoseOut::default();
        rt.pose(&gta, &rg, false, &mut out);
        let hz = |o: &PoseOut, n: &str| o.objects[gta.index(n).unwrap()].w_axis.truncate();
        println!("REST: head {:?} spine2 {:?} pelvis {:?} lhand {:?}", hz(&out, "SKEL_Head"), hz(&out, "SKEL_Spine2"), hz(&out, "SKEL_Pelvis"), hz(&out, "SKEL_L_Hand"));
        for n in ["Spine", "Spine1", "SpineX", "Neck", "Head", "LeftShoulder"] {
            let i = idx(n);
            let q = glam::Quat::from_mat4(&g[i]) * glam::Quat::from_mat4(&rg[i]).inverse();
            println!("  {n} parent {} delta angle {:.0} deg", arms.mesh.bones[arms.mesh.bones[i].parent].name, q.angle_between(glam::Quat::IDENTITY).to_degrees());
        }
        rt.pose(&gta, &g, true, &mut out);
        assert!(out.objects.iter().chain(&out.locals).all(|m| m.is_finite()));
        let at = |n: &str| out.objects[gta.index(n).unwrap()].w_axis.truncate();
        let head = gta.index("SKEL_Head").unwrap();
        let s = out.objects[head].x_axis.truncate().length();
        println!("head scale {s:.4}; head {:?}; hands {:?} {:?}; toes {:?} feet {:?}", at("SKEL_Head"), at("SKEL_L_Hand"), at("SKEL_R_Hand"), at("SKEL_L_Toe0"), at("SKEL_L_Foot"));
        assert!(s < 0.01, "head shrunk");
        assert!(at("SKEL_L_Toe0").y > at("SKEL_L_Foot").y, "toes point forward");
        // Children of the head stay on it (collapsed), e.g. the face's root.
        let child = (0..gta.names.len()).find(|&i| gta.parents[i] == head as i32).unwrap();
        assert!((out.objects[child].w_axis.truncate() - at("SKEL_Head")).length() < 0.01, "{}", gta.names[child]);
    }
}
