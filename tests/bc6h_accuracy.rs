//! Independent BC6H accuracy regressions for the public `rage_texture::bc6h` API.
//!
//! Source credit: the decoder under test is an attributed, GPL-3.0-only
//! adaptation of SixNG's upstream work in PR #23
//! (<https://github.com/holdmysocks/SixNG/pull/23>, revision
//! `2979c57fdeb35863bb8885e6b48d09090cd1f7de`). The vectors below are *not*
//! generated from that implementation: they are derived from the BC6H
//! specification (Microsoft, "BC6H format": `unquantize`, `finish_unquantize`,
//! weights `0,9,18,27,37,46,55,64` / `0,4,9,13,17,21,26,30,34,38,43,47,51,55,
//! 60,64`, reserved mode codes = zero) and cross-checked against the
//! independent `bcdec.h` reference decoder. Blocks are written as raw bytes by
//! the small spec-order bit writer below, so these tests exercise the public
//! API only and never the crate-private `bc6pack` helpers.

use rage_texture::bc6h::{decode_block, decode_image, decode_rgb16f};

/// Spec weight tables for 3-bit (two-region) and 4-bit (one-region) indices.
const W4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// Little-endian, LSB-first bit writer: the BC6H block is a 128-bit little
/// endian integer whose fields are read from the least significant bit up.
#[derive(Default)]
struct Bits {
    value: u128,
    bits: u32,
}

impl Bits {
    fn put(&mut self, value: u32, width: u32) {
        debug_assert!(width == 0 || value < (1u32 << width));
        self.value |= ((value as u128) & ((1u128 << width) - 1)) << self.bits;
        self.bits += width;
    }

    fn block(self) -> [u8; 16] {
        assert_eq!(self.bits, 128, "BC6H blocks are exactly 128 bits");
        self.value.to_le_bytes()
    }
}

/// Spec mode 11 (stored code `0x03`): one region, untransformed, 10-bit
/// endpoints, 4-bit indices with a 3-bit anchor at pixel 0.
fn mode_11_block(w: [u16; 3], x: [u16; 3], idx: [u8; 16]) -> [u8; 16] {
    let mut b = Bits::default();
    b.put(0x03, 5);
    for &value in &w {
        b.put(value as u32, 10);
    }
    for &value in &x {
        b.put(value as u32, 10);
    }
    b.put(idx[0] as u32, 3);
    for &index in &idx[1..] {
        b.put(index as u32, 4);
    }
    b.block()
}

/// Spec mode 14 (stored code `0x0f`): one region, transformed, 16-bit `w`
/// (low 10 bits first, then bits 15..10 highest-first) and 4-bit signed deltas.
fn mode_14_block(w: [u16; 3], delta: [u16; 3], idx: [u8; 16]) -> [u8; 16] {
    let mut b = Bits::default();
    b.put(0x0f, 5);
    for &value in &w {
        b.put((value & 0x03ff) as u32, 10);
    }
    for (&value, &delta) in w.iter().zip(delta.iter()) {
        b.put((delta & 0x0f) as u32, 4);
        for shift in [15, 14, 13, 12, 11, 10] {
            b.put(((value >> shift) & 1) as u32, 1);
        }
    }
    b.put(idx[0] as u32, 3);
    for &index in &idx[1..] {
        b.put(index as u32, 4);
    }
    b.block()
}

/// `unquantize` from the specification (UF16 / SF16).
fn unquantize(x: i32, bits: u32, signed: bool) -> i32 {
    if signed {
        if bits >= 16 {
            return x;
        }
        let magnitude = x.abs();
        let unq = if magnitude == 0 {
            0
        } else if magnitude >= (1 << (bits - 1)) - 1 {
            0x7fff
        } else {
            ((magnitude << 15) + 0x4000) >> (bits - 1)
        };
        if x < 0 { -unq } else { unq }
    } else if bits >= 15 {
        x
    } else if x == 0 {
        0
    } else if x == (1 << bits) - 1 {
        0xffff
    } else {
        ((x << 15) + 0x4000) >> (bits - 1)
    }
}

/// `finish_unquantize` from the specification: scale the magnitude into the
/// signed value first, then set the half-float sign bit only if the *scaled*
/// value is negative. A negative interpolant that truncates to zero therefore
/// produces positive zero (0x0000), never negative zero (0x8000).
fn finish(v: i32, signed: bool) -> u16 {
    if !signed {
        return ((v * 31) >> 6) as u16;
    }
    let scaled = if v < 0 {
        -(((-v) * 31) >> 5)
    } else {
        (v * 31) >> 5
    };
    if scaled < 0 {
        0x8000 | (-scaled) as u16
    } else {
        scaled as u16
    }
}

fn interpolate(a: i32, b: i32, weight: i32) -> i32 {
    (a * (64 - weight) + b * weight + 32) >> 6
}

/// Sign-extend a stored endpoint field of `bits` bits.
fn sext(value: u16, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value as i32) << shift) >> shift
}

