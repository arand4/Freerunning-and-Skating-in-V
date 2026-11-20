//! Xbox 360 Texture2D mips: read (inline, or LZO chunks in a .tfc), untile, and swap to PC byte order.
//!
//! The 360 stores DXT textures tiled (in 32x32-block tiles, so small mips are padded to 32 blocks)
//! and its GPU reads 16-bit words big-endian. Untiling follows XGAddress2DTiledOffset (as Xenia
//! and every 360 texture tool implement it); then each 16-bit word is byte-swapped, giving the
//! plain DXT blocks the PC reader decodes.
use me_assets::package::{decompress_chunked, Package};
use me_assets::props::Props;

pub struct Mip {
    pub w: u32,
    pub h: u32,
    /// Plain (untiled, little-endian) DXT or ARGB data.
    pub data: Vec<u8>,
}

/// One mip as stored: bulk flags, element count, size on disk, offset, inline data position, w, h.
pub struct Stored {
    pub flags: i32,
    pub count: usize,
    pub on_disk: i32,
    pub offset: i64,
    pub at: usize,
    pub w: u32,
    pub h: u32,
}

pub struct Texture {
    pub format: String,
    pub cache: Option<String>,
    /// Where the source-art bulk starts (just after the properties) and where the mips end.
    pub source_at: usize,
    pub mips: Vec<Stored>,
    pub end: usize,
}

pub fn read(pkg: &Package, e: &me_assets::package::Export) -> Result<Texture, String> {
    let props = Props::read(pkg, e).map_err(|x| x.to_string())?;
    let format = props.name(pkg, "Format").unwrap_or_else(|| "PF_A8R8G8B8".into());
    let cache = props.name(pkg, "TextureFileCacheName").filter(|c| c != "None");
    let mut c = pkg.cursor(props.end);
    let err = |x: me_assets::Error| x.to_string();
    let (f, _n, d, _o) = (c.i32().map_err(err)?, c.i32().map_err(err)?, c.i32().map_err(err)?, c.i32().map_err(err)?);
    if f & 1 == 0 && d > 0 {
        c.skip(d as usize).map_err(err)?;
    }
    let n = c.count(24).map_err(err)?;
    let mut mips = vec![];
    for _ in 0..n {
        let flags = c.i32().map_err(err)?;
        let count = c.i32().map_err(err)? as usize;
        let on_disk = c.i32().map_err(err)?;
        let offset = c.i32().map_err(err)? as u32 as i64;
        let at = c.pos;
        if flags & 1 == 0 && on_disk > 0 {
            c.skip(on_disk as usize).map_err(err)?;
        }
        let w = c.i32().map_err(err)? as u32;
        let h = c.i32().map_err(err)? as u32;
        mips.push(Stored { flags, count, on_disk, offset, at, w, h });
    }
    Ok(Texture { format, cache, source_at: props.end, mips, end: c.pos })
}

/// (block size in bytes, pixels per block side) of a format.
pub fn block(format: &str) -> Option<(usize, u32)> {
    match format {
        "PF_DXT1" => Some((8, 4)),
        "PF_DXT3" | "PF_DXT5" => Some((16, 4)),
        "PF_A8R8G8B8" => Some((4, 1)),
        _ => None,
    }
}

/// A stored mip's bytes as on the 360 (decompressed): inline, or from the .tfc via `tfc`.
pub fn stored_bytes(pkg: &Package, m: &Stored, tfc: &mut dyn FnMut(i64, usize) -> Result<Vec<u8>, String>) -> Result<Option<Vec<u8>>, String> {
    if m.flags & 0x20 != 0 || m.on_disk <= 0 {
        return Ok(None); // unused
    }
    let raw;
    let src: &[u8] = if m.flags & 1 != 0 {
        raw = tfc(m.offset, m.on_disk as usize)?;
        &raw
    } else {
        &pkg.data[m.at..m.at + m.on_disk as usize]
    };
    Ok(Some(if m.flags & 0x10 != 0 { decompress_chunked(src, 0, true).map_err(|e| e.to_string())? } else { src[..m.count.min(src.len())].to_vec() }))
}

/// Untiles one 360 mip and swaps it to PC byte order.
pub fn convert(format: &str, w: u32, h: u32, tiled: &[u8]) -> Result<Vec<u8>, String> {
    let (bsize, bpx) = block(format).ok_or_else(|| format!("texture format {format}"))?;
    let (bw, bh) = (w.div_ceil(bpx).max(1), h.div_ceil(bpx).max(1));
    let pitch = bw.div_ceil(32) * 32; // tiled rows are padded to 32 blocks
    let mut out = vec![0u8; (bw * bh) as usize * bsize];
    for y in 0..bh {
        for x in 0..bw {
            let src = tiled_offset(x, y, pitch, bsize as u32) as usize * bsize;
            let dst = (y * bw + x) as usize * bsize;
            if src + bsize > tiled.len() {
                return Err(format!("{w}x{h} {format}: tiled data too short ({} bytes)", tiled.len()));
            }
            out[dst..dst + bsize].copy_from_slice(&tiled[src..src + bsize]);
        }
    }
    if bsize == 4 {
        // A8R8G8B8 is a big-endian u32 on the 360: reverse each one (the PC stores B, G, R, A).
        for p in out.chunks_exact_mut(4) {
            p.reverse();
        }
    } else {
        for p in out.chunks_exact_mut(2) {
            p.swap(0, 1);
        }
    }
    Ok(out)
}

/// XGAddress2DTiledOffset: the block index in the tiled data of block (x, y), for a surface
/// `width` blocks wide (a multiple of 32) with `texel` bytes per block.
fn tiled_offset(x: u32, y: u32, width: u32, texel: u32) -> u32 {
    let aligned = (width + 31) & !31;
    let log_bpp = (texel >> 2) + ((texel >> 1) >> (texel >> 2));
    let macro_ = ((x >> 5) + (y >> 5) * (aligned >> 5)) << (log_bpp + 7);
    let micro = ((x & 7) + ((y & 6) << 2)) << log_bpp;
    let offset = macro_ + ((micro & !15) << 1) + (micro & 15) + ((y & 8) << (3 + log_bpp)) + ((y & 1) << 4);
    (((offset & !511) << 3) + ((offset & 448) << 2) + (offset & 63) + ((y & 16) << 7) + (((((y & 8) >> 2) + (x >> 3)) & 3) << 6)) >> log_bpp
}

/// The plain mips of a texture, largest first (unused ones skipped).
pub fn mips(pkg: &Package, t: &Texture, tfc: &mut dyn FnMut(i64, usize) -> Result<Vec<u8>, String>) -> Result<Vec<Mip>, String> {
    let mut out = vec![];
    for m in &t.mips {
        let Some(tiled) = stored_bytes(pkg, m, tfc)? else { continue };
        out.push(Mip { w: m.w, h: m.h, data: convert(&t.format, m.w, m.h, &tiled)? });
    }
    Ok(out)
}
