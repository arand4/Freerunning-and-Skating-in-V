//! Xbox 360 SoundNodeWave audio: XMA packets with a small Unreal header, decoded to PCM16 WAV.
//!
//! The third bulk block of a SoundNodeWave (CompressedXbox360Data) holds, big-endian:
//! format size (0x2c), seek table size, data size, then the format (11 u32s), the seek table,
//! and the XMA data as whole 2048-byte packets. The rate and channel count come from the
//! wave's own SampleRate and NumChannels properties.

use me_assets::package::Package;
use me_assets::props::Props;

pub struct Decoded {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
    pub av_errors: u32,
}

/// The bulk blocks after a SoundNodeWave's properties: (flags, size, data start) for raw, PC, Xbox, PS3.
pub fn bulks(pkg: &Package, props_end: usize, end: usize) -> Vec<(i32, usize, usize)> {
    let mut c = pkg.cursor(props_end);
    let mut out = vec![];
    for _ in 0..4 {
        if c.pos + 16 > end {
            break;
        }
        let (Ok(flags), Ok(_count), Ok(disk), Ok(_off)) = (c.i32(), c.i32(), c.i32(), c.i32()) else { break };
        let at = c.pos;
        let inline = flags & 1 == 0 && disk > 0;
        out.push((flags, if inline { disk as usize } else { 0 }, at));
        if inline && c.skip(disk as usize).is_err() {
            break;
        }
    }
    out
}

const DELAY: usize = 576;

pub fn rate_id(rate: u32) -> i32 {
    match rate {
        0..=28000 => 0,
        28001..=38000 => 1,
        38001..=46000 => 2,
        _ => 3,
    }
}

/// Decodes the wave export `e` of an Xbox package, if it holds XMA.
pub fn decode_wave(pkg: &Package, e: &me_assets::package::Export) -> Result<Option<Decoded>, String> {
    let props = Props::read(pkg, e).map_err(|x| x.to_string())?;
    let b = bulks(pkg, props.end, e.offset + e.size);
    let Some(&(flags, size, at)) = b.get(2) else { return Ok(None) };
    if size < 12 || flags & 0x10 != 0 {
        return Ok(None);
    }
    let d = &pkg.data[at..at + size];
    let be = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
    let (fmt, seek, data) = (be(0), be(4), be(8));
    let start = 12 + fmt + seek;
    if start + data > d.len() || data == 0 {
        return Err(format!("{}: XMA sizes {fmt} {seek} {data} don't fit {}", e.name, d.len()));
    }
    let rate = props.i32(pkg, "SampleRate").unwrap_or(44100).max(1) as u32;
    let channels = props.i32(pkg, "NumChannels").unwrap_or(1).clamp(1, 2) as u16;
    let pcm = skate_xma::decode_packets(&d[start..start + data], channels == 2, rate_id(rate));
    // The decoder's output starts ~576 samples late (its first frame and the transform's overlap;
    // measured against the PC game's Ogg of the same waves: 568-581). Then trim to the wave's own
    // length (XMA decodes whole 512-sample frames).
    let lead = (DELAY * channels as usize).min(pcm.samples.len());
    let mut samples = pcm.samples[lead..].to_vec();
    if let Some(dur) = props.f32(pkg, "Duration") {
        let want = ((dur * pcm.sample_rate as f32).round() as usize) * channels as usize;
        if want > 0 && want < samples.len() {
            samples.truncate(want);
        }
    }
    Ok(Some(Decoded { rate: pcm.sample_rate, channels, samples, av_errors: pcm.av_errors }))
}

/// A PCM16 RIFF WAVE file.
pub fn wav(d: &Decoded) -> Vec<u8> {
    let bytes = d.samples.len() * 2;
    let mut w = Vec::with_capacity(44 + bytes);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + bytes) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&d.channels.to_le_bytes());
    w.extend_from_slice(&d.rate.to_le_bytes());
    w.extend_from_slice(&(d.rate * d.channels as u32 * 2).to_le_bytes());
    w.extend_from_slice(&(d.channels * 2).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(bytes as u32).to_le_bytes());
    for s in &d.samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}
