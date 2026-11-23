//! PNG is an 8-bit preview. DDS remains the precision-preserving export.
//!
//! Signed-normalized formats map [-1, +1] to [0, 255] through [`snorm`]:
//! -1 -> 0, 0 -> 128, +1 -> 255. This mapping is preview presentation only;
//! the DDS keeps the original signed bytes. BC4_SNORM is shown as grey and
//! BC5_SNORM maps its X (first 8 bytes) to red and Y (second 8 bytes) to
//! green, matching the DXGI channel layout. Blue is zero and alpha is opaque.
//! The X/Y sign range is preserved instead of being reinterpreted as UNORM.
use crate::invalid;
use std::io;

/// Map a signed-normalized float in [-1, 1] to one preview channel.
///
/// Values outside the range are clamped, NaN becomes 0, and rounding uses
/// the nearest integer (0 maps to 128). Callers that need the signed float
/// itself should keep the DDS export.
pub fn snorm(value: f32) -> u8 {
    if value.is_nan() {
        0
    } else {
        ((value.clamp(-1., 1.) * 0.5 + 0.5) * 255.).round() as u8
    }
}

fn snorm8(value: u8) -> u8 {
    snorm(f32::from(value as i8) / 127.)
}

fn snorm16(value: u16) -> u8 {
    snorm(f32::from(value as i16) / 32767.)
}

fn unorm10(value: u32) -> u8 {
    ((value * 255 + 511) / 1023) as u8
}

/// Five-bit-exponent unsigned float with 6-bit (R/G) or 5-bit (B) mantissas.
fn unsigned_float(code: u32, mantissa_bits: u32) -> f32 {
    let exponent = code >> mantissa_bits;
    let fraction = code & ((1 << mantissa_bits) - 1);
    if exponent == 31 {
        return if fraction == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        };
    }
    let significand = fraction as f32
        + if exponent == 0 {
            0.
        } else {
            (1u32 << mantissa_bits) as f32
        };
    significand * 2f32.powi(exponent.max(1) as i32 - 15 - mantissa_bits as i32)
}

/// Whether [`rgba8`] has a typed preview for `dxgi`.
///
/// Typeless descriptors, integer formats, depth/stencil formats and packed
/// video formats are deliberately not previewed.
pub fn supports(dxgi: u32) -> bool {
    const TYPED: [u32; 40] = [
        2, 10, 11, 13, 16, 24, 26, 28, 29, 31, 34, 35, 37, 41, 49, 51, 54, 56, 58, 61, 63, 65, 71,
        72, 74, 75, 77, 78, 80, 81, 83, 84, 87, 88, 91, 93, 95, 96, 98, 99,
    ];
    TYPED.contains(&dxgi)
}

