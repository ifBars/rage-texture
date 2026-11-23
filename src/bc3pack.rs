// Adapted from holdmysocks/SixNG, commit 7881042c294f3eeea1e8153e7bcafd84f9d787b9.
// Copyright SixNG contributors. SPDX-License-Identifier: GPL-3.0-only
// Source: https://github.com/holdmysocks/SixNG
//! BCPack BC3 (codec 0x0C) chunks -> plain BC3 blocks.
//!
//! The entropy layer is the BC1 one from `bcpack-rs` (12 lanes, cycle-level
//! rANS), with different lane widths and two symbols per lane per block
//! (`target` = blocks x 2; even symbol = E, odd symbol = O):
//!
//! | lane | width | E / O |
//! |---|---|---|
//! | 0 | 4 | back-reference distance, bit-interleaved over E and O |
//! | 1 | 4 | unused (always 0) |
//! | 2 | 8 | zigzag residual of alpha endpoint a0 / a1 |
//! | 3-6 | 6 | alpha index pairs, pixels (0,1) (2,3) ... (14,15) |
//! | 7-9 | 5,6,5 | zigzag residuals of R G B of colour endpoint c0 / c1 |
//! | 10-11 | 8 | colour index quadrants TL, TR (lane 10) and BL, BR (lane 11) |
//!
//! Lane 0: `v = (E & 1) | x << 1` where `x` takes O's bits at even positions
//! and `E >> 1`'s bits at odd positions; the distance is BC1's
//! `unzigzag(v) mod 256` (`x`, or `255 - x` when E is odd). Distance 0
//! predicts colour 0 and alpha 128; a distance past block 0 reads slot
//! `(i - d) mod 256` of a fixed preset table, as in BC1, but BC3 has its own
//! table (colour and alpha). Endpoints are prediction + residual modulo the
//! field width. Index lanes are not predicted: lanes 10-11 index a fixed
//! 2x2-quadrant codebook (BC1 layout, different entries) and lanes 3-6 a
//! fixed table of 3-bit index pairs.
//!
//! The preset table and the colour codebook were fitted against neighbouring
//! mips; slots or codebook symbols that no retail chunk uses are not known
//! and decode as zeros. Blocks come out in stream (Gfx10 tile) order.

use bcpack::rans;

/// `alphabet_bits` (symbol width - 1) of the 12 BC3 lanes, as `bcpack::rans` takes them.
pub const BC3_ALPHABET_BITS: [u8; 12] = [3, 3, 7, 5, 5, 5, 5, 4, 5, 4, 7, 7];
const COLOUR_WIDTHS: [u32; 3] = [5, 6, 5];

include!("bc3pack_tables.rs");

/// Lane 0 (E, O) -> back-reference distance.
#[inline]
pub fn back_distance(e: u16, o: u16) -> usize {
    let mut x = 0usize;
    for k in 0..4 {
        x |= ((o as usize >> k) & 1) << (2 * k);
    }
    for k in 0..3 {
        x |= ((e as usize >> (k + 1)) & 1) << (2 * k + 1);
    }
    if e & 1 != 0 {
        255 - x
    } else {
        x
    }
}

#[inline]
fn unzigzag(v: u16) -> i32 {
    let v = v as i32;
    if v & 1 != 0 {
        -((v + 1) >> 1)
    } else {
        v >> 1
    }
}

/// Decoded endpoint fields of one block: R0 G0 B0 R1 G1 B1, a0 a1.
#[derive(Clone, Copy, Default)]
struct Fields {
    c: [u8; 6],
    a: [u8; 2],
}

/// Result of a BC3 chunk decode.
pub struct Bc3Output {
    /// BC3 blocks, 16 bytes each, stream order.
    pub blocks: Vec<u8>,
    /// Blocks whose reference chain starts at a preset slot not in the table.
    pub unfitted_blocks: usize,
}

