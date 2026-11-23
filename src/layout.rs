//! Native Rust Gfx10 surface layout, detiling and DX10 DDS export.
//!
//! This is an attributed port of the layout and export rules from **Leonida**
//! (<https://github.com/ItsNotRexed/Leonida>, revision
//! `aa2988cc555bad7ecd508b48a702adbb32fe7c95`, GPL-3.0-or-later):
//!
//! - `tools/tex_headers.py`: `compact_surface_layout`, `array_surface_layout`,
//!   `cubemap_surface_layout`, `volume_surface_layout`, `gfx10_tile_mode`,
//!   `gfx10_tile_bytes`, the element-size/tile-origin tables and descriptor
//!   limits.
//! - `tools/ttpf_mips.py`: `detile`, `detile_volume_slice`, `_visible_rows`,
//!   `dds_bytes`/`texture_dds` (the DDS writer lives in `dds.rs`), and the
//!   AMD Gfx10/RDNA2 intra-tile bit patterns.
//!
//! The algorithm and its comments are adapted from those files. Upstream is
//! GPL-3.0-or-later; this combined project uses GPL-3.0-only and the
//! notices above must be retained.
//!
//! The layout rules are measured Gfx10 facts, not fitted guesses:
//! standalone mips are padded to whole tiles and stored smallest-first, and a
//! trailing group of small mips shares one tile at offset 0. Unsupported
//! shapes, formats, modes or data extents fail explicitly; nothing is
//! zero-filled or silently clamped.

use std::io;

const TILE_4K: u32 = 4096;
const TILE_64K: u32 = 65536;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn unsupported(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message.into())
}

fn malformed(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn checked_mul(a: u64, b: u64, what: &str) -> io::Result<u64> {
    a.checked_mul(b)
        .ok_or_else(|| invalid(format!("{what} overflows a 64-bit byte count")))
}

fn checked_add(a: u64, b: u64, what: &str) -> io::Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| invalid(format!("{what} overflows a 64-bit byte count")))
}

fn to_usize(value: u64, what: &str) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid(format!("{what} does not fit this platform")))
}

fn div_ceil_u64(value: u64, divisor: u64) -> u64 {
    value.div_ceil(divisor)
}

/// The 16-byte ETPF descriptor fields that determine a texture's surface.
///
/// `depth` is the array slice count, the cube face count (6) or the volume
/// depth. `mips` is the descriptor mip count (`packed_mips + non_tail`).
/// `dimension` is 1 for 2D/2D arrays, 2 for cubemaps and 3 for 3D volumes.
/// `mode` is the raw 9-bit descriptor mode: bit 0 is a flag and bits 1..5 are
/// the Gfx10 tile mode. `non_tail` is the cubemap streaming snapshot field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureShape {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mips: u32,
    pub dxgi: u32,
    pub dimension: u32,
    pub mode: u32,
    pub non_tail: u32,
}

/// Element size and block-compression class for a DXGI format.
///
/// For block-compressed formats `element_bytes` is one 4x4 block (8 or 16
/// bytes) and `block` is 4. Otherwise it is one texel and `block` is 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    pub element_bytes: u32,
    pub block: u32,
}

/// The four surface families the measured layout rules cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    /// `dimension == 1`, `depth == 1`.
    Texture2D,
    /// `dimension == 1`, `depth > 1`, mip-major `SW_64KB_D` (mode 1).
    Array2D,
    /// `dimension == 2`, `depth == 6`, slice-major `SW_4KB_S` snapshots.
    Cubemap,
    /// `dimension == 3`, single-mip `SW_4KB_S` (mode 2).
    Volume3D,
}

/// One mip level's placement in the allocation.
///
/// `offset`, `region` and `slice_stride` are byte counts. For arrays and
/// cubemaps, slice `s` starts at `offset + s * slice_stride`. `region` is the
/// storage region read for one slice (or, for a volume, the whole volume).
/// `blocks_w/h/d` are the storage grid extents. The packed tail's small mips
/// all have `offset == 0`, `region == 4096` and a nonzero `tile_x`/`tile_y`
/// origin within that shared 4 KiB tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLayout {
    pub index: u32,
    pub offset: u64,
    pub region: u64,
    pub slice_stride: u64,
    pub blocks_w: u32,
    pub blocks_h: u32,
    pub blocks_d: u32,
    pub padded_w: u32,
    pub padded_h: u32,
    pub pixels_w: u32,
    pub pixels_h: u32,
    pub pixels_d: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub in_tail: bool,
    pub tail_offset: Option<u32>,
}

/// A complete measured surface allocation.
///
/// `total_bytes` is the allocation span the descriptor should match. For a
/// streaming cubemap it is the sum of all snapshot chunks; `data_offset` is
/// where the final full-resolution six-face cubemap starts. `slice_size` is
/// one compact face (cubemap) or one compact array slice. `face_size` is the
/// cubemap face stride.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceLayout {
    pub total_bytes: u64,
    pub kind: SurfaceKind,
    pub tile: u32,
    pub element_bytes: u32,
    pub block: u32,
    pub first_tail: u32,
    pub block_w: u32,
    pub block_h: u32,
    pub block_d: u32,
    pub levels: u32,
    pub depth: u32,
    pub slices: u32,
    pub data_offset: u64,
    pub face_size: u64,
    pub slice_size: u64,
    pub padded_full_bytes: u64,
    pub padded_d: u32,
    pub z_tile_stride: u64,
    pub snapshots: u32,
    pub mips: Vec<MipLayout>,
}

