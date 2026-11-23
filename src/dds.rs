//! DX10 DDS writer for detiled RDS5/TTPF surfaces.
//!
//! Adapted from Leonida's `tools/ttpf_mips.py` (`dds_bytes` and
//! `texture_dds`) at revision `aa2988cc555bad7ecd508b48a702adbb32fe7c95`,
//! GPL-3.0-or-later (combined project GPL-3.0-only). The surface layout,
//! visible-row cropping and subresource
//! ordering come from `src/layout.rs`.
//!
//! Every subresource length is checked against the computed layout before a
//! header is written. A short or long vector is an error; padding is never
//! invented.

use crate::layout::{MipLayout, SurfaceKind, SurfaceLayout, TextureShape, format_info, layout};
use std::io;

const DDSD_CAPS: u32 = 0x0000_0001;
const DDSD_HEIGHT: u32 = 0x0000_0002;
const DDSD_WIDTH: u32 = 0x0000_0004;
const DDSD_PITCH: u32 = 0x0000_0008;
const DDSD_PIXELFORMAT: u32 = 0x0000_1000;
const DDSD_MIPMAPCOUNT: u32 = 0x0002_0000;
const DDSD_LINEARSIZE: u32 = 0x0008_0000;
const DDSD_DEPTH: u32 = 0x0080_0000;

const DDSCAPS_COMPLEX: u32 = 0x0000_0008;
const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
const DDSCAPS_MIPMAP: u32 = 0x0040_0000;

const DDSCAPS2_CUBEMAP_ALLFACES: u32 = 0x0000_FE00;
const DDSCAPS2_VOLUME: u32 = 0x0020_0000;

const D3D10_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
const D3D10_RESOURCE_DIMENSION_TEXTURE3D: u32 = 4;
const D3D10_RESOURCE_MISC_TEXTURECUBE: u32 = 0x4;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn checked_u32(value: u64, what: &str) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid(format!("{what} does not fit a DDS field")))
}

fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn levels_for(layout: &SurfaceLayout, count: usize) -> io::Result<u32> {
    let slices = if layout.kind == SurfaceKind::Volume3D {
        1
    } else {
        layout.slices as usize
    };
    if count == 0 {
        return Err(invalid("no subresources supplied"));
    }
    if slices == 0 || !count.is_multiple_of(slices) {
        return Err(invalid(format!(
            "{count} subresources are not a whole number of {slices}-slice sets"
        )));
    }
    let levels = count / slices;
    if levels == 0 || levels > layout.mips.len() {
        return Err(invalid(format!(
            "subresource count implies {levels} mips, layout has {}",
            layout.mips.len()
        )));
    }
    Ok(levels as u32)
}

fn expected_len(layout: &SurfaceLayout, mip: &MipLayout) -> io::Result<usize> {
    layout.subresource_len(mip.index)
}

