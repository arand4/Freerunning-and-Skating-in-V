//! Xbox 360 disc image (XDVDFS) reader, after SK8V's tools/xiso.py (Apache-2.0, becaring).
//!
//! The game partition starts at a fixed offset that depends on the disc format (0 for a trimmed
//! image, XGD2 0xFD90000, XGD3 0x2080000, XGD1 0x18300000). Its volume descriptor is at sector
//! 32: "MICROSOFT*XBOX*MEDIA", then the root directory's sector and size. A directory is a binary
//! tree of entries: left and right subtree offsets (in 4-byte units, 0 = none), the entry's start
//! sector and size, attributes (0x10 = directory), name length and name. Sectors are 2048 bytes.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const SECTOR: u64 = 2048;
const MAGIC: &[u8] = b"MICROSOFT*XBOX*MEDIA";
const PARTITIONS: [u64; 4] = [0, 0xFD9_0000, 0x208_0000, 0x1830_0000];

pub struct Disc {
    f: File,
    base: u64,
    root: (u32, u32),
}

impl Disc {
    pub fn open(path: &Path) -> Result<Disc, String> {
        let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for base in PARTITIONS {
            let mut head = [0u8; 28];
            if f.seek(SeekFrom::Start(base + 32 * SECTOR)).is_ok() && f.read_exact(&mut head).is_ok() && &head[..20] == MAGIC {
                let root = (u32::from_le_bytes(head[20..24].try_into().unwrap()), u32::from_le_bytes(head[24..28].try_into().unwrap()));
                return Ok(Disc { f, base, root });
            }
        }
        Err(format!("{}: not an Xbox 360 disc image", path.display()))
    }

    fn read_at(&mut self, at: u64, len: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; len];
        self.f.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        self.f.read_exact(&mut b).map_err(|_| "the disc image ends early (incomplete ISO?)".to_string())?;
        Ok(b)
    }

    /// (name, sector, size, is_dir) of one directory.
    fn entries(&mut self, sector: u32, size: u32) -> Result<Vec<(String, u32, u32, bool)>, String> {
        let table = self.read_at(self.base + sector as u64 * SECTOR, size as usize)?;
        let mut out = vec![];
        let mut todo = if size > 0 { vec![0usize] } else { vec![] };
        let mut seen = std::collections::HashSet::new();
        while let Some(at) = todo.pop() {
            if at + 14 > table.len() || !seen.insert(at) {
                continue;
            }
            let u16le = |o: usize| u16::from_le_bytes([table[o], table[o + 1]]);
            let u32le = |o: usize| u32::from_le_bytes(table[o..o + 4].try_into().unwrap());
            let (left, right) = (u16le(at), u16le(at + 2));
            if left == 0xFFFF {
                continue;
            }
            let (start, len, attr, n) = (u32le(at + 4), u32le(at + 8), table[at + 12], table[at + 13] as usize);
            let name = String::from_utf8_lossy(&table[at + 14..(at + 14 + n).min(table.len())]).into_owned();
            out.push((name, start, len, attr & 0x10 != 0));
            for o in [left, right] {
                if o != 0 {
                    todo.push(o as usize * 4);
                }
            }
        }
        Ok(out)
    }

    /// (sector, size) of a file, names matched case-insensitively like the console.
    pub fn find(&mut self, path: &str) -> Result<(u32, u32), String> {
        let mut at = self.root;
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i == parts.len() - 1;
            let hit = self
                .entries(at.0, at.1)?
                .into_iter()
                .find(|e| e.0.eq_ignore_ascii_case(part) && e.3 != last)
                .ok_or_else(|| format!("{path} isn't on this disc (is it Mirror's Edge?)"))?;
            at = (hit.1, hit.2);
        }
        Ok(at)
    }

    /// Every file on the disc: (path with '/', size).
    pub fn list(&mut self) -> Result<Vec<(String, u32)>, String> {
        let mut out = vec![];
        let mut todo = vec![(String::new(), self.root)];
        while let Some((prefix, (sector, size))) = todo.pop() {
            for (name, s, len, dir) in self.entries(sector, size)? {
                let p = format!("{prefix}{name}");
                if dir { todo.push((p + "/", (s, len))) } else { out.push((p, len)) }
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let (sector, size) = self.find(path)?;
        self.read_at(self.base + sector as u64 * SECTOR, size as usize)
    }

    /// `len` bytes at `offset` inside a file on the disc.
    pub fn read_range(&mut self, file: (u32, u32), offset: u64, len: usize) -> Result<Vec<u8>, String> {
        if offset + len as u64 > file.1 as u64 {
            return Err("a texture points past the end of its cache file".into());
        }
        self.read_at(self.base + file.0 as u64 * SECTOR + offset, len)
    }
}
