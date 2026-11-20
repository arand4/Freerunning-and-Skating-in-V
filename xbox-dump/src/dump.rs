//! The whole dump: TdGame.xxx from the disc, Faith's textures from the .tfc caches, her sounds
//! decoded, written as packages me_assets reads (`<out>/TdGame/CookedXenon/TdGame.xxx`, plus the
//! level packages that hold the rest of her sounds, cut down to those sounds, in `Sounds/`).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use me_assets::package::Package;

use crate::{iso, repack, sound, texture};

const PACKAGE: &str = "TdGame/CookedXenon/TdGame.xxx";
/// Whose textures are converted (the other textures in TdGame.xxx are never read).
const TEXTURES: &[&str] = &["CH_TKY_Crim_Fixer_1P", "CH_TKY_Crim_Fixer", "CH_Materials", "CH_Faith_Cinematic"];
/// Whose sounds are decoded: what Faith Runner plays.
pub const SOUNDS: &[&str] = &[
    "A_Bodyfalls", "A_Character_Disarm", "A_Character_Effects", "A_Character_Female_01", "A_Character_Melee",
    "A_Material_Footstep", "A_Material_Handstep", "A_Props_Interactive", "A_WP_Handling", "A_VO_AI", "A_VO_AI_CUE",
];

/// The level packages that hold the sound cues TdGame.xxx lacks (surfaces like cardboard, air ducts,
/// props): with TdGame.xxx, every cue of Faith's sound groups on the disc (387; found by scanning all
/// 1457 packages with --find-cues and picking the fewest files). Missing ones are skipped.
const SOUND_PACKAGES: &[&str] = &[
    "Scraper_p", "Escape_St1_Spt", "Cranes_Plaza", "Boat_Deck_Spt", "Convoy_Conv-Snipe_Slc_Spt", "Cranes_Puzz_Spt",
    "Cranes_Off_Spt", "Factory_Pursu_Spt", "Stormdrain_StdP-StdE_slc_Spt", "Boat_PDeck_Spt", "Cranes_Off-Roof_Spt",
    "Factory_Arena_Slc_Spt", "Subway_MoPu_Spt", "Boat_Chase_Spt", "Convoy_Mall_Spt", "Cranes_Plaza_Aud", "Cranes_Plaza_Spt",
    "Escape_R1_Spt", "Factory_Facto-Arena_Spt", "Scraper_Lobby-Shaft_Spt", "Stormdrain_StdP_Spt",
];

pub struct Report {
    pub textures: usize,
    pub sounds: usize,
    pub bytes: usize,
    pub path: PathBuf,
}

/// The top-level object (the PC package name) an export sits under.
pub fn root(pkg: &Package, mut i: usize) -> &str {
    for _ in 0..64 {
        let o = pkg.exports[i].outer;
        if o <= 0 {
            return &pkg.exports[i].name;
        }
        i = o as usize - 1;
    }
    ""
}

