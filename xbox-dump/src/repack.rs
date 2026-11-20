//! Rewrites the 360's TdGame.xxx as an uncompressed package (still big-endian, which me_assets
//! reads) in which Faith's textures hold plain inline mips and her sounds hold WAV in the PC slot.
//! Everything else is copied unchanged.
use me_assets::package::Package;

/// The summary fields of the original file (as on disk, before the chunks were expanded).
struct Summary {
    /// Offset of the CompressionFlags field.
    compression_at: usize,
    /// End of the chunk table.
    chunks_end: usize,
    /// Where the expanded data starts (the first chunk's uncompressed offset).
    data_start: usize,
}

fn summary(raw: &[u8]) -> Result<Summary, String> {
    let be = |o: usize| u32::from_be_bytes(raw[o..o + 4].try_into().unwrap()) as usize;
    let mut o = 12;
    let folder = be(o) as i32;
    o += 4 + if folder > 0 { folder as usize } else { (-folder) as usize * 2 };
    o += 4 + 7 * 4 + 16; // flags, tables, guid
    let gens = be(o);
    o += 4 + gens * 12 + 8; // generations, engine, cooker
    let compression_at = o;
    let n = be(o + 4);
    let chunks_end = o + 8 + n * 16;
    let data_start = if n > 0 { be(o + 8) } else { chunks_end };
    Ok(Summary { compression_at, chunks_end, data_start })
}

/// Positions of each export entry's SerialSize field (SerialOffset follows it).
fn export_fields(pkg: &Package, export_off: usize, count: usize) -> Result<Vec<usize>, String> {
    let mut c = pkg.cursor(export_off);
    let mut out = Vec::with_capacity(count);
    let err = |e: me_assets::Error| e.to_string();
    for _ in 0..count {
        c.skip(4 * 3 + 8 + 4 + 8).map_err(err)?; // class, super, outer, name, archetype, flags
        out.push(c.pos);
        c.skip(8).map_err(err)?;
        let comps = c.count(12).map_err(err)?;
        c.skip(comps * 12 + 4).map_err(err)?;
        let net = c.count(4).map_err(err)?;
        c.skip(net * 4 + 16 + 4).map_err(err)?;
    }
    Ok(out)
}

/// `raw`: the file as on the disc; `pkg`: it parsed; `replace`: new serial bytes per export index.
/// Exports `keep` says no to are left empty (size 0), to cut a package down to what's read.
pub fn write(raw: &[u8], pkg: &Package, replace: &std::collections::HashMap<usize, Vec<u8>>, keep: &dyn Fn(usize) -> bool) -> Result<Vec<u8>, String> {
    let s = summary(raw)?;
    let be = |o: usize| u32::from_be_bytes(pkg.data[o..o + 4].try_into().unwrap()) as usize;
    // Summary with no compression and no chunks; the bytes after the chunk table keep their place.
    let mut out = raw[..s.compression_at].to_vec();
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    let tail = s.data_start.checked_sub(out.len()).ok_or("summary longer than expected")?;
    out.extend_from_slice(&raw[s.chunks_end..s.chunks_end + tail]);
    // Tables (names, imports, exports, depends...): everything up to the first object.
    let first = pkg.exports.iter().filter(|e| e.size > 0).map(|e| e.offset).min().unwrap_or(pkg.data.len());
    out.extend_from_slice(&pkg.data[s.data_start..first]);
    // The export table offset, read from the summary: tag, version, header size, folder, flags,
    // name count, name offset, export count, export offset.
    let folder = be(12) as i32;
    let after_folder = 16 + if folder > 0 { folder as usize } else { (-folder) as usize * 2 };
    let export_count = be(after_folder + 4 + 8);
    let export_off = be(after_folder + 4 + 12);
    let fields = export_fields(pkg, export_off, export_count)?;
    let mut order: Vec<usize> = (0..pkg.exports.len()).filter(|&i| pkg.exports[i].size > 0).collect();
    order.sort_by_key(|&i| pkg.exports[i].offset);
    let mut place = vec![(0usize, 0usize); pkg.exports.len()];
    for i in order {
        let e = &pkg.exports[i];
        let bytes: &[u8] = match replace.get(&i) {
            Some(b) => b,
            None if !keep(i) => &[],
            None => &pkg.data[e.offset..e.offset + e.size],
        };
        place[i] = (bytes.len(), out.len());
        out.extend_from_slice(bytes);
    }
    for (i, &(size, offset)) in place.iter().enumerate() {
        if pkg.exports[i].size == 0 {
            continue;
        }
        let f = fields[i];
        out[f..f + 4].copy_from_slice(&(size as u32).to_be_bytes());
        out[f + 4..f + 8].copy_from_slice(&(offset as u32).to_be_bytes());
    }
    Ok(out)
}

/// A bulk data header for inline, uncompressed bytes (or an empty one).
pub fn bulk(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(data);
}
