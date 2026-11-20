//! me_assets: read Mirror's Edge's first-person arms from a local install.
//!
//! Nothing from the game ships with this crate. [`FaithArms::load`] opens the
//! player's own `TdGame/CookedPC` packages at runtime:
//! - `Characters/CH_TKY_Crim_Fixer_1P.upk`: the first-person upper body
//!   (`SK_UpperBody`, the shared 1P skeleton) and Faith's skin/glove textures;
//! - `Animations/AS_C1P_Unarmed.upk`: every unarmed first-person animation.
//!
//! Or, from the Xbox 360 version, one `TdGame/CookedXenon/TdGame.xxx` (big-endian) that holds
//! all of those under top-level objects named like the PC packages; Faith Runner V's Xbox dump
//! writes one with plain textures and WAV sounds. [`open_package`] and [`Packages`] hand out that
//! one package for every PC package name.

pub mod anim;
#[cfg(feature = "prologue")]
pub mod level;
#[cfg(feature = "prologue")]
pub mod collision;
#[cfg(feature = "prologue")]
pub mod material;
pub mod morph;
pub mod package;
#[cfg(feature = "prologue")]
pub mod postfx;
pub mod pose;
pub mod props;
pub mod skeletal;
pub mod sound;
#[cfg(feature = "prologue")]
pub mod staticmesh;
pub mod texture;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use anim::{AnimSet, Sequence};
pub use package::Package;
pub use pose::{Pose, Skinner, TrackMap};
#[cfg(feature = "prologue")]
pub use level::{Level, LevelMesh, Placed, Spawn};
pub use morph::MorphTarget;
pub use skeletal::SkeletalMesh;
#[cfg(feature = "prologue")]
pub use staticmesh::StaticMesh;
pub use sound::{Cue, SoundBank, Wave};
pub use texture::Rgba;

