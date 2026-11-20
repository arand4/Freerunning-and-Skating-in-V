//! FaithRunnerXbox: takes what Faith Runner V reads out of the player's own Mirror's Edge Xbox 360
//! disc image and writes it into the GTA V folder.
mod dump;
mod iso;
mod repack;
mod sound;
mod texture;

const USAGE: &str = "usage: FaithRunnerXbox <Mirror's Edge Xbox 360 .iso> <output folder>
  Writes <output folder>\\TdGame\\CookedXenon\\TdGame.xxx: Faith's body, animations, textures and
  sounds from your own disc, which Faith Runner V reads. For Faith Runner V the output folder is
  <GTA V folder>\\FaithRunnerV\\MirrorsEdge.";

fn main() -> std::process::ExitCode {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("--sound-test") => sound_test(&a[1], &a[2], a.get(3).map(String::as_str)),
        Some("--texture-test") => texture_test(&a[1], &a[2], &a[3]),
        Some("--find-cues") => find_cues(&a[1]),
        Some(iso) if a.len() == 2 && !iso.starts_with('-') => {
            let t0 = std::time::Instant::now();
            match dump::dump(std::path::Path::new(iso), std::path::Path::new(&a[1]), &mut |s| println!("{s}")) {
                Ok(r) => {
                    println!("Done in {:.0} s: {} textures, {} sounds, {:.0} MB in {}", t0.elapsed().as_secs_f32(), r.textures, r.sounds,
                        r.bytes as f64 / 1048576.0, r.path.display());
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            return std::process::ExitCode::from(2);
        }
    }
    std::process::ExitCode::SUCCESS
}

/// Converts Faith's first-person textures from the disc and compares them with the PC's at the same size.
fn texture_test(iso_path: &str, pc: &str, out: &str) {
    use me_assets::package::Package;
    use std::path::Path;
    let mut disc = iso::Disc::open(Path::new(iso_path)).unwrap();
    let x = Package::parse(disc.read_file("TdGame/CookedXenon/TdGame.xxx").unwrap()).unwrap();
    let pcp = Package::open(&Path::new(pc).join("Characters").join("CH_TKY_Crim_Fixer_1P.upk")).unwrap();
    for name in ["Female_1p_C", "Female_1p_N", "Female_1p_S", "Faith_Glove_C", "Faith_Glove_N", "Female_1p_Trans"] {
        let e = x.find("Texture2D", name).unwrap();
        let t = texture::read(&x, e).unwrap();
        let cache = t.cache.clone().unwrap_or_default();
        let file = disc.find(&format!("TdGame/CookedXenon/{cache}.tfc")).ok();
        let mut tfc = |off: i64, len: usize| disc.read_range(file.ok_or("no cache file")?, off as u64, len);
        let mips = match texture::mips(&x, &t, &mut tfc) {
            Ok(m) => m,
            Err(err) => { println!("{name}: {err}"); continue }
        };
        let m = &mips[0];
        let alpha = t.format == "PF_DXT5";
        let rgba = me_assets::texture::decode_bc(&m.data, m.w, m.h, alpha).unwrap();
        let p = me_assets::texture::read_texture(&pcp, name, m.w).unwrap();
        let diff: f64 = if p.width == m.w {
            rgba.iter().zip(&p.pixels).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / rgba.len() as f64
        } else { -1.0 };
        println!("{name}: {} {}x{} from {cache} ({} mips); PC {}x{}; mean difference {diff:.1} of 255", t.format, m.w, m.h, mips.len(), p.width, p.height);
        bmp(&Path::new(out).join(format!("{name}_xbox.bmp")), m.w, m.h, &rgba);
        bmp(&Path::new(out).join(format!("{name}_pc.bmp")), p.width, p.height, &p.pixels);
    }
}

/// Lists, for every package on the disc, the SoundCues of Faith's sound groups it holds (one line each:
/// file, cue path), to find where the cues missing from TdGame.xxx live.
fn find_cues(iso_path: &str) {
    let mut disc = iso::Disc::open(std::path::Path::new(iso_path)).unwrap();
    let files = disc.list().unwrap();
    for (path, _) in files.iter().filter(|(p, _)| p.ends_with(".xxx")) {
        let Ok(raw) = disc.read_file(path) else { continue };
        let Ok(pkg) = me_assets::package::Package::parse(raw) else { eprintln!("{path}: unreadable"); continue };
        for (i, e) in pkg.exports.iter().enumerate() {
            if pkg.class_name(e) == "SoundCue" && dump::SOUNDS.contains(&dump::root(&pkg, i)) {
                println!("{path}\t{}", pkg.path(i as i32 + 1));
            }
        }
    }
}

fn bmp(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) {
    let mut b = vec![];
    let size = 54 + w * h * 4;
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40, w, (-(h as i32)) as u32] { b.extend_from_slice(&v.to_le_bytes()); }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, w * h * 4, 2835, 2835, 0, 0] { b.extend_from_slice(&v.to_le_bytes()); }
    for p in rgba.chunks_exact(4) { b.extend_from_slice(&[p[2], p[1], p[0], 255]); }
    std::fs::write(path, b).unwrap();
}