#[test]
fn unsigned_half_range_is_an_ordinary_value_not_the_maximum() {
    // texture2ddecoder 0.1.2 maps 1 << (bits - 1) to 0xFFFF. The specification
    // only special-cases (1 << bits) - 1, so 512 of 10 bits is ordinary and
    // must finish to 0x3E0F (about 1.51), not 0x7BFF (65504).
    let mid = unquantize(512, 10, false);
    assert_eq!(finish(mid, false), 0x3e0f);

    let mut idx = [0u8; 16];
    idx[5] = 8; // weight 34
    idx[15] = 15;
    let flat = decode_block(&mode_11_block([512; 3], [512; 3], idx), false);
    assert_eq!(flat, [[0x3e0f; 3]; 16]);

    // 0 .. 512: pixel 0 is zero, pixel 15 the half-range endpoint, pixel 5 the
    // weight-34 interpolant. None of them is 0x7BFF.
    let ramp = decode_block(&mode_11_block([0; 3], [512; 3], idx), false);
    assert_eq!(ramp[0], [0x0000; 3]);
    assert_eq!(ramp[15], [0x3e0f; 3]);
    let expected = finish(interpolate(0, mid, W4[8]), false);
    assert_eq!(ramp[5], [expected; 3]);
    assert!(ramp.iter().flatten().all(|&channel| channel != 0x7bff));

    // The eight-bit preview agrees with the half-float path. Half-range
    // finishes to 0x3E0F (about 1.51), so the clamped preview is full scale;
    // the weight-34 interpolant is about 0.0097, i.e. level 2.
    let mut image = [0u32; 16];
    decode_image(
        &mode_11_block([0; 3], [512; 3], idx),
        4,
        4,
        &mut image,
        false,
    )
    .unwrap();
    assert_eq!(image[5], 0xff02_0202);
    assert_eq!(image[15], 0xffff_ffff);
}

#[test]
fn unsigned_maximum_maps_to_the_maximum_half() {
    // (1 << bits) - 1 is the only value that unquantizes to 0xFFFF.
    let max = unquantize(1023, 10, false);
    assert_eq!(max, 0xffff);
    assert_eq!(finish(max, false), 0x7bff);

    let mut idx = [0u8; 16];
    idx[5] = 8;
    idx[15] = 15;
    let block = decode_block(&mode_11_block([1023; 3], [0; 3], idx), false);
    assert_eq!(block[0], [0x7bff; 3]);
    assert_eq!(block[15], [0x0000; 3]);
    assert_eq!(block[5], [finish(interpolate(max, 0, W4[8]), false); 3]);

    let mut image = [0u32; 16];
    decode_image(
        &mode_11_block([1023; 3], [1023; 3], [0; 16]),
        4,
        4,
        &mut image,
        false,
    )
    .unwrap();
    assert_eq!(image, [0xffff_ffff; 16]);
}

#[test]
fn sixteen_bit_mode_keeps_the_second_endpoint_and_wraps() {
    // texture2ddecoder builds the 16-bit endpoint mask in u16, which is zero in
    // release builds, so its second endpoint collapses to 0.
    let mut idx = [0u8; 16];
    idx[15] = 15;
    let block = decode_block(&mode_14_block([0x2000; 3], [3; 3], idx), false);
    assert_eq!(block[0], [finish(0x2000, false); 3]);
    assert_eq!(block[15], [finish(0x2003, false); 3]);
    assert_eq!(block[0], [0x0f80; 3]);
    assert_eq!(block[15], [0x0f81; 3]);

    // 0xFFFF + 1 wraps to zero at 16 bits (masked transformed endpoint).
    let wrapped = decode_block(&mode_14_block([0xffff; 3], [1; 3], idx), false);
    assert_eq!(wrapped[15], [0x0000; 3]);

    // Negative delta: 0x2000 - 1.
    let negative = decode_block(&mode_14_block([0x2000; 3], [0x0f; 3], idx), false);
    assert_eq!(negative[15], [finish(0x1fff, false); 3]);
}

#[test]
fn signed_endpoints_use_two_s_complement_extremes() {
    // 0x200 (=-512) and 0x1FF (=+511) are the 10-bit extremes. Both magnitudes
    // reach the specification's 0x7FFF clamp.
    let mut idx = [0u8; 16];
    idx[15] = 15;
    let extremes = decode_block(&mode_11_block([0x200; 3], [0x1ff; 3], idx), true);
    assert_eq!(extremes[0], [0xfbff; 3]);
    assert_eq!(extremes[15], [0x7bff; 3]);
    assert_eq!(
        extremes[0][0],
        finish(unquantize(sext(0x200, 10), 10, true), true)
    );

    // +1 and -1 are ordinary small magnitudes: 96 after unquantize.
    let small = decode_block(&mode_11_block([1; 3], [0x3ff; 3], idx), true);
    assert_eq!(small[0], [0x005d; 3]);
    assert_eq!(small[15], [0x805d; 3]);

    // 16-bit transformed mode passes values through unquantize, then scales.
    let wide = decode_block(&mode_14_block([0xf000; 3], [1; 3], idx), true);
    assert_eq!(wide[0], [0x8f80; 3]);
    assert_eq!(wide[15], [0x8f7f; 3]);

    // The eight-bit preview clamps the negative endpoint to zero and the
    // positive one to 255.
    let mut image = [0u32; 16];
    decode_image(
        &mode_11_block([0x200; 3], [0x1ff; 3], idx),
        4,
        4,
        &mut image,
        true,
    )
    .unwrap();
    assert_eq!(image[0], 0xff00_0000);
    assert_eq!(image[15], 0xffff_ffff);
}