/// BCPack BC3 chunk bytes + block count -> BC3 blocks. `None` if the stream
/// does not decode.
pub fn decode(chunk: &[u8], blocks: usize) -> Option<Bc3Output> {
    let lanes = rans::decode_slice(chunk, &BC3_ALPHABET_BITS, 2 * blocks).ok()?;
    let l = |k: usize| lanes.lane(k);
    let (l0, l2, l7, l8, l9, l10, l11) = (l(0), l(2), l(7), l(8), l(9), l(10), l(11));
    let la = [l(3), l(4), l(5), l(6)];
    let mut fields = vec![Fields::default(); blocks];
    let mut tainted = vec![false; blocks];
    let mut out = vec![0u8; blocks * 16];
    let mut unfitted = 0;
    for i in 0..blocks {
        let (e, o) = (2 * i, 2 * i + 1);
        let d = back_distance(l0[e], l0[o]);
        let (pred, t) = if d == 0 {
            (Fields { c: [0; 6], a: [128, 128] }, false)
        } else if d <= i {
            (fields[i - d], tainted[i - d])
        } else {
            let s = (i as isize - d as isize).rem_euclid(256) as usize;
            match PRESET[s] {
                Some((c, a)) => (Fields { c, a }, false),
                None => (Fields::default(), true),
            }
        };
        let res = [l7[e], l8[e], l9[e], l7[o], l8[o], l9[o]];
        let mut f = Fields::default();
        for k in 0..6 {
            let m = (1i32 << COLOUR_WIDTHS[k % 3]) - 1;
            f.c[k] = ((pred.c[k] as i32 + unzigzag(res[k])) & m) as u8;
        }
        f.a[0] = (pred.a[0] as i32 + unzigzag(l2[e])) as u8;
        f.a[1] = (pred.a[1] as i32 + unzigzag(l2[o])) as u8;
        fields[i] = f;
        tainted[i] = t;
        unfitted += t as usize;

        let b = &mut out[i * 16..i * 16 + 16];
        // Alpha block: a0, a1, 48-bit index (pixel p at bits 3p).
        b[0] = f.a[0];
        b[1] = f.a[1];
        let mut aw = 0u64;
        for (k, lane) in la.iter().enumerate() {
            for (h, sym) in [lane[e], lane[o]].into_iter().enumerate() {
                let p = 2 * (2 * k + h);
                let (x, y) = ALPHA_PAIRS[sym as usize & 63];
                aw |= (x as u64) << (3 * p) | (y as u64) << (3 * (p + 1));
            }
        }
        b[2..8].copy_from_slice(&aw.to_le_bytes()[..6]);
        // Colour block: c0, c1 (565), 32-bit index from the quadrant codebook.
        let c0 = (f.c[0] as u16) << 11 | (f.c[1] as u16) << 5 | f.c[2] as u16;
        let c1 = (f.c[3] as u16) << 11 | (f.c[4] as u16) << 5 | f.c[5] as u16;
        b[8..10].copy_from_slice(&c0.to_le_bytes());
        b[10..12].copy_from_slice(&c1.to_le_bytes());
        let quads = [l10[e], l10[o], l11[e], l11[o]];
        let mut cw = 0u32;
        for (q, &sym) in quads.iter().enumerate() {
            let (ox, oy) = [(0, 0), (2, 0), (0, 2), (2, 2)][q];
            let codes = COLOUR_QUADS[sym as usize & 0xFF];
            for (k, &(dx, dy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
                let p = 4 * (oy + dy) + ox + dx;
                cw |= (codes[k] as u32 & 3) << (2 * p);
            }
        }
        b[12..16].copy_from_slice(&cw.to_le_bytes());
    }
    Some(Bc3Output { blocks: out, unfitted_blocks: unfitted })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_bit_layout() {
        assert_eq!(back_distance(0, 0), 0);
        assert_eq!(back_distance(0, 1), 1);
        assert_eq!(back_distance(2, 0), 2);
        assert_eq!(back_distance(0, 2), 4);
        assert_eq!(back_distance(14, 15), 127);
        assert_eq!(back_distance(1, 0), 255);
        assert_eq!(back_distance(15, 15), 128);
    }

    #[test]
    fn alpha_pairs_are_a_permutation() {
        let mut seen = [false; 64];
        for &(x, y) in ALPHA_PAIRS.iter() {
            seen[(x * 8 + y) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }
}