pub fn dump(iso_path: &Path, out_dir: &Path, log: &mut dyn FnMut(&str)) -> Result<Report, String> {
    let mut disc = iso::Disc::open(iso_path)?;
    log("Reading Mirror's Edge's TdGame.xxx from the disc");
    let raw = disc.read_file(PACKAGE)?;
    let pkg = Package::parse(raw.clone()).map_err(|e| format!("TdGame.xxx: {e}"))?;
    if !pkg.big_endian || pkg.find("SkeletalMesh", me_assets::ARMS_MESH).is_none() {
        return Err("This disc's TdGame.xxx has no Faith in it. Is it Mirror's Edge for Xbox 360?".into());
    }
    let mut replace: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut caches: HashMap<String, (u32, u32)> = HashMap::new();

    let tex: Vec<usize> = (0..pkg.exports.len())
        .filter(|&i| pkg.class_name(&pkg.exports[i]) == "Texture2D" && TEXTURES.contains(&root(&pkg, i)))
        .collect();
    log(&format!("Converting {} textures", tex.len()));
    for &i in &tex {
        let e = &pkg.exports[i];
        let t = texture::read(&pkg, e)?;
        if texture::block(&t.format).is_none() {
            continue;
        }
        let cache = match &t.cache {
            Some(c) => match caches.get(c) {
                Some(f) => Some(*f),
                None => {
                    let f = disc.find(&format!("TdGame/CookedXenon/{c}.tfc")).ok();
                    if let Some(f) = f {
                        caches.insert(c.clone(), f);
                    }
                    f
                }
            },
            None => None,
        };
        let mut tfc = |off: i64, len: usize| disc.read_range(cache.ok_or("its texture cache file isn't on the disc")?, off as u64, len);
        let mips = texture::mips(&pkg, &t, &mut tfc).map_err(|x| format!("{}: {x}", e.name))?;
        let mut b = pkg.data[e.offset..t.source_at].to_vec();
        repack::bulk(&mut b, &[]); // source art
        b.extend_from_slice(&(mips.len() as u32).to_be_bytes());
        for m in &mips {
            repack::bulk(&mut b, &m.data);
            b.extend_from_slice(&m.w.to_be_bytes());
            b.extend_from_slice(&m.h.to_be_bytes());
        }
        b.extend_from_slice(&pkg.data[t.end..e.offset + e.size]);
        replace.insert(i, b);
    }

    log("Decoding sounds");
    let mut sounds = convert_sounds(&pkg, &mut replace)?;
    let bytes = repack::write(&raw, &pkg, &replace, &|_| true)?;
    let path = PACKAGE.split('/').fold(out_dir.to_path_buf(), |p, s| p.join(s));
    save(&path, &bytes, true)?;
    let mut total = bytes.len();

    // The rest of her sounds, from level packages: only the sound groups' objects are kept.
    let dir = path.parent().unwrap().join("Sounds");
    for name in SOUND_PACKAGES {
        let Ok(raw) = disc.read_file(&format!("TdGame/CookedXenon/{name}.xxx")) else { continue };
        log(&format!("Decoding sounds from {name}"));
        let pkg = Package::parse(raw.clone()).map_err(|e| format!("{name}: {e}"))?;
        let mut replace = HashMap::new();
        sounds += convert_sounds(&pkg, &mut replace)?;
        let b = repack::write(&raw, &pkg, &replace, &|i| SOUNDS.contains(&root(&pkg, i)))?;
        save(&dir.join(format!("{name}.xxx")), &b, false)?;
        total += b.len();
    }
    Ok(Report { textures: tex.len(), sounds, bytes: total, path })
}

/// Decodes the Xbox sounds of Faith's sound groups to WAV, in the PC slot. Returns how many.
fn convert_sounds(pkg: &Package, replace: &mut HashMap<usize, Vec<u8>>) -> Result<usize, String> {
    let mut sounds = 0;
    for i in 0..pkg.exports.len() {
        let e = &pkg.exports[i];
        if pkg.class_name(e) != "SoundNodeWave" || !SOUNDS.contains(&root(pkg, i)) || e.size == 0 {
            continue;
        }
        let Some(d) = sound::decode_wave(pkg, e).map_err(|x| format!("{}: {x}", e.name))? else { continue };
        let props = me_assets::props::Props::read(pkg, e).map_err(|x| x.to_string())?;
        let b4 = sound::bulks(pkg, props.end, e.offset + e.size);
        if b4.len() < 4 {
            continue;
        }
        let after = b4[3].2 + b4[3].1;
        let mut b = pkg.data[e.offset..props.end].to_vec();
        repack::bulk(&mut b, &[]); // raw
        repack::bulk(&mut b, &sound::wav(&d)); // "PC": the slot me_assets plays from
        repack::bulk(&mut b, &[]); // Xbox 360
        repack::bulk(&mut b, &[]); // PS3
        b.extend_from_slice(&pkg.data[after..e.offset + e.size]);
        replace.insert(i, b);
        sounds += 1;
    }
    Ok(sounds)
}

/// Writes a package via a .part file, checking it reads back first (`arms`: and has Faith's arms).
fn save(path: &Path, bytes: &[u8], arms: bool) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.parent().unwrap().display()))?;
    let tmp = path.with_extension("xxx.part");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let check = Package::open(&tmp).map_err(|e| format!("the written package doesn't read back: {e}"))?;
    if arms {
        me_assets::SkeletalMesh::read(&check, me_assets::ARMS_MESH).map_err(|e| format!("the written package doesn't read back: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}