pub fn rgba8(dxgi: u32, width: u32, height: u32, linear: &[u8]) -> io::Result<Vec<u8>> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .filter(|n| *n <= 64 * 1024 * 1024)
        .ok_or_else(|| invalid("preview dimensions exceed limit"))?;
    if matches!(dxgi, 74..=75 | 77..=78) {
        return bc23(width, height, linear, matches!(dxgi, 77..=78));
    }
    if dxgi == 81 {
        return bc4_snorm(width, height, linear);
    }
    if dxgi == 84 {
        return bc5_snorm(width, height, linear);
    }
    type Decoder = fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>;
    let decode: Option<Decoder> = match dxgi {
        71..=72 => Some(
            texture2ddecoder::decode_bc1a
                as fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>,
        ),
        80 => Some(texture2ddecoder::decode_bc4),
        83 => Some(texture2ddecoder::decode_bc5),
        95 => Some(crate::bc6h::decode_unsigned),
        96 => Some(crate::bc6h::decode_signed),
        98..=99 => Some(texture2ddecoder::decode_bc7),
        _ => None,
    };
    if let Some(decode) = decode {
        let mut pixels = vec![0u32; count];
        decode(linear, width as usize, height as usize, &mut pixels).map_err(invalid)?;
        // texture2ddecoder returns 0xAARRGGBB, independent of host endianness.
        return Ok(pixels
            .into_iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
            .collect());
    }
    let bpp = match dxgi {
        2 => 16,
        10 | 11 | 13 | 16 => 8,
        24 | 26 | 28..=29 | 31 | 34 | 35 | 37 | 41 | 87 | 88 | 91 | 93 => 4,
        49 | 51 | 54 | 56 | 58 => 2,
        61 | 63 | 65 => 1,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "PNG preview not implemented for this DXGI format; DDS preserves raw data",
            ));
        }
    };
    if linear.len() != count * bpp {
        return Err(invalid("linear pixel length mismatch"));
    }
    let mut out = Vec::with_capacity(count * 4);
    for p in linear.chunks_exact(bpp) {
        let u16at = |i| u16::from_le_bytes([p[i], p[i + 1]]);
        let f32at = |i| f32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
        let unorm = |v: u16| ((v as u32 * 255 + 32767) / 65535) as u8;
        let channel = |v: f32| {
            if v.is_nan() {
                0
            } else {
                (v.clamp(0., 1.) * 255.).round() as u8
            }
        };
        let rgba = match dxgi {
            2 => [
                channel(f32at(0)),
                channel(f32at(4)),
                channel(f32at(8)),
                channel(f32at(12)),
            ],
            10 => [
                channel(half_float(u16at(0))),
                channel(half_float(u16at(2))),
                channel(half_float(u16at(4))),
                channel(half_float(u16at(6))),
            ],
            11 => [
                unorm(u16at(0)),
                unorm(u16at(2)),
                unorm(u16at(4)),
                unorm(u16at(6)),
            ],
            13 => [
                snorm16(u16at(0)),
                snorm16(u16at(2)),
                snorm16(u16at(4)),
                snorm16(u16at(6)),
            ],
            16 => [channel(f32at(0)), channel(f32at(4)), 0, 255],
            24 => {
                let v = u32::from_le_bytes(p[..4].try_into().unwrap());
                [
                    unorm10(v & 1023),
                    unorm10((v >> 10) & 1023),
                    unorm10((v >> 20) & 1023),
                    ((v >> 30) * 85) as u8,
                ]
            }
            26 => {
                let v = u32::from_le_bytes(p[..4].try_into().unwrap());
                [
                    channel(unsigned_float(v & 2047, 6)),
                    channel(unsigned_float((v >> 11) & 2047, 6)),
                    channel(unsigned_float(v >> 22, 5)),
                    255,
                ]
            }
            28..=29 => [p[0], p[1], p[2], p[3]],
            31 => [snorm8(p[0]), snorm8(p[1]), snorm8(p[2]), snorm8(p[3])],
            34 => [
                channel(half_float(u16at(0))),
                channel(half_float(u16at(2))),
                0,
                255,
            ],
            35 => [unorm(u16at(0)), unorm(u16at(2)), 0, 255],
            37 => [snorm16(u16at(0)), snorm16(u16at(2)), 0, 255],
            41 => {
                let v = channel(f32at(0));
                [v, v, v, 255]
            }
            49 => [p[0], p[1], 0, 255],
            51 => [snorm8(p[0]), snorm8(p[1]), 0, 255],
            54 => {
                let v = channel(half_float(u16at(0)));
                [v, v, v, 255]
            }
            56 => {
                let v = unorm(u16at(0));
                [v, v, v, 255]
            }
            58 => {
                let v = snorm16(u16at(0));
                [v, v, v, 255]
            }
            61 => [p[0], p[0], p[0], 255],
            63 => {
                let v = snorm8(p[0]);
                [v, v, v, 255]
            }
            65 => [255, 255, 255, p[0]],
            87 | 91 => [p[2], p[1], p[0], p[3]],
            // B8G8R8X8: the fourth byte is padding and is not shown.
            88 | 93 => [p[2], p[1], p[0], 255],
            _ => unreachable!(),
        };
        out.extend_from_slice(&rgba);
    }
    Ok(out)
}

/// Signed BC4 levels for one block, with -128 clamped to -127.
///
/// Interpolation follows the Microsoft BC4_SNORM table: six values when
/// `red_0 > red_1`, otherwise four values plus -1 and +1. Comparison happens
/// after the endpoint clamp.
fn bc4_snorm_levels(block: &[u8]) -> [u8; 8] {
    let endpoint = |byte: u8| f32::from((byte as i8).max(-127)) / 127.;
    let a0 = endpoint(block[0]);
    let a1 = endpoint(block[1]);
    let mut levels = [0u8; 8];
    levels[0] = snorm(a0);
    levels[1] = snorm(a1);
    if a0 > a1 {
        levels[2] = snorm((6. * a0 + a1) / 7.);
        levels[3] = snorm((5. * a0 + 2. * a1) / 7.);
        levels[4] = snorm((4. * a0 + 3. * a1) / 7.);
        levels[5] = snorm((3. * a0 + 4. * a1) / 7.);
        levels[6] = snorm((2. * a0 + 5. * a1) / 7.);
        levels[7] = snorm((a0 + 6. * a1) / 7.);
    } else {
        levels[2] = snorm((4. * a0 + a1) / 5.);
        levels[3] = snorm((3. * a0 + 2. * a1) / 5.);
        levels[4] = snorm((2. * a0 + 3. * a1) / 5.);
        levels[5] = snorm((a0 + 4. * a1) / 5.);
        levels[6] = 0;
        levels[7] = 255;
    }
    levels
}