impl SurfaceLayout {
    pub fn mip(&self, index: u32) -> io::Result<&MipLayout> {
        self.mips
            .get(index as usize)
            .ok_or_else(|| invalid(format!("mip {index} is outside the layout")))
    }

    /// Number of vectors [`crate::dds::write_dds`] expects: slice-major then
    /// mip. A 3D volume has one vector per mip, each holding all Z slices.
    pub fn subresource_count(&self) -> io::Result<usize> {
        if self.kind == SurfaceKind::Volume3D {
            Ok(self.mips.len())
        } else {
            self.mips
                .len()
                .checked_mul(self.slices as usize)
                .ok_or_else(|| invalid("subresource count overflows this platform"))
        }
    }

    /// Tightly packed visible bytes of one 2D slice, cube face or volume
    /// Z slice at `mip`. BC formats keep whole 4x4 blocks; odd mip sizes are
    /// cropped to the blocks that hold visible texels.
    pub fn slice_len(&self, mip: u32) -> io::Result<usize> {
        let m = self.mip(mip)?;
        if self.block == 0 || self.element_bytes == 0 {
            return Err(malformed("layout has a zero element or block size"));
        }
        let units = if self.kind == SurfaceKind::Volume3D {
            checked_mul(m.pixels_w as u64, m.pixels_h as u64, "volume slice")?
        } else {
            let w = div_ceil_u64(m.pixels_w as u64, self.block as u64);
            let h = div_ceil_u64(m.pixels_h as u64, self.block as u64);
            checked_mul(w, h, "block grid")?
        };
        let bytes = checked_mul(units, self.element_bytes as u64, "visible payload")?;
        to_usize(bytes, "visible payload")
    }

    /// Bytes of one DDS subresource: one slice for 2D/array/cubemap, or the
    /// whole depth for a volume mip.
    pub fn subresource_len(&self, mip: u32) -> io::Result<usize> {
        let slice = self.slice_len(mip)?;
        if self.kind == SurfaceKind::Volume3D {
            let depth = self.mip(mip)?.pixels_d as usize;
            slice
                .checked_mul(depth)
                .ok_or_else(|| invalid("volume subresource overflows this platform"))
        } else {
            Ok(slice)
        }
    }

    /// Byte offset of a slice's storage region inside the allocation. For a
    /// 3D volume the whole mip is read at once; `slice` selects a Z index for
    /// [`detile`] and this returns the mip's base offset.
    pub fn subresource_offset(&self, mip: u32, slice: u32) -> io::Result<u64> {
        let m = self.mip(mip)?;
        if slice >= self.slices {
            return Err(invalid(format!(
                "slice {slice} is outside the {}-slice layout",
                self.slices
            )));
        }
        if self.kind == SurfaceKind::Volume3D {
            if slice >= m.pixels_d {
                return Err(invalid(format!(
                    "volume Z slice {slice} is outside depth {}",
                    m.pixels_d
                )));
            }
            Ok(m.offset)
        } else {
            let stride = checked_mul(slice as u64, m.slice_stride, "slice offset")?;
            let face = checked_add(m.offset, stride, "slice offset")?;
            // Streaming cubemaps place the final full-resolution snapshot after
            // earlier snapshot chunks; every other family starts at byte 0.
            checked_add(self.data_offset, face, "slice offset")
        }
    }
}

/// DXGI element size and block dimension, matching Leonida's `ELEMENT_BYTES`
/// plus the block-compressed typeless IDs. Returns `None` for unknown IDs.
pub fn format_info(dxgi: u32) -> Option<FormatInfo> {
    let (element_bytes, block) = match dxgi {
        70..=72 | 79..=81 => (8, 4),
        73..=78 | 82..=84 | 94..=99 => (16, 4),
        2..=4 => (16, 1),
        6..=8 => (12, 1),
        10..=14 | 16..=18 => (8, 1),
        24..=26 | 28..=32 | 34..=38 | 41..=43 | 87 | 88 | 91 | 93 => (4, 1),
        49..=52 | 54 | 56..=59 => (2, 1),
        61..=65 => (1, 1),
        _ => return None,
    };
    Some(FormatInfo {
        element_bytes,
        block,
    })
}

fn element_log(element_bytes: u32) -> Option<u32> {
    match element_bytes {
        1 => Some(0),
        2 => Some(1),
        4 => Some(2),
        8 => Some(3),
        16 => Some(4),
        _ => None,
    }
}

/// The 4-bit Gfx10 tile mode packed at bit 1 of the descriptor mode.
pub fn gfx10_tile_mode(mode: u32) -> u32 {
    (mode >> 1) & 0xF
}

/// Tile size for a descriptor mode: 64 KiB only for Gfx10 tile mode 0.
pub fn gfx10_tile_bytes(mode: u32) -> u32 {
    if gfx10_tile_mode(mode) == 0 {
        TILE_64K
    } else {
        TILE_4K
    }
}