/// Decodes waves from the Xbox package and compares them with the PC's Ogg of the same name.
fn sound_test(xxx: &str, pc: &str, out: Option<&str>) {
    use me_assets::package::Package;
    use std::path::Path;
    let x = Package::open(Path::new(xxx)).unwrap();
    let pcs: Vec<Package> = ["A_Bodyfalls", "A_Material_Footstep", "A_Character_Female_01", "A_Material_Handstep"]
        .iter()
        .map(|n| Package::open(&Path::new(pc).join("Audio").join(format!("{n}.upk"))).unwrap())
        .collect();
    let mut shown = 0;
    for e in x.of_class("SoundNodeWave") {
        let Some(pkg) = pcs.iter().find(|p| p.find("SoundNodeWave", &e.name).is_some()) else { continue };
        let d = match sound::decode_wave(&x, e) {
            Ok(Some(d)) => d,
            Ok(None) => { println!("{}: no XMA", e.name); continue }
            Err(err) => { println!("{}: {err}", e.name); continue }
        };
        let pe = pkg.find("SoundNodeWave", &e.name).unwrap();
        let props = me_assets::props::Props::read(pkg, pe).unwrap();
        let b = sound::bulks(pkg, props.end, pe.offset + pe.size);
        let (_, size, at) = b[1];
        let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(pkg.data[at..at + size].to_vec())).unwrap();
        let mut ogg: Vec<i16> = vec![];
        while let Ok(Some(p)) = r.read_dec_packet_itl() {
            ogg.extend(p);
        }
        // Best alignment within +-2048 samples: normalised correlation.
        let ch = d.channels as usize;
        let (mut best, mut best_off) = (-1.0f64, 0i64);
        for off in (-2048i64..=2048).step_by(1) {
            let (mut sxy, mut sxx, mut syy) = (0f64, 0f64, 0f64);
            for i in (0..ogg.len().min(20000)).step_by(ch) {
                let j = i as i64 + off * ch as i64;
                if j < 0 || j as usize >= d.samples.len() { continue }
                let (a, b) = (ogg[i] as f64, d.samples[j as usize] as f64);
                sxy += a * b; sxx += a * a; syy += b * b;
            }
            let c = if sxx > 0.0 && syy > 0.0 { sxy / (sxx * syy).sqrt() } else { 0.0 };
            if c > best { best = c; best_off = off; }
        }
        println!("{}: xbox {} Hz {} ch {} samples (errors {}), pc {} Hz {} samples; correlation {best:.3} at offset {best_off}",
            e.name, d.rate, d.channels, d.samples.len() / ch, d.av_errors, r.ident_hdr.audio_sample_rate, ogg.len() / r.ident_hdr.audio_channels as usize);
        if let Some(o) = out {
            std::fs::write(Path::new(o).join(format!("{}.wav", e.name)), sound::wav(&d)).unwrap();
        }
        shown += 1;
        if shown >= 12 { break }
    }
}
