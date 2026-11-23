//! Attributed adaptation of SixNG contributors' GPL-3.0-only decoder.
//! Source: https://github.com/holdmysocks/SixNG/pull/23
//! Revision 2979c57fdeb35863bb8885e6b48d09090cd1f7de (open upstream PR).
//! Reused with attribution; independent half-bit oracle validation is separate.
//! Plain BC6H (UF16 / SF16) blocks -> pixels, following the BC6H
//! specification. Block fields come from `bc6pack::unpack`.
//!
//! This replaces `texture2ddecoder::decode_bc6` (0.1.2), which differs from
//! the specification:
//!
//! - unsigned unquantize maps `1 << (bits - 1)` to 0xFFFF; the value the
//!   specification maps there is `(1 << bits) - 1`. Half-range endpoints came
//!   out at full brightness.
//! - the 16-bit one-region mode (code 15) builds its endpoint mask
//!   `(1 << 16) - 1` in u16, which is 0 in release builds, so the second
//!   endpoint decoded as 0.
//! - signed blocks are handled as sign-magnitude after a two's complement
//!   sign extension, so negative endpoints came out wrong.

use crate::bc6pack::{Fields, MODES, unpack};

const W3: [i32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const W4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

fn sext(v: i32, bits: u32) -> i32 {
    let s = 32 - bits;
    (v << s) >> s
}

/// Endpoints [w, x, y, z][R, G, B] with deltas resolved; sign-extended for signed formats.
#[allow(clippy::needless_range_loop)] // Endpoint/channel coordinates mirror the specification.
fn endpoints(f: &Fields, signed: bool) -> [[i32; 3]; 4] {
    let m = &MODES[f.mode as usize];
    let n = if m.two_regions { 4 } else { 2 };
    let mask = (1i32 << m.ep_bits) - 1;
    let fin = |v: i32| {
        if signed {
            sext(v & mask, m.ep_bits)
        } else {
            v & mask
        }
    };
    let mut e = [[0i32; 3]; 4];
    for ch in 0..3 {
        let w = f.ep[0][ch] as i32;
        e[0][ch] = fin(w);
        for k in 1..n {
            let v = f.ep[k][ch] as i32;
            e[k][ch] = fin(if m.transformed {
                w + sext(v, m.delta_bits[ch])
            } else {
                v
            });
        }
    }
    e
}

/// Endpoint of `bits` bits -> 16-bit interpolation range (17-bit signed for signed formats).
fn unquantize(x: i32, bits: u32, signed: bool) -> i32 {
    if signed {
        if bits >= 16 {
            return x;
        }
        let a = x.abs();
        let u = if a == 0 {
            0
        } else if a >= (1 << (bits - 1)) - 1 {
            0x7FFF
        } else {
            ((a << 15) + 0x4000) >> (bits - 1)
        };
        if x < 0 { -u } else { u }
    } else if bits >= 15 {
        x
    } else if x == 0 {
        0
    } else if x == (1 << bits) - 1 {
        0xFFFF
    } else {
        ((x << 15) + 0x4000) >> (bits - 1)
    }
}

/// Interpolated value -> half-float bits.
fn finish(v: i32, signed: bool) -> u16 {
    if !signed {
        ((v * 31) >> 6) as u16
    } else {
        // The specification scales before deciding the sign. A negative
        // interpolant that truncates to zero must produce positive zero.
        let scaled = if v < 0 {
            -((-v * 31) >> 5)
        } else {
            (v * 31) >> 5
        };
        if scaled < 0 {
            0x8000 | (-scaled) as u16
        } else {
            scaled as u16
        }
    }
}

/// One block -> 16 RGB pixels as half-float bits, row order. Reserved mode
/// codes give black, as the specification says.
pub fn decode_block(block: &[u8; 16], signed: bool) -> [[u16; 3]; 16] {
    let mut out = [[0u16; 3]; 16];
    let Some(f) = unpack(block) else { return out };
    let m = &MODES[f.mode as usize];
    let e = endpoints(&f, signed);
    let part = crate::bc7pack::PART2[f.partition as usize & 31];
    for p in 0..16 {
        let (r, w) = if m.two_regions {
            ((part >> (2 * p) & 3) as usize, W3[f.idx[p] as usize & 7])
        } else {
            (0, W4[f.idx[p] as usize & 15])
        };
        for ch in 0..3 {
            let a = unquantize(e[2 * r][ch], m.ep_bits, signed);
            let b = unquantize(e[2 * r + 1][ch], m.ep_bits, signed);
            out[p][ch] = finish((a * (64 - w) + b * w + 32) >> 6, signed);
        }
    }
    out
}

/// BC6H image (blocks in row order) -> `0xAARRGGBB` pixels with each channel
/// clamped to 0..1, the same output as `texture2ddecoder::decode_bc6`.
pub fn decode_image(
    data: &[u8],
    w: usize,
    h: usize,
    img: &mut [u32],
    signed: bool,
) -> Result<(), &'static str> {
    let (bw, bh, count) = image_dimensions(data, w, h)?;
    if img.len() < count {
        return Err("Image buffer is too small!");
    }
    let to8 = |hf: u16| (crate::preview::half_float(hf) * 255.0).clamp(0.0, 255.0) as u32;
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * 16;
            let px = decode_block(data[o..o + 16].try_into().unwrap(), signed);
            for (p, &[r, g, b]) in px.iter().enumerate() {
                let (x, y) = (bx * 4 + p % 4, by * 4 + p / 4);
                if x < w && y < h {
                    img[y * w + x] = 0xFF00_0000 | to8(r) << 16 | to8(g) << 8 | to8(b);
                }
            }
        }
    }
    Ok(())
}