#[derive(Debug)]
pub enum Error {
    Io(String, std::io::Error),
    Format(String),
    Missing(String),
    Truncated,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "{p}: {e}"),
            Error::Format(s) => write!(f, "unexpected data: {s}"),
            Error::Missing(s) => write!(f, "not found: {s}"),
            Error::Truncated => write!(f, "file ends early"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub const ARMS_PACKAGE: &str = "Characters/CH_TKY_Crim_Fixer_1P.upk";
/// The Xbox 360's everything-package, in its CookedXenon folder.
pub const XBOX_PACKAGE: &str = "TdGame.xxx";
pub const ANIMS_PACKAGE: &str = "Animations/AS_C1P_Unarmed.upk";
/// TdMove_Disarm's clips (SnatchFwd, SnatchFwd2, SnatchFwd3, SnatchBack, SnatchFail): the
/// one-handed weapon set, as the game swaps it in for the disarm.
pub const DISARM_PACKAGE: &str = "Animations/AS_C1P_OneHanded_Common.upk";
/// The disarm's victim: the patrol cop and the clips it plays as Faith takes its gun
/// (TdAIController.TriggerCannedAnim with the same name as hers).
pub const VICTIM_PACKAGE: &str = "Characters/CH_TKY_Cop_Patrol.upk";
pub const VICTIM_MESH: &str = "SK_TKY_Cop_Patrol";
pub const VICTIM_ANIMS: &str = "Animations/AS_AI_PatrolCop_OneHanded.upk";
pub const VICTIM_SET: &str = "AS_AI_PatrolCop_OneHanded";
pub const ARMS_MESH: &str = "SK_UpperBody";
pub const LEGS_MESH: &str = "SK_LowerBody";
/// The arms' morph targets (TdPawnMesh1p.MorphSets): the forearm twist fixes.
pub const ARMS_MORPHS: &str = "Female1p_UpperBody_MorphSet";

/// One material slot: its diffuse, normal and specular textures.
pub struct MaterialSlot {
    pub material: String,
    pub diffuse: Option<Rgba>,
    /// Tangent-space normal map (the game's TC_Normalmap textures).
    pub normal: Option<Rgba>,
    /// Specular intensity map (grey-scale in practice).
    pub specular: Option<Rgba>,
}

pub struct FaithArms {
    pub mesh: SkeletalMesh,
    pub anims: AnimSet,
    pub materials: Vec<MaterialSlot>,
    /// Faith's lower body (same skeleton, so the same pose drives it).
    pub legs: Option<(SkeletalMesh, Vec<MaterialSlot>)>,
    /// The arms mesh's morph targets (empty if they don't fit the mesh).
    pub morphs: Vec<MorphTarget>,
    pub cooked_pc: PathBuf,
}

/// Opens packages by name from CookedPC (any subfolder), caching them, so
/// references into other packages (materials, textures, sounds) can be followed.
pub struct Packages {
    cooked: PathBuf,
    /// The Xbox 360 TdGame.xxx, which stands for every package, then the dump's sound packages
    /// (`Sounds/*.xxx`: level packages cut down to Faith's sounds).
    xbox: Vec<Rc<Package>>,
    files: HashMap<String, PathBuf>,
    open: HashMap<String, Rc<Package>>,
}

impl Packages {
    pub fn new(cooked: &Path) -> Self {
        let mut files = HashMap::new();
        let mut stack = vec![cooked.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    files.entry(stem.to_ascii_lowercase()).or_insert(p);
                }
            }
        }
        let mut xbox = vec![];
        if is_xbox(cooked) {
            xbox.extend(open_package(cooked, XBOX_PACKAGE).ok());
            let mut more: Vec<PathBuf> = std::fs::read_dir(cooked.join("Sounds"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xxx")))
                .collect();
            more.sort();
            xbox.extend(more.iter().filter_map(|p| Package::open(p).ok().map(Rc::new)));
        }
        Packages { cooked: cooked.to_path_buf(), xbox, files, open: HashMap::new() }
    }

    pub fn has(&self, name: &str) -> bool {
        if !self.xbox.is_empty() {
            return self.xbox.iter().any(|x| x.exports.iter().any(|e| e.outer == 0 && e.name.eq_ignore_ascii_case(name)));
        }
        self.files.contains_key(&name.to_ascii_lowercase())
    }

    pub fn get(&mut self, name: &str) -> Result<Rc<Package>> {
        if let Some(x) = self.xbox.first() {
            return Ok(x.clone());
        }
        let key = name.to_ascii_lowercase();
        if let Some(p) = self.open.get(&key) {
            return Ok(p.clone());
        }
        let path = self
            .files
            .get(&key)
            .cloned()
            .ok_or_else(|| Error::Missing(format!("{name}.upk under {}", self.cooked.display())))?;
        let p = Rc::new(Package::open(&path)?);
        self.open.insert(key, p.clone());
        Ok(p)
    }

    /// The package holding the export `path` (full, with the PC package name first) of `class`, and
    /// its index there: on the Xbox, TdGame.xxx or one of the sound packages.
    pub fn find(&mut self, class: &str, path: &str) -> Option<(Rc<Package>, usize)> {
        self.find_all(class, path).into_iter().next()
    }

    /// Every copy of the export (the Xbox's level packages each hold their own copies of shared
    /// objects, not always complete); one at most on PC.
    pub fn find_all(&mut self, class: &str, path: &str) -> Vec<(Rc<Package>, usize)> {
        if !self.xbox.is_empty() {
            return self.xbox.iter().filter_map(|p| p.find_path(class, path).map(|i| (p.clone(), i))).collect();
        }
        let Some((name, rest)) = path.split_once('.') else { return vec![] };
        if !self.has(name) {
            return vec![];
        }
        let Ok(p) = self.get(name) else { return vec![] };
        p.find_path(class, rest).map(|i| (p, i)).into_iter().collect()
    }

    /// Follow an object reference (export, or import from another package) to
    /// the package that actually holds it and the export's index there.
    pub fn resolve(&mut self, pkg: &Rc<Package>, object: i32) -> Option<(Rc<Package>, usize)> {
        if object > 0 {
            return Some((pkg.clone(), object as usize - 1));
        }
        if object == 0 {
            return None;
        }
        let imp = pkg.imports.get((-object) as usize - 1)?;
        let mut top = imp.outer;
        let mut top_name = imp.name.clone();
        while top < 0 {
            let o = pkg.imports.get((-top) as usize - 1)?;
            top_name = o.name.clone();
            top = o.outer;
        }
        let other = self.get(&top_name).ok()?;
        let idx = other
            .exports
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(&imp.name) && other.class_name(e) == imp.class)?;
        Some((other, idx))
    }
}