struct CompactLayout {
    size: u64,
    tile: u32,
    first_tail: u32,
    block_w: u32,
    block_h: u32,
    levels: u32,
    mips: Vec<MipLayout>,
}

/// AMD Gfx10 compact mip chain for one 2D surface (adapted from
/// `compact_surface_layout`).
fn compact_surface_layout(
    width: u32,
    height: u32,
    mips: u32,
    element_bytes: u32,
    block: u32,
    tile: u32,
) -> io::Result<CompactLayout> {
    if width == 0 || height == 0 {
        return Err(invalid(format!(
            "invalid surface dimensions {width}x{height}"
        )));
    }
    let log = element_log(element_bytes).ok_or_else(|| {
        unsupported(format!(
            "unsupported compact element size: {element_bytes} bytes"
        ))
    })?;
    if block != 1 && block != 4 {
        return Err(unsupported(format!("unsupported block dimension: {block}")));
    }
    if tile != TILE_4K && tile != TILE_64K {
        return Err(unsupported(format!(
            "unsupported compact tile size: {tile}"
        )));
    }
    let levels = 32 - width.max(height).leading_zeros();
    if mips < 1 || mips > levels {
        return Err(invalid(format!(
            "invalid mip count {mips} for {width}x{height} (maximum {levels})"
        )));
    }

    let half_bits = if tile == TILE_4K { 6 } else { 8 };
    let max_tail = if tile == TILE_4K { 8 } else { 12 };
    let block_w = 1u32 << (half_bits - log / 2);
    let block_h = 1u32 << (half_bits - log.div_ceil(2));
    let (origin_w, origin_h) = match log {
        0 => (16, 16),
        1 => (16, 8),
        2 => (8, 8),
        3 => (8, 4),
        _ => (4, 4),
    };
    let ew = div_ceil_u64(width as u64, block as u64);
    let eh = div_ceil_u64(height as u64, block as u64);

    let mut tail = mips;
    let mut sizes: Vec<u64> = Vec::new();
    let mut out: Vec<MipLayout> = Vec::with_capacity(mips as usize);
    for i in 0..mips {
        let scale = 1u64 << i;
        let wc = div_ceil_u64(ew, scale);
        let hc = div_ceil_u64(eh, scale);
        if mips > 1 && wc <= (block_w / 2) as u64 && hc <= block_h as u64 && mips - i <= max_tail {
            tail = i;
            break;
        }
        let pw = div_ceil_u64(wc, block_w as u64) * block_w as u64;
        let ph = div_ceil_u64(hc, block_h as u64) * block_h as u64;
        let region = checked_mul(
            checked_mul(pw, ph, "compact mip")?,
            element_bytes as u64,
            "compact mip",
        )?;
        sizes.push(region);
        out.push(MipLayout {
            index: i,
            offset: 0,
            region,
            slice_stride: 0,
            blocks_w: u32::try_from(wc).map_err(|_| invalid("mip width overflow"))?,
            blocks_h: u32::try_from(hc).map_err(|_| invalid("mip height overflow"))?,
            blocks_d: 1,
            padded_w: u32::try_from(pw).map_err(|_| invalid("padded width overflow"))?,
            padded_h: u32::try_from(ph).map_err(|_| invalid("padded height overflow"))?,
            pixels_w: (width >> i).max(1),
            pixels_h: (height >> i).max(1),
            pixels_d: 1,
            tile_x: 0,
            tile_y: 0,
            in_tail: false,
            tail_offset: None,
        });
    }

    let mut offset = if tail < mips { tile as u64 } else { 0 };
    for i in (0..tail).rev() {
        out[i as usize].offset = offset;
        offset = checked_add(offset, sizes[i as usize], "compact mip chain")?;
    }
    for i in tail..mips {
        // AMD packed-tail entry offset: mips are placed largest first inside
        // the shared tile at the row/column origin encoded by `moff`.
        let entry = max_tail - 1 - (i - tail);
        let moff = if entry > 6 {
            16u32 << entry
        } else {
            entry << 8
        };
        let mut x = 0u32;
        let mut y = 0u32;
        for bit in 0..6 {
            x |= ((moff >> (9 + 2 * bit)) & 1) << bit;
            y |= ((moff >> (8 + 2 * bit)) & 1) << bit;
        }
        let wc = div_ceil_u64(ew, 1u64 << i);
        let hc = div_ceil_u64(eh, 1u64 << i);
        out.push(MipLayout {
            index: i,
            offset: 0,
            region: tile as u64,
            slice_stride: 0,
            blocks_w: u32::try_from(wc).map_err(|_| invalid("tail mip width overflow"))?,
            blocks_h: u32::try_from(hc).map_err(|_| invalid("tail mip height overflow"))?,
            blocks_d: 1,
            padded_w: block_w,
            padded_h: block_h,
            pixels_w: (width >> i).max(1),
            pixels_h: (height >> i).max(1),
            pixels_d: 1,
            tile_x: x * origin_w,
            tile_y: y * origin_h,
            in_tail: true,
            tail_offset: Some(moff),
        });
    }

    Ok(CompactLayout {
        size: offset,
        tile,
        first_tail: tail,
        block_w,
        block_h,
        levels,
        mips: out,
    })
}