#[test]
fn canonical_zero_for_sub_unit_negative_signed_interpolants() {
    // Differential regression from an independent bcdec oracle. This block's
    // pixel 2, G channel interpolates to a negative value whose scaled
    // magnitude truncates to zero. The specification's finish_unquantize order
    // (scale first, then sign) yields 0x0000; checking the sign before scaling
    // yields negative zero (0x8000), which the oracle flagged as a mismatch.
    let block: [u8; 16] = [
        0x40, 0xb1, 0x03, 0xc8, 0xf6, 0x86, 0xae, 0x69, 0x17, 0xd5, 0x16, 0xba, 0x88, 0x58, 0xfd,
        0xe7,
    ];
    for signed in [true, false] {
        let pixels = decode_block(&block, signed);
        // After scaling, a zero magnitude is positive zero for both formats;
        // negative zero must never be emitted.
        assert!(
            !pixels.iter().flatten().any(|&half| half == 0x8000),
            "negative zero emitted for signed={signed}"
        );
    }
    let pixels = decode_block(&block, true);
    assert_eq!(pixels[2][1], 0x0000);
}

#[test]
fn reserved_modes_decode_to_black_and_ignore_the_rest_of_the_block() {
    // Reachable reserved stored codes: 19, 23, 27 and 31 (2-bit prefix 0b11).
    for code in [0x13u8, 0x17, 0x1b, 0x1f] {
        let mut block = [0xa5u8; 16];
        block[0] = code;
        for signed in [false, true] {
            assert_eq!(
                decode_block(&block, signed),
                [[0u16; 3]; 16],
                "code {code:#04x}"
            );
            let mut image = [0u32; 16];
            decode_image(&block, 4, 4, &mut image, signed).unwrap();
            assert_eq!(image, [0xff00_0000; 16]);
            let rgb = decode_rgb16f(&block, 4, 4, signed).unwrap();
            assert_eq!(rgb, vec![[0u16; 3]; 16]);
        }
    }
}

#[test]
fn image_dimensions_and_edge_cropping_are_explicit() {
    let max_block = mode_11_block([1023; 3], [1023; 3], [0; 16]);

    // Full-precision output keeps HDR (0x7BFF, 65504) instead of clamping to 1.0
    // (0x3C00); the eight-bit image path clamps.
    let rgb = decode_rgb16f(&[max_block; 2].concat(), 5, 3, false).unwrap();
    assert_eq!(rgb.len(), 15);
    assert!(rgb.iter().all(|pixel| *pixel == [0x7bff; 3]));
    let mut image = [0u32; 15];
    decode_image(&[max_block; 2].concat(), 5, 3, &mut image, false).unwrap();
    assert_eq!(image, [0xffff_ffff; 15]);

    // Row-major cropping: a 3x3 window uses block pixels 0,1,2,4,5,6,8,9,10.
    let mut indices = [0u8; 16];
    for (p, index) in indices.iter_mut().enumerate() {
        *index = p as u8;
    }
    let gradient = mode_11_block([1023; 3], [0; 3], indices);
    let cropped = decode_rgb16f(&gradient, 3, 3, false).unwrap();
    let full = decode_block(&gradient, false);
    let expected_pixels = [0usize, 1, 2, 4, 5, 6, 8, 9, 10];
    for (slot, &p) in expected_pixels.iter().enumerate() {
        assert_eq!(
            cropped[slot], full[p],
            "crop slot {slot} from block pixel {p}"
        );
    }

    // Rejected shapes and buffers.
    assert!(decode_rgb16f(&gradient, 0, 4, false).is_err());
    assert!(decode_rgb16f(&gradient, 4, 0, false).is_err());
    assert!(decode_image(&gradient, 0, 0, &mut [0u32; 1], false).is_err());
    // 8193 * 8193 = 67_125_249 pixels, above the 64 MiB-pixel limit; the check
    // runs before any allocation.
    assert!(decode_rgb16f(&gradient, 8193, 8193, false).is_err());
    // One block is not enough for 5x3.
    assert!(decode_rgb16f(&gradient, 5, 3, false).is_err());
    assert!(decode_image(&gradient, 5, 3, &mut [0u32; 15], false).is_err());
    // The image buffer must hold every pixel.
    assert!(decode_image(&gradient, 3, 3, &mut [0u32; 8], false).is_err());
    assert!(decode_image(&gradient, 3, 3, &mut [0u32; 9], false).is_ok());
}