impl FaithArms {
    /// Load from a Mirror's Edge install folder (the one containing `TdGame`),
    /// or directly from its `TdGame/CookedPC` folder.
    pub fn load(install: &Path, max_texture: u32) -> Result<Self> {
        let cooked = cooked_pc(install).ok_or_else(|| {
            Error::Missing(format!("TdGame/CookedPC under {}", install.display()))
        })?;
        let mut pkgs = Packages::new(&cooked);
        let arms_pkg = open_package(&cooked, ARMS_PACKAGE)?;
        let mesh = SkeletalMesh::read(&arms_pkg, ARMS_MESH)?;
        let mut anims = read_anim_set(&cooked, ANIMS_PACKAGE)?;
        if let Ok(disarm) = read_anim_set(&cooked, DISARM_PACKAGE) {
            anims.add_from(&disarm, |n| n.to_ascii_lowercase().starts_with("snatch"));
        }
        let materials = slots(&mut pkgs, &arms_pkg, &mesh, max_texture);
        let legs = SkeletalMesh::read(&arms_pkg, LEGS_MESH).ok().filter(|l| {
            l.bones.len() == mesh.bones.len() && l.bones.iter().zip(&mesh.bones).all(|(a, b)| a.name == b.name)
        });
        let legs = legs.map(|l| {
            let m = slots(&mut pkgs, &arms_pkg, &l, max_texture);
            (l, m)
        });
        let morphs = morph::read_morph_set(&arms_pkg, ARMS_MORPHS)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.base_vertices as usize == mesh.vertices.len())
            .collect();
        Ok(FaithArms { mesh, anims, materials, legs, morphs, cooked_pc: cooked })
    }

    /// Another of the game's characters, skeleton and animations only (no textures, legs or
    /// morphs): `mesh` from `package`, the AnimSet `set` from `anims` (e.g. the patrol cop and
    /// AS_AI_PatrolCop_OneHanded, for the disarm's victim side).
    pub fn load_character(install: &Path, package: &str, mesh: &str, anims: &str, set: &str) -> Result<Self> {
        let cooked = cooked_pc(install).ok_or_else(|| Error::Missing(format!("TdGame/CookedPC under {}", install.display())))?;
        let pkg = open_package(&cooked, package)?;
        let mesh = SkeletalMesh::read(&pkg, mesh)?;
        let anims = AnimSet::read_named(&*open_package(&cooked, anims)?, set)?;
        Ok(FaithArms { mesh, anims, materials: vec![], legs: None, morphs: vec![], cooked_pc: cooked })
    }

    pub fn bone(&self, name: &str) -> Option<usize> {
        self.mesh.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// Accept either the install root or the CookedPC folder itself; or, for the Xbox 360 version,
/// a folder with `TdGame/CookedXenon/TdGame.xxx` (or that CookedXenon folder).
pub fn cooked_pc(path: &Path) -> Option<PathBuf> {
    for c in [path.join("TdGame").join("CookedPC"), path.join("CookedPC"), path.to_path_buf()] {
        if c.join(ARMS_PACKAGE).is_file() && c.join(ANIMS_PACKAGE).is_file() {
            return Some(c);
        }
    }
    [path.join("TdGame").join("CookedXenon"), path.join("CookedXenon"), path.to_path_buf()].into_iter().find(|c| is_xbox(c))
}

/// A CookedXenon folder (the Xbox 360 version's TdGame.xxx) rather than CookedPC.
pub fn is_xbox(cooked: &Path) -> bool {
    cooked.join(XBOX_PACKAGE).is_file()
}

thread_local! {
    /// Packages already open on this thread (so the Xbox's one big package is read once per loader).
    static OPEN: std::cell::RefCell<HashMap<PathBuf, std::rc::Weak<Package>>> = std::cell::RefCell::new(HashMap::new());
}

/// Opens `rel` (a PC package path like `Characters/CH_TKY_Crim_Fixer_1P.upk`) under `cooked`, or,
/// in a CookedXenon folder, the TdGame.xxx that holds it. Shared while any user keeps it.
pub fn open_package(cooked: &Path, rel: &str) -> Result<Rc<Package>> {
    let path = if is_xbox(cooked) { cooked.join(XBOX_PACKAGE) } else { cooked.join(rel) };
    if let Some(p) = OPEN.with(|o| o.borrow().get(&path).and_then(|w| w.upgrade())) {
        return Ok(p);
    }
    let p = Rc::new(Package::open(&path)?);
    OPEN.with(|o| o.borrow_mut().insert(path, Rc::downgrade(&p)));
    Ok(p)
}

/// The AnimSet of a PC animation package (on the Xbox: the set of that name in TdGame.xxx).
pub fn read_anim_set(cooked: &Path, rel: &str) -> Result<AnimSet> {
    let pkg = open_package(cooked, rel)?;
    if pkg.big_endian {
        let name = Path::new(rel).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        AnimSet::read_named(&pkg, name)
    } else {
        AnimSet::read(&pkg)
    }
}

fn slots(pkgs: &mut Packages, pkg: &Rc<Package>, mesh: &SkeletalMesh, max_texture: u32) -> Vec<MaterialSlot> {
    mesh.materials
        .iter()
        .map(|&m| {
            let material = pkg.object_name(m).to_string();
            let mut read = |pick: &dyn Fn(&str) -> bool| {
                let (mp, mi) = pkgs.resolve(pkg, m)?;
                let tex = material_textures(&mp, mi).into_iter().find(|(n, _)| pick(&n.to_ascii_lowercase()))?.1;
                let (tp, ti) = pkgs.resolve(&mp, tex)?;
                texture::read_texture(&tp, &tp.exports[ti].name, max_texture).ok()
            };
            let diffuse = read(&|n| n.contains("diffuse"));
            let normal = read(&|n| n.contains("normal"));
            let specular = read(&|n| n.contains("spec") && !n.contains("tile"));
            MaterialSlot { material, diffuse, normal, specular }
        })
        .collect()
}

/// A material instance's texture parameters: (parameter name, texture
/// object index in `pkg`).
pub fn material_textures(pkg: &Package, export: usize) -> Vec<(String, i32)> {
    let mut out = vec![];
    let Some(e) = pkg.exports.get(export) else { return out };
    let Ok(props) = props::Props::read(pkg, e) else { return out };
    let Some(arr) = props.get("TextureParameterValues") else { return out };
    // Array of structs, each a tagged property list: walk it.
    let mut c = pkg.cursor(arr.at);
    let Ok(n) = c.count(8) else { return out };
    let mut walk = || -> Option<()> {
        for _ in 0..n {
            let mut pname = String::new();
            let mut value = 0;
            loop {
                let name = package::read_name(&pkg.names, &mut c).ok()?;
                if name == "None" {
                    break;
                }
                let kind = package::read_name(&pkg.names, &mut c).ok()?;
                let size = c.i32().ok()? as usize;
                c.i32().ok()?;
                if kind == "StructProperty" {
                    package::read_name(&pkg.names, &mut c).ok()?;
                }
                if kind == "BoolProperty" {
                    c.i32().ok()?;
                    continue;
                }
                let at = c.pos;
                match name.as_str() {
                    "ParameterName" => pname = package::read_name(&pkg.names, &mut c).ok()?,
                    "ParameterValue" => value = c.i32().ok()?,
                    _ => {}
                }
                c.pos = at + size;
            }
            if value != 0 {
                out.push((pname, value));
            }
        }
        Some(())
    };
    walk();
    out
}

#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "prologue"))]
mod level_tests;