fn format_for(shape: &TextureShape) -> io::Result<FormatInfo> {
    format_info(shape.dxgi)
        .ok_or_else(|| unsupported(format!("unsupported or unknown DXGI format {}", shape.dxgi)))
}

fn layout_2d(shape: &TextureShape) -> io::Result<SurfaceLayout> {
    if shape.depth != 1 {
        return Err(invalid(format!(
            "a 2D texture needs depth 1, got {}",
            shape.depth
        )));
    }
    let info = format_for(shape)?;
    let tile = gfx10_tile_bytes(shape.mode);
    let compact = compact_surface_layout(
        shape.width,
        shape.height,
        shape.mips,
        info.element_bytes,
        info.block,
        tile,
    )?;
    Ok(SurfaceLayout {
        total_bytes: compact.size,
        kind: SurfaceKind::Texture2D,
        tile: compact.tile,
        element_bytes: info.element_bytes,
        block: info.block,
        first_tail: compact.first_tail,
        block_w: compact.block_w,
        block_h: compact.block_h,
        block_d: 1,
        levels: compact.levels,
        depth: 1,
        slices: 1,
        data_offset: 0,
        face_size: 0,
        slice_size: 0,
        padded_full_bytes: 0,
        padded_d: 1,
        z_tile_stride: 0,
        snapshots: 1,
        mips: compact.mips,
    })
}

fn layout_array(shape: &TextureShape) -> io::Result<SurfaceLayout> {
    if shape.mode != 1 {
        return Err(unsupported(format!(
            "unsupported 2D array mode {} (measured arrays use mode 1)",
            shape.mode
        )));
    }
    if shape.depth < 2 {
        return Err(invalid("a 2D array needs at least two slices"));
    }
    let info = format_for(shape)?;
    let tile = gfx10_tile_bytes(shape.mode);
    let compact = compact_surface_layout(
        shape.width,
        shape.height,
        shape.mips,
        info.element_bytes,
        info.block,
        tile,
    )?;
    let depth = shape.depth as u64;
    let mut mips = compact.mips;
    let mut offset = if compact.first_tail < shape.mips {
        checked_mul(depth, tile as u64, "array tail")?
    } else {
        0
    };
    for i in (0..compact.first_tail).rev() {
        mips[i as usize].offset = offset;
        mips[i as usize].slice_stride = mips[i as usize].region;
        offset = checked_add(
            offset,
            checked_mul(depth, mips[i as usize].region, "array mip")?,
            "array mip chain",
        )?;
    }
    for i in compact.first_tail..shape.mips {
        mips[i as usize].offset = 0;
        mips[i as usize].slice_stride = tile as u64;
    }
    Ok(SurfaceLayout {
        total_bytes: offset,
        kind: SurfaceKind::Array2D,
        tile: compact.tile,
        element_bytes: info.element_bytes,
        block: info.block,
        first_tail: compact.first_tail,
        block_w: compact.block_w,
        block_h: compact.block_h,
        block_d: 1,
        levels: compact.levels,
        depth: shape.depth,
        slices: shape.depth,
        data_offset: 0,
        face_size: 0,
        slice_size: compact.size,
        padded_full_bytes: 0,
        padded_d: 1,
        z_tile_stride: 0,
        snapshots: 1,
        mips,
    })
}

fn layout_cubemap(shape: &TextureShape, chunk_pages: Option<&[u32]>) -> io::Result<SurfaceLayout> {
    if shape.depth != 6 {
        return Err(unsupported(format!(
            "unsupported cubemap depth {} (expected 6)",
            shape.depth
        )));
    }
    if shape.non_tail > 3 {
        return Err(unsupported(format!(
            "unsupported cubemap non_tail {}",
            shape.non_tail
        )));
    }
    let expected_mode = 18 + 2 * shape.non_tail;
    if shape.mode != expected_mode {
        return Err(unsupported(format!(
            "unsupported cubemap mode {} (non_tail {} needs mode {expected_mode})",
            shape.mode, shape.non_tail
        )));
    }
    let info = format_for(shape)?;
    // Cubemap faces are stored slice-major in SW_4KB_S regardless of the
    // descriptor mode's tile family.
    let compact = compact_surface_layout(
        shape.width,
        shape.height,
        shape.mips,
        info.element_bytes,
        info.block,
        TILE_4K,
    )?;
    let face_size = compact.size;
    let full_bytes = checked_mul(6, face_size, "cubemap full size")?;
    let padded_full = if shape.non_tail == 0 {
        full_bytes
    } else if full_bytes >= 36 * TILE_4K as u64 {
        div_ceil_u64(full_bytes, TILE_64K as u64) * TILE_64K as u64
    } else {
        full_bytes
    };

    let (data_offset, total_bytes) = match chunk_pages {
        Some(pages) if shape.non_tail == 0 => {
            if !pages.is_empty()
                && (pages.len() != 1
                    || checked_mul(pages[0] as u64, TILE_4K as u64, "cubemap chunk")?
                        != padded_full)
            {
                return Err(invalid(format!(
                    "non-streaming cubemap chunk_pages {pages:?} do not match one \
                     {padded_full}-byte chunk"
                )));
            }
            (0, padded_full)
        }
        Some(pages) => {
            let expected = shape.non_tail as usize + 1;
            if pages.len() != expected {
                return Err(invalid(format!(
                    "cubemap chunk count mismatch: expected {expected}, got {}",
                    pages.len()
                )));
            }
            let last = checked_mul(pages[expected - 1] as u64, TILE_4K as u64, "cubemap chunk")?;
            if last != padded_full {
                return Err(invalid(format!(
                    "cubemap final chunk size {last} != expected {padded_full}"
                )));
            }
            let mut earlier = 0u64;
            for page in &pages[..expected - 1] {
                earlier = checked_add(
                    earlier,
                    checked_mul(*page as u64, TILE_4K as u64, "cubemap chunk")?,
                    "cubemap chunks",
                )?;
            }
            (earlier, checked_add(earlier, last, "cubemap allocation")?)
        }
        None => (0, padded_full),
    };

    let mut mips = compact.mips;
    for mip in &mut mips {
        mip.slice_stride = face_size;
    }
    Ok(SurfaceLayout {
        total_bytes,
        kind: SurfaceKind::Cubemap,
        tile: TILE_4K,
        element_bytes: info.element_bytes,
        block: info.block,
        first_tail: compact.first_tail,
        block_w: compact.block_w,
        block_h: compact.block_h,
        block_d: 1,
        levels: compact.levels,
        depth: 6,
        slices: 6,
        data_offset,
        face_size,
        slice_size: face_size,
        padded_full_bytes: padded_full,
        padded_d: 1,
        z_tile_stride: 0,
        snapshots: shape.non_tail + 1,
        mips,
    })
}