fn image_dimensions(
    data: &[u8],
    w: usize,
    h: usize,
) -> Result<(usize, usize, usize), &'static str> {
    let count = w
        .checked_mul(h)
        .filter(|&n| n > 0 && n <= 64 * 1024 * 1024)
        .ok_or("Image dimensions exceed limit")?;
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let packed = bw
        .checked_mul(bh)
        .and_then(|n| n.checked_mul(16))
        .ok_or("Compressed image size overflow")?;
    if data.len() < packed {
        return Err("Not enough data to decode image!");
    }
    Ok((bw, bh, count))
}

/// Decode to RGB IEEE half-float bit patterns without clamping or reducing HDR
/// precision. Pixels are row-major and edge blocks are cropped to width/height.
/// Reserved block modes decode to black, following the BC6H specification.
pub fn decode_rgb16f(
    data: &[u8],
    w: usize,
    h: usize,
    signed: bool,
) -> Result<Vec<[u16; 3]>, &'static str> {
    let (bw, bh, count) = image_dimensions(data, w, h)?;
    let mut img = vec![[0; 3]; count];
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * 16;
            let px = decode_block(data[o..o + 16].try_into().unwrap(), signed);
            for (p, &rgb) in px.iter().enumerate() {
                let (x, y) = (bx * 4 + p % 4, by * 4 + p / 4);
                if x < w && y < h {
                    img[y * w + x] = rgb;
                }
            }
        }
    }
    Ok(img)
}