/// Write a DX10 DDS from detiled subresources.
///
/// `subresources` is ordered **slice-major, then mip**: for every array slice
/// or cube face, mip 0 first and lower mips after it. A 2D texture has one
/// slice. A 3D volume has one vector per mip and that vector contains all Z
/// slices in order, because DX10 has no per-Z subresource index.
///
/// The chain may be partial (the leading `N` mips) as long as every vector is
/// exactly the layout's tightly packed visible size. Passing complete chains
/// is the normal case.
pub fn write_dds(shape: &TextureShape, subresources: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    let layout = layout(shape)?;
    let info = format_info(shape.dxgi).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("unsupported or unknown DXGI format {}", shape.dxgi),
        )
    })?;
    let levels = levels_for(&layout, subresources.len())?;

    let mut payload_len = 0usize;
    let mut index = 0usize;
    if layout.kind == SurfaceKind::Volume3D {
        for mip in 0..levels {
            let expected = expected_len(&layout, layout.mip(mip)?)?;
            payload_len = payload_len
                .checked_add(expected)
                .ok_or_else(|| invalid("DDS payload overflows this platform"))?;
            let actual = subresources[index].len();
            if actual != expected {
                return Err(invalid(format!(
                    "subresource {index} (mip {mip}) has {actual} bytes, expected {expected}"
                )));
            }
            index += 1;
        }
    } else {
        for slice in 0..layout.slices {
            for mip in 0..levels {
                let expected = expected_len(&layout, layout.mip(mip)?)?;
                payload_len = payload_len
                    .checked_add(expected)
                    .ok_or_else(|| invalid("DDS payload overflows this platform"))?;
                let actual = subresources[index].len();
                if actual != expected {
                    return Err(invalid(format!(
                        "subresource {index} (slice {slice}, mip {mip}) has {actual} bytes, \
                         expected {expected}"
                    )));
                }
                index += 1;
            }
        }
    }

    let mut payload = Vec::new();
    payload
        .try_reserve_exact(payload_len)
        .map_err(|_| invalid("DDS payload is too large to allocate"))?;
    for bytes in subresources {
        payload.extend_from_slice(bytes);
    }

    // Top-level pitch (uncompressed) or top-mip linear size (BC), mirroring
    // texture_dds: tight visible block rows for BC, width * element for plain.
    let (pitch_or_linear, size_flag) = if layout.block == 4 {
        let w = (shape.width as u64).div_ceil(4);
        let h = (shape.height as u64).div_ceil(4);
        let linear = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(info.element_bytes as u64))
            .ok_or_else(|| invalid("DDS top-mip linear size overflows"))?;
        (
            checked_u32(linear, "DDS top-mip linear size")?,
            DDSD_LINEARSIZE,
        )
    } else {
        let pitch = (shape.width as u64)
            .checked_mul(info.element_bytes as u64)
            .ok_or_else(|| invalid("DDS row pitch overflows"))?;
        (checked_u32(pitch, "DDS row pitch")?, DDSD_PITCH)
    };

    let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | size_flag;
    if levels > 1 {
        flags |= DDSD_MIPMAPCOUNT;
    }
    if layout.kind == SurfaceKind::Volume3D {
        flags |= DDSD_DEPTH;
    }

    let mut caps = DDSCAPS_TEXTURE;
    if levels > 1 {
        caps |= DDSCAPS_COMPLEX | DDSCAPS_MIPMAP;
    }
    let array_size = match layout.kind {
        SurfaceKind::Array2D => shape.depth,
        _ => 1,
    };
    if layout.kind == SurfaceKind::Cubemap || layout.kind == SurfaceKind::Volume3D || array_size > 1
    {
        caps |= DDSCAPS_COMPLEX;
    }
    let caps2 = match layout.kind {
        SurfaceKind::Cubemap => DDSCAPS2_CUBEMAP_ALLFACES,
        SurfaceKind::Volume3D => DDSCAPS2_VOLUME,
        _ => 0,
    };

    let mut header = [0u8; 128];
    header[..4].copy_from_slice(b"DDS ");
    set_u32(&mut header, 4, 124);
    set_u32(&mut header, 8, flags);
    set_u32(&mut header, 12, shape.height);
    set_u32(&mut header, 16, shape.width);
    set_u32(&mut header, 20, pitch_or_linear);
    set_u32(
        &mut header,
        24,
        if layout.kind == SurfaceKind::Volume3D {
            shape.depth
        } else {
            0
        },
    );
    set_u32(&mut header, 28, levels);
    set_u32(&mut header, 76, 32);
    set_u32(&mut header, 80, 4);
    header[84..88].copy_from_slice(b"DX10");
    set_u32(&mut header, 108, caps);
    set_u32(&mut header, 112, caps2);

    let dimension = if layout.kind == SurfaceKind::Volume3D {
        D3D10_RESOURCE_DIMENSION_TEXTURE3D
    } else {
        D3D10_RESOURCE_DIMENSION_TEXTURE2D
    };
    let misc = if layout.kind == SurfaceKind::Cubemap {
        D3D10_RESOURCE_MISC_TEXTURECUBE
    } else {
        0
    };
    let mut dx10 = [0u8; 20];
    set_u32(&mut dx10, 0, shape.dxgi);
    set_u32(&mut dx10, 4, dimension);
    set_u32(&mut dx10, 8, misc);
    set_u32(&mut dx10, 12, array_size);
    set_u32(&mut dx10, 16, 0);

    let mut out = Vec::new();
    out.try_reserve_exact(148 + payload.len())
        .map_err(|_| invalid("DDS is too large to allocate"))?;
    out.extend_from_slice(&header);
    out.extend_from_slice(&dx10);
    out.extend_from_slice(&payload);
    Ok(out)
}
