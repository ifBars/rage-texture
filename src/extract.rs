//! Texture extraction over the checked source session. No implicit fallback mips.
use crate::{
    Quality, Store,
    catalog::Texture,
    invalid,
    layout::{self, SurfaceKind, SurfaceLayout},
};
use std::io;

pub struct ExtractedTexture {
    pub dds: Vec<u8>,
    /// One provenance entry per contributing chunk read, in read order.
    /// A chunk may appear again in the bounded-memory fallback; this vector
    /// is not indexed by mip or slice. The coalesced path removes duplicates.
    pub quality: Vec<Quality>,
    /// Virtual source bytes requested, including tile/row padding. Distinct
    /// from packed physical source bytes and visible DDS payload bytes.
    pub input_bytes: u64,
    pub timings: ExtractionTimings,
}
#[derive(Clone, Copy, Default, serde::Serialize)]
pub struct ExtractionTimings {
    pub read_decode_seconds: f64,
    pub detile_seconds: f64,
    pub dds_seconds: f64,
}
fn chunk_pages(t: &Texture) -> io::Result<Vec<u32>> {
    if (t.extra >> 9) & 0x3ff != 0 {
        return Err(invalid("reserved streaming metadata bits set"));
    }
    let packed = ((t.extra as u64) << 11) | t.meta as u64;
    let mut pages = Vec::new();
    let mut ended = false;
    for i in 0..4 {
        let code = ((packed >> (5 * i)) & 31) as u32;
        if code == 0 {
            ended = true;
            continue;
        }
        if ended {
            return Err(invalid("streaming page codes after terminator"));
        }
        let n = match code {
            1..=8 => code,
            10 => 12,
            11 => 16,
            12 => 24,
            13 => 32,
            14 => 48,
            15 => 64,
            16 => 128,
            21 => 18,
            22 => 80,
            23 => 96,
            24 => 144,
            26 => 272,
            27 => 384,
            28 => 528,
            _ => return Err(invalid("unknown streaming page code")),
        };
        pages.push(n);
    }
    Ok(pages)
}
impl Store {
    /// Inspect the measured allocation layout for a descriptor from this source
    /// generation, including streaming cube page metadata and span validation.
    pub fn texture_layout(&mut self, t: &Texture) -> io::Result<SurfaceLayout> {
        self.validate_source()?;
        if t.generation != self.generation {
            return Err(invalid(
                "texture descriptor belongs to another source generation; reload catalog",
            ));
        }
        if t.base >= self.virtual_size || t.base == 0x1fffff * 4096 {
            return Err(invalid("texture allocation is not mapped"));
        }
        let pages = if t.dimension == 2 {
            Some(chunk_pages(t)?)
        } else {
            None
        };
        let layout = layout::layout_with_chunk_pages(&t.shape(), pages.as_deref())?;
        let end = t
            .base
            .checked_add(layout.total_bytes)
            .ok_or_else(|| invalid("texture extent overflow"))?;
        if end > self.virtual_size {
            return Err(invalid("texture allocation exceeds virtual source"));
        }
        if end != t.allocation_end {
            return Err(invalid(
                "computed layout differs from catalog allocation span",
            ));
        }
        Ok(layout)
    }
    /// Linear BC blocks or uncompressed texels for one visible mip/slice.
    pub fn texture_mip(
        &mut self,
        t: &Texture,
        mip: u32,
        slice: u32,
    ) -> io::Result<(Vec<u8>, Vec<Quality>)> {
        let layout = self.texture_layout(t)?;
        let m = layout.mip(mip)?;
        let offset = layout.subresource_offset(mip, slice)?;
        let length = usize::try_from(m.region).map_err(|_| invalid("mip size exceeds platform"))?;
        let (raw, quality) = self.read_range_inner(
            t.base
                .checked_add(offset)
                .ok_or_else(|| invalid("mip offset overflow"))?,
            length,
        )?;
        let bytes = layout::detile(&t.shape(), &layout, &raw, mip, slice)?;
        self.validate_source()?;
        Ok((bytes, quality))
    }
    /// Complete DX10 DDS, preserving source format and all visible mips/slices.
    pub fn texture_dds(&mut self, t: &Texture) -> io::Result<ExtractedTexture> {
        self.texture_dds_mips(t, t.mips)
    }
    /// Export the leading `mips` levels for every face/slice, using the full
    /// source layout for placement and the same bounded uncached input path.
    pub fn texture_dds_mips(&mut self, t: &Texture, mips: u32) -> io::Result<ExtractedTexture> {
        if mips == 0 || mips > t.mips {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DDS mip count is outside the source chain",
            ));
        }
        let layout = self.texture_layout(t)?;
        let shape = t.shape();
        let mut subresources = Vec::new();
        let mut quality = Vec::new();
        let mut input_bytes = 0u64;
        let mut timings = ExtractionTimings::default();
        let slices = if layout.kind == SurfaceKind::Volume3D {
            1
        } else {
            layout.slices
        };
        let mut output_bytes = 0usize;
        for mip in 0..mips {
            output_bytes = layout
                .subresource_len(mip)?
                .checked_mul(slices as usize)
                .and_then(|n| output_bytes.checked_add(n))
                .ok_or_else(|| invalid("DDS size overflow"))?;
        }
        if output_bytes > 256 * 1024 * 1024 {
            return Err(invalid("DDS output exceeds 256 MiB limit"));
        }
        // A bounded, operation-local input buffer avoids decompressing shared
        // mip tails/chunks again for each subresource. It is discarded after
        // this call and never changes the Store's decoded-cache budget.
        let mut input_start = u64::MAX;
        let mut input_end = 0;
        for slice in 0..slices {
            for mip in 0..mips {
                let offset = layout.subresource_offset(mip, slice)?;
                input_start = input_start.min(offset);
                input_end = input_end.max(
                    offset
                        .checked_add(layout.mip(mip)?.region)
                        .ok_or_else(|| invalid("texture input extent overflow"))?,
                );
            }
        }
        let input_len = input_end
            .checked_sub(input_start)
            .ok_or_else(|| invalid("texture has no subresources"))?;
        let input = if input_len <= self.limits.max_range_bytes as u64 {
            let timer = std::time::Instant::now();
            let (bytes, q) = self.read_range_inner(
                t.base
                    .checked_add(input_start)
                    .ok_or_else(|| invalid("texture input offset overflow"))?,
                input_len as usize,
            )?;
            timings.read_decode_seconds += timer.elapsed().as_secs_f64();
            input_bytes += input_len;
            quality.extend(q);
            Some(bytes)
        } else {
            None
        };
        for slice in 0..slices {
            for mip in 0..mips {
                let size = layout.subresource_len(mip)?;
                let mut linear = Vec::with_capacity(size);
                let zs = if layout.kind == SurfaceKind::Volume3D {
                    layout.mip(mip)?.pixels_d
                } else {
                    1
                };
                // Supported volumes have one mip: their whole region fits the
                // input buffer or exceeds max_range_bytes and is refused.
                for z in 0..zs {
                    let selected = if layout.kind == SurfaceKind::Volume3D {
                        z
                    } else {
                        slice
                    };
                    let m = layout.mip(mip)?;
                    let offset = layout.subresource_offset(mip, selected)?;
                    let timer = std::time::Instant::now();
                    let decoded;
                    let raw = if let Some(ref bytes) = input {
                        let start = usize::try_from(offset - input_start)
                            .map_err(|_| invalid("mip offset exceeds platform"))?;
                        let len = usize::try_from(m.region)
                            .map_err(|_| invalid("mip region exceeds platform"))?;
                        bytes
                            .get(start..start + len)
                            .ok_or_else(|| invalid("mip exceeds texture input"))?
                    } else {
                        let (bytes, q) = self.read_range_inner(
                            t.base
                                .checked_add(offset)
                                .ok_or_else(|| invalid("mip offset overflow"))?,
                            usize::try_from(m.region).map_err(|_| invalid("mip size overflow"))?,
                        )?;
                        quality.extend(q);
                        input_bytes += m.region;
                        decoded = bytes;
                        decoded.as_slice()
                    };
                    if input.is_none() {
                        timings.read_decode_seconds += timer.elapsed().as_secs_f64();
                    }
                    let timer = std::time::Instant::now();
                    linear.extend_from_slice(&layout::detile(&shape, &layout, raw, mip, selected)?);
                    timings.detile_seconds += timer.elapsed().as_secs_f64();
                }
                subresources.push(linear);
            }
        }
        let timer = std::time::Instant::now();
        let dds = crate::dds::write_dds(&shape, &subresources)?;
        timings.dds_seconds = timer.elapsed().as_secs_f64();
        self.validate_source()?;
        Ok(ExtractedTexture {
            dds,
            quality,
            input_bytes,
            timings,
        })
    }
    /// RGBA8 PNG preview. HDR is explicitly clamped; use DDS for original precision.
    pub fn texture_png(
        &mut self,
        t: &Texture,
        mip: u32,
        slice: u32,
    ) -> io::Result<(Vec<u8>, Vec<Quality>)> {
        let (linear, q) = self.texture_mip(t, mip, slice)?;
        Ok((
            crate::preview::png(
                t.dxgi,
                (t.width >> mip).max(1),
                (t.height >> mip).max(1),
                &linear,
            )?,
            q,
        ))
    }
}