pub(crate) fn decode_unsigned(
    data: &[u8],
    w: usize,
    h: usize,
    img: &mut [u32],
) -> Result<(), &'static str> {
    decode_image(data, w, h, img, false)
}
pub(crate) fn decode_signed(
    data: &[u8],
    w: usize,
    h: usize,
    img: &mut [u32],
) -> Result<(), &'static str> {
    decode_image(data, w, h, img, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bc6pack::pack;

    /// One-region block: endpoint fields w, x on all channels; pixel 0 at w, pixel 15 at x, pixel 5 halfway (weight 34).
    fn block(mode: u8, w: u16, x: u16) -> [u8; 16] {
        let mut f = Fields {
            mode,
            ep: [[w; 3], [x; 3], [0; 3], [0; 3]],
            ..Default::default()
        };
        f.idx[15] = 15;
        f.idx[5] = 8;
        pack(&f).unwrap()
    }

    #[test]
    fn unsigned_maximum_is_all_ones_not_half_range() {
        // 10-bit untransformed mode: 1023 is the maximum, 512 is an ordinary value.
        let px = decode_block(&block(10, 1023, 512), false);
        assert_eq!(px[0], [0x7BFF; 3]); // 0xFFFF * 31 >> 6
        let mid = ((512 << 16) + 0x8000) >> 10; // 0x8020
        assert_eq!(px[15], [((mid * 31) >> 6) as u16; 3]); // 0x3E0F, about 1.51
        // 0 .. 512: halfway is about 0.0076, where texture2ddecoder gives about 1.5
        let px = decode_block(&block(10, 0, 512), false);
        let half = (mid * 34 + 32) >> 6;
        assert_eq!(px[5], [((half * 31) >> 6) as u16; 3]);
        let mut img = [0u32; 16];
        decode_image(&block(10, 0, 512), 4, 4, &mut img, false).unwrap();
        assert_eq!(img[5], 0xFF02_0202);
        // same in a 6-bit two-region mode (index 9): 32 is not the maximum, 63 is
        let mut f = Fields {
            mode: 9,
            ep: [[32; 3], [63; 3], [0; 3], [0; 3]],
            ..Default::default()
        };
        f.idx[1] = 7; // partition 0: pixels 0 and 1 are both in region 0
        let px = decode_block(&pack(&f).unwrap(), false);
        assert_eq!(
            px[0],
            [(((((32 << 16) + 0x8000) >> 6) * 31) >> 6) as u16; 3]
        );
        assert_eq!(px[1], [0x7BFF; 3]);
    }

    #[test]
    fn sixteen_bit_mode_keeps_the_second_endpoint() {
        // Mode code 15: 16-bit w, 4-bit signed deltas.
        let px = decode_block(&block(13, 0x2000, 3), false);
        assert_eq!(px[0], [((0x2000 * 31) >> 6) as u16; 3]);
        assert_eq!(px[15], [((0x2003 * 31) >> 6) as u16; 3]); // texture2ddecoder: 0
        // negative delta, and the sum wraps at 16 bits
        assert_eq!(
            decode_block(&block(13, 0x2000, 0xF), false)[15],
            [((0x1FFF * 31) >> 6) as u16; 3]
        );
        assert_eq!(decode_block(&block(13, 0xFFFF, 1), false)[15], [0; 3]);
    }

    #[test]
    fn signed_endpoints() {
        // 10-bit untransformed: 0x200 = -512 and 0x1FF = 511 are the extremes
        let px = decode_block(&block(10, 0x200, 0x1FF), true);
        assert_eq!(px[0], [0xFBFF; 3]);
        assert_eq!(px[15], [0x7BFF; 3]);
        // +1 and -1: ((1 << 15) + 0x4000) >> 9 = 96, 96 * 31 >> 5 = 93
        let px = decode_block(&block(10, 1, 0x3FF), true);
        assert_eq!(px[0], [93; 3]);
        assert_eq!(px[15], [0x8000 | 93; 3]);
        // transformed 11-bit mode (index 11, 9-bit deltas): -3 + 2 = -1
        let px = decode_block(&block(11, 0x7FD, 2), true);
        let q = |a: i32| (((((a << 15) + 0x4000) >> 10) * 31) >> 5) as u16;
        assert_eq!(px[0], [0x8000 | q(3); 3]);
        assert_eq!(px[15], [0x8000 | q(1); 3]);
        // 16-bit mode: values pass through
        let px = decode_block(&block(13, 0xF000, 1), true);
        assert_eq!(px[0], [0x8000 | ((0x1000 * 31) >> 5) as u16; 3]);
        assert_eq!(px[15], [0x8000 | ((0xFFF * 31) >> 5) as u16; 3]);
        // negative values clamp to 0 in the 8-bit image
        let mut img = [0u32; 16];
        decode_image(&block(10, 0x200, 0x1FF), 4, 4, &mut img, true).unwrap();
        assert_eq!((img[0], img[15]), (0xFF00_0000, 0xFFFF_FFFF));
    }

    #[test]
    fn matches_texture2ddecoder_outside_its_bugs() {
        let mut seed = 0x9E37_79B9u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let (mut same, mut total) = (0, 0);
        for _ in 0..4000 {
            let mut blk = [0u8; 16];
            for b in blk.iter_mut() {
                *b = rnd() as u8;
            }
            let mut mine = [0u32; 16];
            decode_image(&blk, 4, 4, &mut mine, false).unwrap();
            let Some(f) = unpack(&blk) else {
                assert_eq!(mine, [0xFF00_0000; 16]);
                continue;
            };
            let m = &MODES[f.mode as usize];
            if m.ep_bits == 16 {
                continue;
            }
            let n = if m.two_regions { 4 } else { 2 };
            let e = endpoints(&f, false);
            if e[..n]
                .iter()
                .flatten()
                .any(|&v| v == 1 << (m.ep_bits - 1) || v == (1 << m.ep_bits) - 1)
            {
                continue;
            }
            let mut theirs = [0u32; 16];
            texture2ddecoder::decode_bc6_block(&blk, &mut theirs, false);
            assert_eq!(mine, theirs, "mode {}", f.mode);
            total += 1;
            same += (mine == theirs) as u32;
        }
        assert!(total > 1000 && same == total);
    }

    #[test]
    fn image_sizes() {
        let blk = block(10, 1023, 1023);
        let data = [blk, blk].concat();
        let mut img = [0u32; 5 * 3];
        decode_image(&data, 5, 3, &mut img, false).unwrap();
        assert!(img.iter().all(|&p| p == 0xFFFF_FFFF));
        assert!(decode_image(&data[..16], 5, 3, &mut img, false).is_err());
    }
}