fn layout_volume(shape: &TextureShape) -> io::Result<SurfaceLayout> {
    if shape.mode != 2 {
        return Err(unsupported(format!(
            "unsupported 3D volume mode {} (measured volumes use mode 2)",
            shape.mode
        )));
    }
    if shape.mips != 1 {
        return Err(unsupported(format!(
            "only single-mip 3D volumes are validated, got mips={}",
            shape.mips
        )));
    }
    if shape.depth == 0 {
        return Err(invalid("a 3D volume needs a nonzero depth"));
    }
    let info = format_for(shape)?;
    if info.block != 1 {
        return Err(unsupported(format!(
            "block-compressed 3D volumes are not supported: format {}",
            shape.dxgi
        )));
    }
    let log = element_log(info.element_bytes).ok_or_else(|| {
        unsupported(format!(
            "unsupported volume element size: {} bytes",
            info.element_bytes
        ))
    })?;
    // AMD Gfx10 3D SW_4KB_S micro-tile extents (blocks_w, blocks_h, blocks_d).
    let (bw, bh, bd) = match log {
        0 => (16, 16, 16),
        1 => (16, 8, 16),
        2 => (8, 8, 16),
        3 => (8, 8, 8),
        _ => (8, 4, 8),
    };
    let pw = div_ceil_u64(shape.width as u64, bw as u64) * bw as u64;
    let ph = div_ceil_u64(shape.height as u64, bh as u64) * bh as u64;
    let pd = div_ceil_u64(shape.depth as u64, bd as u64) * bd as u64;
    let z_tile_stride = checked_mul(
        checked_mul(pw / bw as u64, ph / bh as u64, "volume tile grid")?,
        TILE_4K as u64,
        "volume tile stride",
    )?;
    let size = checked_mul(pd / bd as u64, z_tile_stride, "volume size")?;
    let levels = 32
        - shape
            .width
            .max(shape.height)
            .max(shape.depth)
            .leading_zeros();
    let mip = MipLayout {
        index: 0,
        offset: 0,
        region: size,
        slice_stride: 0,
        blocks_w: shape.width,
        blocks_h: shape.height,
        blocks_d: shape.depth,
        padded_w: u32::try_from(pw).map_err(|_| invalid("volume padded width overflow"))?,
        padded_h: u32::try_from(ph).map_err(|_| invalid("volume padded height overflow"))?,
        pixels_w: shape.width,
        pixels_h: shape.height,
        pixels_d: shape.depth,
        tile_x: 0,
        tile_y: 0,
        in_tail: false,
        tail_offset: None,
    };
    Ok(SurfaceLayout {
        total_bytes: size,
        kind: SurfaceKind::Volume3D,
        tile: TILE_4K,
        element_bytes: info.element_bytes,
        block: 1,
        first_tail: 1,
        block_w: bw,
        block_h: bh,
        block_d: bd,
        levels,
        depth: shape.depth,
        slices: shape.depth,
        data_offset: 0,
        face_size: 0,
        slice_size: 0,
        padded_full_bytes: 0,
        padded_d: u32::try_from(pd).map_err(|_| invalid("volume padded depth overflow"))?,
        z_tile_stride,
        snapshots: 1,
        mips: vec![mip],
    })
}

/// Compute the measured Gfx10 surface layout for `shape`.
///
/// For a streaming cubemap (`non_tail > 0`) the caller should use
/// [`layout_with_chunk_pages`] with the descriptor's decoded 4 KiB chunk page
/// counts so `total_bytes` and `data_offset` describe the real allocation.
/// Without them, this reports the final full-resolution snapshot only.
pub fn layout(shape: &TextureShape) -> io::Result<SurfaceLayout> {
    layout_with_chunk_pages(shape, None)
}

