//! SkeletalMesh (skeleton + LOD0 geometry) as stored by Mirror's Edge.
//!
//! Layout after the property block (all verified against SK_UpperBody):
//! i32 (ME-specific, 1) · bounds (7 f32) · materials · origin · rot-origin ·
//! bones[52 bytes] · skeletal depth · LOD models. In LOD0: sections are 10 bytes
//! (u16 material, u16 chunk, u32 first index, u16 triangles); two index lists;
//! active bones; per-triangle shadow flags; chunks of rigid (49-byte) and soft
//! (56-byte) vertices with a per-chunk bone map.

use crate::package::{Cursor, Package};
use crate::props::Props;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    pub parent: usize,
    /// Rest rotation (x, y, z, w) in parent space, UE3 mesh convention.
    pub rotation: [f32; 4],
    /// Rest translation in parent space, Unreal units (cm).
    pub translation: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Tangent (TangentX) and bitangent (TangentY): the normal map's basis.
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub uv: [f32; 2],
    /// Mesh bone indices and weights (0..=255, summing to 255).
    pub bones: [u16; 4],
    pub weights: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct Section {
    pub material: u16,
    pub first_index: u32,
    pub triangles: u32,
}

#[derive(Clone, Debug)]
pub struct SkeletalMesh {
    pub name: String,
    /// Package object indices of the material slots.
    pub materials: Vec<i32>,
    pub bones: Vec<Bone>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub sections: Vec<Section>,
}

impl SkeletalMesh {
    pub fn read(pkg: &Package, name: &str) -> Result<Self> {
        let e = pkg
            .find("SkeletalMesh", name)
            .ok_or_else(|| Error::Missing(format!("SkeletalMesh {name}")))?;
        let props = Props::read(pkg, e)?;
        let mut c = pkg.cursor(props.end);
        c.i32()?; // ME-specific leading int
        c.skip(28)?; // bounds
        let nmat = c.count(4)?;
        let materials = (0..nmat).map(|_| c.i32()).collect::<Result<Vec<_>>>()?;
        c.skip(12 + 12)?; // origin, rotation origin

        let nb = c.count(52)?;
        let mut bones = Vec::with_capacity(nb);
        for i in 0..nb {
            let name = crate::package::read_name(&pkg.names, &mut c)?;
            let _flags = c.i32()?;
            let rotation = [c.f32()?, c.f32()?, c.f32()?, c.f32()?];
            let translation = c.vec3()?;
            let _children = c.i32()?;
            let parent = c.i32()?;
            let _color = c.i32()?;
            if parent < 0 || (parent as usize >= nb) || (i > 0 && parent as usize >= i) {
                return Err(Error::Format(format!("bone {name} has bad parent {parent}")));
            }
            bones.push(Bone { name, parent: parent as usize, rotation, translation });
        }
        let _depth = c.i32()?;
        let nlod = c.count(1)?;
        if nlod == 0 {
            return Err(Error::Format("mesh has no LODs".into()));
        }

        // ---- LOD 0
        let nsec = c.count(10)?;
        let mut sections = Vec::with_capacity(nsec);
        for _ in 0..nsec {
            let material = c.u16()?;
            let _chunk = c.u16()?;
            let first_index = c.u32()?;
            let triangles = c.u16()? as u32;
            sections.push(Section { material, first_index, triangles });
        }
        let elem = c.i32()? as usize;
        let nidx = c.count(elem.max(1))?;
        let mut indices = Vec::with_capacity(nidx);
        for _ in 0..nidx {
            indices.push(match elem {
                2 => c.u16()? as u32,
                4 => c.u32()?,
                _ => return Err(Error::Format(format!("index size {elem}"))),
            });
        }
        let n = c.count(2)?;
        c.skip(n * 2)?; // shadow indices
        let n = c.count(2)?;
        c.skip(n * 2)?; // active bone indices
        let n = c.count(1)?;
        c.skip(n)?; // shadow triangle double-sided flags

        let nchunks = c.count(4)?;
        let mut vertices = Vec::new();
        // Per chunk: its bone map and vertex count (for the Xbox 360's separate vertex buffer).
        let mut chunk_maps: Vec<(Vec<u16>, usize)> = Vec::with_capacity(nchunks);
        for _ in 0..nchunks {
            let _base = c.i32()?;
            let nr = c.count(49)?;
            let rigid_at = c.pos;
            c.skip(nr * 49)?;
            let ns = c.count(56)?;
            let soft_at = c.pos;
            c.skip(ns * 56)?;
            let nmap = c.count(2)?;
            let bonemap = (0..nmap).map(|_| c.u16()).collect::<Result<Vec<_>>>()?;
            let (num_rigid, num_soft) = (c.i32()?.max(0) as usize, c.i32()?.max(0) as usize);
            c.i32()?; // max influences
            chunk_maps.push((bonemap.clone(), num_rigid + num_soft));
            let map = |b: u8| -> Result<u16> {
                bonemap.get(b as usize).copied().ok_or_else(|| Error::Format(format!("bone map index {b}")))
            };
            for k in 0..nr {
                let mut v = pkg.cursor(rigid_at + 49 * k);
                let (position, [tangent, bitangent, normal], uv) = read_vertex_common(&mut v)?;
                let bone = pkg.data[rigid_at + 49 * k + 48];
                vertices.push(Vertex { position, normal, tangent, bitangent, uv, bones: [map(bone)?, 0, 0, 0], weights: [255, 0, 0, 0] });
            }
            for k in 0..ns {
                let at = soft_at + 56 * k;
                let mut v = pkg.cursor(at);
                let (position, [tangent, bitangent, normal], uv) = read_vertex_common(&mut v)?;
                let b = &pkg.data[at + 48..at + 52];
                let w = &pkg.data[at + 52..at + 56];
                vertices.push(Vertex {
                    position,
                    normal,
                    tangent,
                    bitangent,
                    uv,
                    bones: [map(b[0])?, map(b[1])?, map(b[2])?, map(b[3])?],
                    weights: [w[0], w[1], w[2], w[3]],
                });
            }
        }
        if vertices.is_empty() && pkg.big_endian {
            vertices = read_xbox_vertices(&mut c, &chunk_maps)?;
        }
        if let Some(&max) = indices.iter().max() {
            if max as usize >= vertices.len() {
                return Err(Error::Format(format!("index {max} beyond {} vertices", vertices.len())));
            }
        }
        Ok(SkeletalMesh { name: name.to_string(), materials, bones, vertices, indices, sections })
    }
}

