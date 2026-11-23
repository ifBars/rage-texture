//! Independent DDS container consumer used by interoperability checks.
//!
//! `ddsfile` parses the file. This module then checks that the parsed DX10
//! header and payload describe exactly the `TextureShape` the SDK claims to
//! have written. It deliberately does not call the SDK DDS writer or layout
//! code: the expected subresource sizes are derived from the DDS header
//! semantics and the format's own pitch/block rules.
//!
//! The payload is treated as one container-level length per mip and slice.
//! Pixel decoding is a separate check (DirectXTex in
//! `scripts/verify_dds_consumers.py`), so a container pass here does not
//! imply the pixels are correct.
//!
//! Full chains only: the header mip count must equal `shape.mips`. Callers
//! that audit partial chains pass the shape of the chain that was written.

use ddsfile::{Caps2, D3D10ResourceDimension, Dds, MiscFlag};
use rage_texture::layout::TextureShape;
use std::io::{Cursor, Error, ErrorKind};

fn bad(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Texture2D,
    Array,
    Cube,
    Volume,
}

fn kind_of(shape: &TextureShape) -> std::io::Result<Kind> {
    match (shape.dimension, shape.depth) {
        (1, 1) => Ok(Kind::Texture2D),
        (1, d) if d > 1 => Ok(Kind::Array),
        (2, 6) => Ok(Kind::Cube),
        (2, d) => Err(bad(format!("a cubemap needs depth 6, got {d}"))),
        (3, d) if d > 0 => Ok(Kind::Volume),
        (3, _) => Err(bad("a 3D volume needs a nonzero depth")),
        (other, _) => Err(bad(format!("unsupported descriptor dimension {other}"))),
    }
}

/// Validate one complete DX10 DDS against the shape it should describe.
///
/// Checks: DX10 format, width/height, mip count, array size, cube flags,
/// volume depth and dimension, top-level pitch/linear size, and the exact
/// payload length of every mip/slice subresource. Unknown formats and
/// non-DX10 files are errors, not passes.
pub fn validate(data: &[u8], shape: &TextureShape) -> std::io::Result<()> {
    let dds = Dds::read(&mut Cursor::new(data)).map_err(|e| bad(format!("ddsfile parse: {e}")))?;
    let h10 = dds
        .header10
        .as_ref()
        .ok_or_else(|| bad("missing DX10 extension header"))?;
    let kind = kind_of(shape)?;

    if h10.dxgi_format as u32 != shape.dxgi {
        return Err(bad(format!(
            "DXGI format {} does not match expected {}",
            h10.dxgi_format as u32, shape.dxgi
        )));
    }
    let Some(format) = dds.get_format() else {
        return Err(bad(format!(
            "ddsfile has no pixel format for DXGI {}",
            shape.dxgi
        )));
    };
    if dds.get_width() != shape.width || dds.get_height() != shape.height {
        return Err(bad(format!(
            "dimensions {}x{} do not match expected {}x{}",
            dds.get_width(),
            dds.get_height(),
            shape.width,
            shape.height
        )));
    }

    let expected_dimension = if kind == Kind::Volume {
        D3D10ResourceDimension::Texture3D
    } else {
        D3D10ResourceDimension::Texture2D
    };
    if h10.resource_dimension != expected_dimension {
        return Err(bad(format!(
            "resource dimension {:?} does not match expected {:?}",
            h10.resource_dimension, expected_dimension
        )));
    }

    if dds.get_num_mipmap_levels() != shape.mips {
        return Err(bad(format!(
            "mip count {} does not match expected {}",
            dds.get_num_mipmap_levels(),
            shape.mips
        )));
    }

    let capped_slices = matches!(kind, Kind::Array | Kind::Cube);
    // DX10 arraySize counts array elements, and one for a cube (six faces).
    let expected_array = if kind == Kind::Array { shape.depth } else { 1 };
    if h10.array_size != expected_array {
        return Err(bad(format!(
            "array size {} does not match expected {}",
            h10.array_size, expected_array
        )));
    }
    if let Some(depth) = dds.header.depth {
        let ok = match kind {
            Kind::Volume => depth == shape.depth,
            _ => false,
        };
        if !ok {
            return Err(bad(format!(
                "header depth {depth} does not match the {:?} shape",
                kind
            )));
        }
    } else if kind == Kind::Volume {
        return Err(bad("a volume needs a header depth"));
    }

    let cube = h10.misc_flag.contains(MiscFlag::TEXTURECUBE);
    if cube != (kind == Kind::Cube) {
        return Err(bad(format!(
            "cube misc flag {cube} does not match the {:?} shape",
            kind
        )));
    }
    let caps_cube = dds.header.caps2.contains(Caps2::CUBEMAP);
    if caps_cube != (kind == Kind::Cube) {
        return Err(bad(format!(
            "caps2 cube flag {caps_cube} does not match the {:?} shape",
            kind
        )));
    }
    if kind == Kind::Cube {
        let all = dds.header.caps2.contains(Caps2::CUBEMAP_ALLFACES);
        if !all {
            return Err(bad("cubemap is missing one or more face flags"));
        }
    }
    if kind == Kind::Volume {
        if !dds.header.caps2.contains(Caps2::VOLUME) {
            return Err(bad("volume is missing the caps2 volume flag"));
        }
    } else if dds.header.caps2.contains(Caps2::VOLUME) {
        return Err(bad("non-volume has the caps2 volume flag"));
    }

    // Top-level pitch (uncompressed) or linear size (block-compressed).
    let pitch_height = format.get_pitch_height();
    let top_pitch = format
        .get_pitch(shape.width)
        .ok_or_else(|| bad(format!("no pitch for DXGI {}", shape.dxgi)))?;
    let top_rows = shape.height.div_ceil(pitch_height);
    if format.get_block_size().is_some() {
        let expected = u64::from(top_pitch) * u64::from(top_rows);
        if dds.header.linear_size.map(u64::from) != Some(expected) {
            return Err(bad(format!(
                "top-level linear size {:?} does not match expected {expected}",
                dds.header.linear_size
            )));
        }
    } else if dds.header.pitch != Some(top_pitch) {
        return Err(bad(format!(
            "top-level pitch {:?} does not match expected {top_pitch}",
            dds.header.pitch
        )));
    }

    // Exact payload length, mip by mip. A volume's Z depth shrinks with the
    // mip; arrays and cube faces carry one independent chain per layer.
    let mut per_layer = 0u64;
    for mip in 0..shape.mips {
        let w = (shape.width >> mip).max(1);
        let h = (shape.height >> mip).max(1);
        let pitch = format
            .get_pitch(w)
            .ok_or_else(|| bad(format!("no pitch for DXGI {} at mip {mip}", shape.dxgi)))?;
        let rows = h.div_ceil(pitch_height);
        let depth = if kind == Kind::Volume {
            u64::from((shape.depth >> mip).max(1))
        } else {
            1
        };
        let bytes = u64::from(pitch) * u64::from(rows) * depth;
        per_layer = per_layer
            .checked_add(bytes)
            .ok_or_else(|| bad("payload length overflow"))?;
    }
    let layers = if capped_slices {
        u64::from(shape.depth)
    } else {
        1
    };
    let expected = per_layer
        .checked_mul(layers)
        .ok_or_else(|| bad("payload length overflow"))?;
    let actual = dds.data.len() as u64;
    if actual != expected {
        return Err(bad(format!(
            "payload is {actual} bytes, expected {expected} \
             ({} mip(s), {layers} layer(s))",
            shape.mips
        )));
    }
    Ok(())
}