/// Like [`layout`], but validates cubemap streaming chunk page counts.
///
/// `chunk_pages` are the 4 KiB page counts from the descriptor's 43-bit
/// `(extra << 11) | meta` field (see Leonida `decode_etpf_chunk_pages`). They
/// are ignored for non-cubemap shapes.
pub fn layout_with_chunk_pages(
    shape: &TextureShape,
    chunk_pages: Option<&[u32]>,
) -> io::Result<SurfaceLayout> {
    if shape.width == 0 || shape.height == 0 {
        return Err(invalid(format!(
            "invalid surface dimensions {}x{}",
            shape.width, shape.height
        )));
    }
    match shape.dimension {
        1 if shape.depth == 1 => layout_2d(shape),
        1 if shape.depth > 1 => layout_array(shape),
        1 => Err(invalid("depth must be at least 1")),
        2 => layout_cubemap(shape, chunk_pages),
        3 => layout_volume(shape),
        other => Err(unsupported(format!(
            "unsupported descriptor dimension {other}"
        ))),
    }
}

const PATTERNS_4K: [[i32; 12]; 5] = [
    [1, 2, 3, 4, -1, -2, -3, -4, -5, 5, -6, 6],
    [0, 1, 2, 3, -1, -2, -3, 4, -4, 5, -5, 6],
    [0, 0, 1, 2, -1, -2, -3, 3, -4, 4, -5, 5],
    [0, 0, 0, 1, -1, -2, 2, 3, -3, 4, -4, 5],
    [0, 0, 0, 0, -1, -2, 1, 2, -3, 3, -4, 4],
];

const PATTERNS_64K: [[i32; 16]; 5] = [
    [1, 2, 3, -2, -1, -3, 4, -4, -5, 5, -6, 6, -7, 7, -8, 8],
    [0, 1, 2, 3, -1, -2, -3, 4, -4, 5, -5, 6, -6, 7, -7, 8],
    // RDNA2/RbPlus SW_64KB_D swaps bits 6..7 vs Navi 1x.
    [0, 0, 1, 2, -1, -2, 3, -3, -4, 4, -5, 5, -6, 6, -7, 7],
    [0, 0, 0, 1, -1, 2, 3, -2, -3, 4, -4, 5, -5, 6, -6, 7],
    [0, 0, 0, 0, 1, -1, 2, -2, -3, 3, -4, 4, -5, 5, -6, 6],
];

// AMD Gfx10 3D SW_4KB_S intra-tile bit order: 0 < s < 100 is an X bit
// (s-1), s < 0 is a Y bit (-s-1), and s >= 101 is a Z bit (s-101).
const VOLUME_PATTERNS_4K: [[i32; 12]; 5] = [
    [1, 2, 101, -1, 102, -2, 3, 103, -3, 4, 104, -4],
    [0, 1, 2, 101, -1, 102, 3, -2, 103, 4, -3, 104],
    [0, 0, 1, 2, 101, -1, 102, -2, 3, 103, -3, 104],
    [0, 0, 0, 1, 101, -1, 2, 102, -2, 3, 103, -3],
    [0, 0, 0, 0, 1, -1, 101, 2, -2, 102, 3, 103],
];

fn validate_shape_layout(shape: &TextureShape, layout: &SurfaceLayout) -> io::Result<()> {
    if layout.mips.len() != shape.mips as usize {
        return Err(malformed(format!(
            "layout has {} mips but shape says {}",
            layout.mips.len(),
            shape.mips
        )));
    }
    let info = format_for(shape)?;
    if layout.element_bytes != info.element_bytes || layout.block != info.block {
        return Err(malformed(format!(
            "layout element {}/{} does not match DXGI {} element {}/{}",
            layout.element_bytes, layout.block, shape.dxgi, info.element_bytes, info.block
        )));
    }
    let expected = match (shape.dimension, shape.depth) {
        (1, 1) => SurfaceKind::Texture2D,
        (1, _) => SurfaceKind::Array2D,
        (2, _) => SurfaceKind::Cubemap,
        (3, _) => SurfaceKind::Volume3D,
        _ => return Err(unsupported("unsupported descriptor dimension")),
    };
    if layout.kind != expected {
        return Err(malformed(format!(
            "layout kind {:?} does not match shape {:?}",
            layout.kind, expected
        )));
    }
    Ok(())
}

fn pattern_2d(tile: u32, element_bytes: u32) -> io::Result<&'static [i32]> {
    let log = element_log(element_bytes)
        .ok_or_else(|| unsupported(format!("unsupported element size: {element_bytes}")))?;
    match tile {
        TILE_4K => Ok(&PATTERNS_4K[log as usize]),
        TILE_64K => Ok(&PATTERNS_64K[log as usize]),
        other => Err(unsupported(format!(
            "unsupported compact tile size: {other}"
        ))),
    }
}