/// Position, packed tangent basis (TangentX, TangentY, TangentZ = normal), UV.
fn read_vertex_common(c: &mut Cursor) -> Result<([f32; 3], [[f32; 3]; 3], [f32; 2])> {
    let position = c.vec3()?;
    let mut basis = [[0.0; 3]; 3];
    for b in &mut basis {
        let z = c.bytes(4)?;
        *b = [z[0] as f32 / 127.5 - 1.0, z[1] as f32 / 127.5 - 1.0, z[2] as f32 / 127.5 - 1.0];
    }
    let uv = [c.f32()?, c.f32()?];
    Ok((position, basis, uv))
}

/// Xbox 360 meshes keep their chunks' vertex lists empty: the vertices follow the LOD's other data
/// as one GPU skinning buffer. After the chunks: size, vertex count, an empty array, required bones, the raw point
/// indices (bulk data), texture coordinate count, full-precision UVs flag, then the vertices
/// (element size, count): position (3 floats), TangentX and TangentZ (packed normals stored as a
/// big-endian u32, so their bytes are reversed), 4 bone indices and 4 weights (also reversed u32s),
/// and UVs (2 half floats, or 2 floats when full precision). Vertices are in chunk order.
fn read_xbox_vertices(c: &mut Cursor, chunks: &[(Vec<u16>, usize)]) -> Result<Vec<Vertex>> {
    let _size = c.i32()?;
    let count = c.count(0)?;
    let n = c.count(1)?;
    c.skip(n)?; // an array that's empty in Faith's meshes (Mirror's Edge's own)
    let n = c.count(1)?;
    c.skip(n)?; // required bones
    let flags = c.i32()?;
    let _elements = c.i32()?;
    let on_disk = c.i32()?;
    let _offset = c.i32()?;
    if flags & 0x01 == 0 && on_disk > 0 {
        c.skip(on_disk as usize)?; // raw point indices, inline
    }
    let _uv_sets = c.i32()?;
    let full_uvs = c.i32()? != 0;
    let elem = c.i32()? as usize;
    let n = c.count(elem.max(1))?;
    let want = if full_uvs { 36 } else { 32 };
    if n != count || elem < want {
        return Err(Error::Format(format!("Xbox vertex buffer: {n} vertices of {elem} bytes (expected {count} of {want})")));
    }
    let start = c.pos;
    c.skip(n * elem)?;
    let data = c.data;
    let rev = |at: usize| [data[at + 3], data[at + 2], data[at + 1], data[at]];
    let unpack = |b: [u8; 4]| [b[0] as f32 / 127.5 - 1.0, b[1] as f32 / 127.5 - 1.0, b[2] as f32 / 127.5 - 1.0, b[3] as f32 / 127.5 - 1.0];
    let half = |at: usize| half_to_f32(u16::from_be_bytes([data[at], data[at + 1]]));
    let mut out = Vec::with_capacity(n);
    let mut chunk = 0;
    let mut left = chunks.first().map_or(0, |c| c.1);
    for k in 0..n {
        while left == 0 && chunk + 1 < chunks.len() {
            chunk += 1;
            left = chunks[chunk].1;
        }
        left = left.saturating_sub(1);
        let map = chunks.get(chunk).map(|c| c.0.as_slice()).unwrap_or(&[]);
        let at = start + k * elem;
        let mut v = Cursor::new_be(data, at, true);
        let position = v.vec3()?;
        let tx = unpack(rev(at + 12));
        let tz = unpack(rev(at + 16));
        let (tangent, normal) = ([tx[0], tx[1], tx[2]], [tz[0], tz[1], tz[2]]);
        // TangentY = (TangentZ x TangentX) * the sign kept in TangentZ.w.
        let sign = if tz[3] < 0.0 { -1.0 } else { 1.0 };
        let bitangent = [
            (normal[1] * tangent[2] - normal[2] * tangent[1]) * sign,
            (normal[2] * tangent[0] - normal[0] * tangent[2]) * sign,
            (normal[0] * tangent[1] - normal[1] * tangent[0]) * sign,
        ];
        let b = rev(at + 20);
        let w = rev(at + 24);
        let bone = |i: u8| -> Result<u16> { map.get(i as usize).copied().ok_or_else(|| Error::Format(format!("bone map index {i}"))) };
        let bones = [bone(b[0])?, if w[1] > 0 { bone(b[1])? } else { 0 }, if w[2] > 0 { bone(b[2])? } else { 0 }, if w[3] > 0 { bone(b[3])? } else { 0 }];
        let uv = if full_uvs {
            let mut u = Cursor::new_be(data, at + 28, true);
            [u.f32()?, u.f32()?]
        } else {
            [half(at + 28), half(at + 30)]
        };
        out.push(Vertex { position, normal, tangent, bitangent, uv, bones, weights: w });
    }
    Ok(out)
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let frac = (h & 0x3FF) as f32;
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => if frac == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}
