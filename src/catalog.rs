//! RDS5 embedded ETPF catalog. Descriptor field observations corroborated with
//! Leonida tools/tex_headers.py at aa2988cc; GPL-3.0-only combined project.
use crate::{Store, invalid, word};
use serde::Serialize;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Debug, Serialize)]
pub struct Texture {
    #[serde(skip)]
    pub(crate) generation: u64,
    #[serde(skip)]
    pub(crate) allocation_end: u64,
    pub index: usize,
    pub asset_key: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mips: u32,
    pub dxgi: u32,
    pub dimension: u32,
    pub mode: u32,
    pub non_tail: u32,
    pub base: u64,
    pub meta: u32,
    pub extra: u32,
}
impl Texture {
    pub fn shape(&self) -> crate::layout::TextureShape {
        crate::layout::TextureShape {
            width: self.width,
            height: self.height,
            depth: self.depth,
            mips: self.mips,
            dxgi: self.dxgi,
            dimension: self.dimension,
            mode: self.mode,
            non_tail: self.non_tail,
        }
    }
}
impl Store {
    pub fn textures(&mut self) -> io::Result<Vec<Texture>> {
        self.validate_source()?;
        let count = self.chunks.len() + 2;
        let etpf = (24 + (count - 1) * 8).div_ceil(256) * 256;
        let mut head = [0u8; 16];
        self.file.seek(SeekFrom::Start(etpf as u64))?;
        self.file.read_exact(&mut head)?;
        if &head[..4] != b"ETPF" || word(&head, 4) != 634 {
            return Err(invalid("unsupported ETPF version"));
        }
        let n = word(&head, 8) as usize;
        let area = word(&head, 12) as usize;
        if n == 0
            || n > 250_000
            || area
                < n.checked_mul(16)
                    .ok_or_else(|| invalid("catalog overflow"))?
            || area > self.limits.max_range_bytes
        {
            return Err(invalid("invalid ETPF record/key area"));
        }
        let boundary = self
            .chunks
            .first()
            .ok_or_else(|| invalid("no mapped data"))?
            .file_offset;
        if etpf as u64 + 16 + area as u64 > boundary {
            return Err(invalid("catalog overlaps chunk payload"));
        }
        let mut bytes = vec![0; area];
        self.file.read_exact(&mut bytes)?;
        let mut textures = Vec::with_capacity(n);
        for i in 0..n {
            let at = i * 16;
            let a = word(&bytes, at);
            let b = word(&bytes, at + 4);
            let c = word(&bytes, at + 8);
            let d = word(&bytes, at + 12);
            let key = bytes
                .get(n * 16 + i * 8..n * 16 + i * 8 + 8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap()));
            textures.push(Texture {
                generation: self.generation,
                allocation_end: 0,
                index: i,
                asset_key: key,
                width: (a & 0x3fff) + 1,
                height: ((a >> 14) & 0x3fff) + 1,
                depth: ((b >> 10) & 0x7ff) + 1,
                mips: (a >> 28) + (b & 7),
                dxgi: (b >> 3) & 127,
                dimension: (b >> 21) & 3,
                mode: b >> 23,
                non_tail: b & 7,
                base: (c as u64 & 0x1fffff) * 4096,
                meta: c >> 21,
                extra: d,
            });
        }
        let mut bases: Vec<_> = textures
            .iter()
            .map(|t| t.base)
            .filter(|&b| b < self.virtual_size && b != 0x1fffff * 4096)
            .collect();
        bases.sort_unstable();
        bases.dedup();
        for t in &mut textures {
            let next = bases.partition_point(|&base| base <= t.base);
            t.allocation_end = bases.get(next).copied().unwrap_or(self.virtual_size);
        }
        self.validate_source()?;
        Ok(textures)
    }
}