fn detile_2d(
    layout: &SurfaceLayout,
    m: &MipLayout,
    data: &[u8],
    slice: u32,
) -> io::Result<Vec<u8>> {
    if slice >= layout.slices {
        return Err(invalid(format!(
            "slice {slice} is outside the {}-slice layout",
            layout.slices
        )));
    }
    let bw = layout.block_w;
    let bh = layout.block_h;
    if bw == 0 || bh == 0 || m.padded_w == 0 || m.padded_h == 0 {
        return Err(malformed("layout has a zero tile or padded extent"));
    }
    if !m.padded_w.is_multiple_of(bw) || !m.padded_h.is_multiple_of(bh) {
        return Err(malformed("padded extent is not a whole number of tiles"));
    }
    if m.blocks_w > m.padded_w || m.blocks_h > m.padded_h {
        return Err(malformed("storage grid exceeds its padded extent"));
    }
    let pattern = pattern_2d(layout.tile, layout.element_bytes)?;
    let element_bytes = layout.element_bytes as usize;
    let region = to_usize(m.region, "mip region")?;
    if data.len() < region {
        return Err(malformed(format!(
            "mip {} needs {region} tiled bytes, got {}",
            m.index,
            data.len()
        )));
    }
    let out_len = (m.blocks_w as usize)
        .checked_mul(m.blocks_h as usize)
        .and_then(|n| n.checked_mul(element_bytes))
        .ok_or_else(|| invalid("detile output overflows this platform"))?;
    let mut out = vec![0u8; out_len];
    let tiles_x = (m.padded_w / bw) as u64;
    let tile = layout.tile as u64;
    for y in 0..m.blocks_h {
        for x in 0..m.blocks_w {
            let mut offset = ((y / bh) as u64 * tiles_x + (x / bw) as u64) * tile;
            for (bit, source) in pattern.iter().copied().enumerate() {
                if source > 0 {
                    offset |= (((x as u64 + m.tile_x as u64) >> (source - 1)) & 1) << bit;
                } else if source < 0 {
                    offset |= (((y as u64 + m.tile_y as u64) >> (-source - 1)) & 1) << bit;
                }
            }
            let end = offset
                .checked_add(element_bytes as u64)
                .ok_or_else(|| malformed("tiled offset overflow"))?;
            if end > m.region {
                return Err(malformed("tiled element outside the mip region"));
            }
            let at = (y as usize * m.blocks_w as usize + x as usize) * element_bytes;
            out[at..at + element_bytes].copy_from_slice(&data[offset as usize..end as usize]);
        }
    }
    Ok(out)
}

fn detile_volume_slice(
    layout: &SurfaceLayout,
    m: &MipLayout,
    data: &[u8],
    slice: u32,
) -> io::Result<Vec<u8>> {
    if slice >= m.pixels_d {
        return Err(invalid(format!(
            "Z slice {slice} is outside depth {}",
            m.pixels_d
        )));
    }
    let bw = layout.block_w;
    let bh = layout.block_h;
    let bd = layout.block_d;
    if bw == 0 || bh == 0 || bd == 0 || m.padded_w == 0 || m.padded_h == 0 {
        return Err(malformed("volume layout has a zero micro-tile extent"));
    }
    if !m.padded_w.is_multiple_of(bw) || !m.padded_h.is_multiple_of(bh) {
        return Err(malformed(
            "volume padded extent is not a whole number of tiles",
        ));
    }
    let log = element_log(layout.element_bytes).ok_or_else(|| {
        unsupported(format!(
            "unsupported element size: {}",
            layout.element_bytes
        ))
    })?;
    let pattern = &VOLUME_PATTERNS_4K[log as usize];
    let element_bytes = layout.element_bytes as usize;
    let region = to_usize(m.region, "volume region")?;
    if data.len() < region {
        return Err(malformed(format!(
            "volume needs {region} tiled bytes, got {}",
            data.len()
        )));
    }
    let out_len = (m.blocks_w as usize)
        .checked_mul(m.blocks_h as usize)
        .and_then(|n| n.checked_mul(element_bytes))
        .ok_or_else(|| invalid("volume detile output overflows this platform"))?;
    let tiles_x = m.padded_w / bw;
    let z_base = (slice / bd) as u64 * layout.z_tile_stride;
    let mut z_in_tile = 0u32;
    for (bit, source) in pattern.iter().copied().enumerate() {
        if source >= 101 {
            z_in_tile |= ((slice >> (source - 101)) & 1) << bit;
        }
    }
    let mut out = vec![0u8; out_len];
    for y in 0..m.blocks_h {
        let y_tile = (y / bh) as u64 * tiles_x as u64;
        let mut y_in_tile = z_in_tile;
        for (bit, source) in pattern.iter().copied().enumerate() {
            if source < 0 {
                y_in_tile |= ((y >> (-source - 1)) & 1) << bit;
            }
        }
        let row_at = y as usize * m.blocks_w as usize * element_bytes;
        for x in 0..m.blocks_w {
            let mut offset =
                z_base + (y_tile + (x / bw) as u64) * layout.tile as u64 + y_in_tile as u64;
            for (bit, source) in pattern.iter().copied().enumerate() {
                if source > 0 && source < 100 {
                    offset |= (((x >> (source - 1)) & 1) as u64) << bit;
                }
            }
            let end = offset
                .checked_add(element_bytes as u64)
                .ok_or_else(|| malformed("volume offset overflow"))?;
            if end > m.region {
                return Err(malformed("volume element outside the mip region"));
            }
            let at = row_at + x as usize * element_bytes;
            out[at..at + element_bytes].copy_from_slice(&data[offset as usize..end as usize]);
        }
    }
    Ok(out)
}