fn bc4_indices(block: &[u8]) -> u64 {
    u64::from_le_bytes(block[..8].try_into().unwrap()) >> 16
}

fn bc4_snorm(width: u32, height: u32, data: &[u8]) -> io::Result<Vec<u8>> {
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    if data.len() != bw * bh * 8 {
        return Err(invalid("BC4_SNORM block length mismatch"));
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for (i, b) in data.chunks_exact(8).enumerate() {
        let levels = bc4_snorm_levels(b);
        let indices = bc4_indices(b);
        for p in 0..16 {
            let x = (i % bw) * 4 + p % 4;
            let y = (i / bw) * 4 + p / 4;
            if x >= width as usize || y >= height as usize {
                continue;
            }
            let v = levels[((indices >> (3 * p)) & 7) as usize];
            let at = (y * width as usize + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    Ok(rgba)
}

fn bc5_snorm(width: u32, height: u32, data: &[u8]) -> io::Result<Vec<u8>> {
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    if data.len() != bw * bh * 16 {
        return Err(invalid("BC5_SNORM block length mismatch"));
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for (i, b) in data.chunks_exact(16).enumerate() {
        let x = bc4_snorm_levels(&b[..8]);
        let y = bc4_snorm_levels(&b[8..]);
        let xi = bc4_indices(&b[..8]);
        let yi = bc4_indices(&b[8..]);
        for p in 0..16 {
            let px = (i % bw) * 4 + p % 4;
            let py = (i / bw) * 4 + p / 4;
            if px >= width as usize || py >= height as usize {
                continue;
            }
            let at = (py * width as usize + px) * 4;
            rgba[at..at + 4].copy_from_slice(&[
                x[((xi >> (3 * p)) & 7) as usize],
                y[((yi >> (3 * p)) & 7) as usize],
                0,
                255,
            ]);
        }
    }
    Ok(rgba)
}

fn bc23(width: u32, height: u32, data: &[u8], interpolated_alpha: bool) -> io::Result<Vec<u8>> {
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    if data.len() != bw * bh * 16 {
        return Err(invalid("BC2/BC3 block length mismatch"));
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for (i, b) in data.chunks_exact(16).enumerate() {
        let rgb = |at| {
            let v = u16::from_le_bytes([b[at], b[at + 1]]) as u32;
            let r = (v >> 11) & 31;
            let g = (v >> 5) & 63;
            let blue = v & 31;
            [
                (r << 3) | (r >> 2),
                (g << 2) | (g >> 4),
                (blue << 3) | (blue >> 2),
            ]
        };
        let c0 = rgb(8);
        let c1 = rgb(10);
        let mut colors = [[0u32; 3]; 4];
        colors[0] = c0;
        colors[1] = c1;
        for j in 0..3 {
            colors[2][j] = (2 * c0[j] + c1[j]) / 3;
            colors[3][j] = (c0[j] + 2 * c1[j]) / 3;
        }
        let indices = u32::from_le_bytes(b[12..16].try_into().unwrap());
        // BC2 and BC3 always use four colour entries, irrespective of
        // endpoint ordering. Their alpha blocks are independent of colour.
        let alpha: [u8; 16] = if interpolated_alpha {
            let mut red = [0u32; 16];
            texture2ddecoder::decode_bc4_block(&b[..8], &mut red);
            red.map(|pixel| (pixel >> 16) as u8)
        } else {
            let bits = u64::from_le_bytes(b[..8].try_into().unwrap());
            std::array::from_fn(|p| (((bits >> (4 * p)) & 15) * 17) as u8)
        };
        for p in 0..16 {
            let x = (i % bw) * 4 + p % 4;
            let y = (i / bw) * 4 + p / 4;
            if x >= width as usize || y >= height as usize {
                continue;
            }
            let color = colors[((indices >> (2 * p)) & 3) as usize];
            let at = (y * width as usize + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[
                color[0] as u8,
                color[1] as u8,
                color[2] as u8,
                alpha[p],
            ]);
        }
    }
    Ok(rgba)
}
pub(crate) fn half_float(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 31;
    let fraction = (bits & 1023) as f32;
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 => {
            if fraction == 0. {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1. + fraction / 1024.) * 2f32.powi(exponent as i32 - 15),
    }
}
pub fn png(dxgi: u32, width: u32, height: u32, linear: &[u8]) -> io::Result<Vec<u8>> {
    let rgba = rgba8(dxgi, width, height, linear)?;
    let mut result = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut result, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&rgba)?;
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bc1_preserves_one_bit_alpha() {
        let block = [0, 0, 255, 255, 255, 255, 255, 255];
        for dxgi in [71, 72] {
            assert_eq!(rgba8(dxgi, 4, 4, &block).unwrap(), [0; 64]);
            let opaque = [255, 255, 0, 0, 255, 255, 255, 255];
            assert_eq!(
                rgba8(dxgi, 4, 4, &opaque).unwrap(),
                [85, 85, 85, 255].repeat(16)
            );
        }
    }
    #[test]
    fn bc3_always_uses_four_color_mode() {
        for dxgi in [77, 78] {
            for (first, second, index, expected) in [
                (0u16, 0xffffu16, 0xaa, 85),
                (0, 0xffff, 0xff, 170),
                (0xffff, 0, 0xaa, 170),
                (0xffff, 0, 0xff, 85),
                (0xffff, 0xffff, 0xff, 255),
            ] {
                let mut block = [0; 16];
                block[0] = 173;
                block[8..10].copy_from_slice(&first.to_le_bytes());
                block[10..12].copy_from_slice(&second.to_le_bytes());
                block[12..].fill(index);
                assert_eq!(
                    rgba8(dxgi, 4, 4, &block).unwrap(),
                    [expected, expected, expected, 173].repeat(16)
                );
            }
        }
    }
    #[test]
    fn bc3_clips_edge_blocks_and_keeps_interpolated_alpha() {
        let mut block = [0u8; 16];
        block[0] = 220;
        block[1] = 80;
        let indices = (0..16).fold(0u64, |bits, p| bits | ((p % 8) as u64) << (3 * p));
        block[2..8].copy_from_slice(&indices.to_le_bytes()[..6]);
        block[10..12].copy_from_slice(&0xffffu16.to_le_bytes());
        block[12..].fill(0xaa);
        let mut data = block.repeat(2);
        data[16 + 12..].fill(0xff);
        let alpha = [220, 80, 200, 180, 160, 140, 120, 100];
        let out = rgba8(77, 5, 3, &data).unwrap();
        assert_eq!(out.len(), 5 * 3 * 4);
        for y in 0..3 {
            for x in 0..5 {
                let value = if x < 4 { 85 } else { 170 };
                let a = alpha[(y * 4 + x % 4) % 8];
                assert_eq!(&out[(y * 5 + x) * 4..][..4], &[value, value, value, a]);
            }
        }
        assert!(rgba8(77, 5, 3, &data[..31]).is_err());
    }
    #[test]
    fn typeless_previews_require_a_typed_interpretation() {
        // DXGI_FORMAT values follow Microsoft's dxgiformat.h enumeration.
        // DDS preserves these formats; a preview cannot infer signedness or
        // color space from a typeless descriptor.
        for format in [
            1, 5, 9, 15, 19, 22, 23, 27, 33, 39, 44, 48, 53, 60, 70, 73, 76, 79, 82, 90, 92, 94, 97,
        ] {
            assert!(!supports(format), "DXGI {format} claimed");
            assert_eq!(
                rgba8(format, 4, 4, &[0; 128]).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
        let texel = [0x00, 0x3c, 0, 0, 0, 0, 0x00, 0x3c];
        assert_eq!(rgba8(10, 1, 1, &texel).unwrap(), [255, 0, 0, 255]);
    }
    #[test]
    fn bc2_explicit_alpha_and_four_color_mode() {
        let mut b = [0u8; 16];
        b[..8].fill(0xff);
        b[10..12].copy_from_slice(&0xffffu16.to_le_bytes());
        b[12..].fill(0xaa);
        let out = rgba8(74, 4, 4, &b).unwrap();
        assert_eq!(&out[..4], &[85, 85, 85, 255]);
    }
    #[test]
    fn rgba_and_bgra_agree() {
        assert_eq!(
            rgba8(28, 1, 1, &[1, 2, 3, 4]).unwrap(),
            rgba8(87, 1, 1, &[3, 2, 1, 4]).unwrap()
        );
    }
    #[test]
    fn floats_are_preview_clamped() {
        assert_eq!(
            rgba8(54, 1, 1, &0x3c00u16.to_le_bytes()).unwrap(),
            [255, 255, 255, 255]
        );
    }
    #[test]
    fn png_signature() {
        assert_eq!(
            &png(28, 1, 1, &[1, 2, 3, 255]).unwrap()[..8],
            b"\x89PNG\r\n\x1a\n"
        );
    }
}