/// Detile one mip/slice out of its tiled storage without cropping.
///
/// `data` must start at [`SurfaceLayout::subresource_offset`] for
/// `(mip, slice)` and hold at least the mip's `region` bytes. The result is
/// the storage block grid (`blocks_w * blocks_h` elements). For a 3D volume
/// it is the selected Z slice in row-major order.
pub fn detile_storage(
    shape: &TextureShape,
    layout: &SurfaceLayout,
    data: &[u8],
    mip: u32,
    slice: u32,
) -> io::Result<Vec<u8>> {
    validate_shape_layout(shape, layout)?;
    let m = layout.mip(mip)?;
    match layout.kind {
        SurfaceKind::Volume3D => detile_volume_slice(layout, m, data, slice),
        _ => detile_2d(layout, m, data, slice),
    }
}

fn crop_visible(
    bytes: Vec<u8>,
    m: &MipLayout,
    block: u32,
    element_bytes: u32,
) -> io::Result<Vec<u8>> {
    if block == 0 || element_bytes == 0 {
        return Err(malformed("layout has a zero block or element size"));
    }
    let visible_w = div_ceil_u64(m.pixels_w as u64, block as u64);
    let visible_h = div_ceil_u64(m.pixels_h as u64, block as u64);
    if visible_w > m.blocks_w as u64 || visible_h > m.blocks_h as u64 {
        return Err(malformed("visible grid exceeds the stored block grid"));
    }
    if visible_w == m.blocks_w as u64 && visible_h == m.blocks_h as u64 {
        return Ok(bytes);
    }
    let row = (m.blocks_w as usize)
        .checked_mul(element_bytes as usize)
        .ok_or_else(|| invalid("crop row overflows this platform"))?;
    let keep = (visible_w as usize)
        .checked_mul(element_bytes as usize)
        .ok_or_else(|| invalid("crop row overflows this platform"))?;
    let mut out = Vec::with_capacity(visible_h as usize * keep);
    for y in 0..visible_h as usize {
        let at = y
            .checked_mul(row)
            .ok_or_else(|| invalid("crop row overflows this platform"))?;
        out.extend_from_slice(&bytes[at..at + keep]);
    }
    Ok(out)
}

/// Detile one mip/slice into the tightly packed linear bytes a DDS expects.
///
/// BC formats return whole 4x4 blocks; rows are cropped to the blocks that
/// hold visible texels, matching Leonida's `_visible_rows`. `data` is the
/// storage region for `(mip, slice)` (see [`detile_storage`]). For a volume,
/// `slice` is the Z index and the returned bytes are that slice's row-major
/// elements.
pub fn detile(
    shape: &TextureShape,
    layout: &SurfaceLayout,
    data: &[u8],
    mip: u32,
    slice: u32,
) -> io::Result<Vec<u8>> {
    validate_shape_layout(shape, layout)?;
    let m = layout.mip(mip)?;
    let full = detile_storage(shape, layout, data, mip, slice)?;
    crop_visible(full, m, layout.block, layout.element_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_table_matches_measured_sizes() {
        assert_eq!(
            format_info(71),
            Some(FormatInfo {
                element_bytes: 8,
                block: 4
            })
        );
        assert_eq!(
            format_info(98),
            Some(FormatInfo {
                element_bytes: 16,
                block: 4
            })
        );
        assert_eq!(
            format_info(11),
            Some(FormatInfo {
                element_bytes: 8,
                block: 1
            })
        );
        assert_eq!(
            format_info(65),
            Some(FormatInfo {
                element_bytes: 1,
                block: 1
            })
        );
        assert_eq!(format_info(999), None);
    }

    #[test]
    fn two_dimensional_layout_tail_and_offsets() {
        let shape = TextureShape {
            width: 1024,
            height: 1024,
            depth: 1,
            mips: 9,
            dxgi: 71,
            dimension: 1,
            mode: 2,
            non_tail: 0,
        };
        let layout = layout(&shape).unwrap();
        assert_eq!(layout.total_bytes, 700416);
        assert_eq!(layout.first_tail, 4);
        assert_eq!(layout.tile, TILE_4K);
        assert_eq!(layout.block_w, 32);
        assert_eq!(layout.block_h, 16);
        assert_eq!(layout.mips[0].offset, 176128);
        assert_eq!(layout.mips[3].offset, 4096);
        assert_eq!(layout.mips[4].tile_x, 16);
        assert_eq!(layout.mips[8].tile_y, 4);
        assert_eq!(layout.mips[8].tail_offset, Some(768));
    }

    #[test]
    fn odd_uncompressed_layout_is_one_tail_tile() {
        let shape = TextureShape {
            width: 17,
            height: 13,
            depth: 1,
            mips: 5,
            dxgi: 61,
            dimension: 1,
            mode: 2,
            non_tail: 0,
        };
        let layout = layout(&shape).unwrap();
        assert_eq!(layout.total_bytes, 4096);
        assert_eq!(layout.first_tail, 0);
        assert_eq!(layout.mips[1].blocks_w, 9);
        assert_eq!(layout.mips[1].pixels_w, 8);
        assert_eq!(layout.mips[1].tile_x, 16);
        assert_eq!(layout.mips[1].tile_y, 32);
        assert_eq!(layout.mips[1].region, 4096);
    }
}
