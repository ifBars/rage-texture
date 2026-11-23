// Adapted from holdmysocks/SixNG, commit 7881042c294f3eeea1e8153e7bcafd84f9d787b9.
// Copyright SixNG contributors. SPDX-License-Identifier: GPL-3.0-only
// Source: https://github.com/holdmysocks/SixNG
//! BCPack BC4 (codec 0x10) and BC5 (codec 0x14) chunks -> plain BC4 / BC5 blocks.
//!
//! Both use the 12-lane entropy layer from `bcpack-rs` with two symbols per
//! lane per 16 bytes of output (E = even symbol, O = odd symbol).
//!
//! BC5, one 16-byte block per E/O pair:
//!
//! | lane | width | E / O |
//! |---|---|---|
//! | 0 | 4 | back-reference distance, bit-interleaved over E and O (as BC3) |
//! | 1 | 4 | unused (always 0) |
//! | 2, 3 | 8 | zigzag residual of channel 0 / 1 endpoint e0 / e1 |
//! | 4-7 | 6 | channel 0 index pairs: lane 4+k E = pixels (4k, 4k+1), O = (4k+2, 4k+3) |
//! | 8-11 | 6 | channel 1 index pairs, same layout |
//!
//! BC4, one 8-byte block per symbol (block 2j is E, block 2j+1 is O):
//!
//! | lane | width | symbol |
//! |---|---|---|
//! | 0 | 8 | back-reference distance, `unzigzag(v) mod 256` |
//! | 1 | 8 | unused (always 0) |
//! | 2, 3 | 8 | zigzag residual of endpoint e0 / e1 |
//! | 4-11 | 6 | index pairs: lane 4+k = pixels (2k, 2k+1) |
//!
//! Distance 0 predicts 128 for every endpoint. A distance up to the block
//! number copies the endpoints of the block that far back; a larger one reads
//! slot `(i - d) mod 256` of a fixed preset table (one table for BC4, another
//! for BC5). Endpoints are prediction + residual modulo 256. Index symbols are
//! not predicted: each maps to a pair of 3-bit indices through a fixed 64-entry
//! table (again one per format).
//!
//! The tables were fitted against neighbouring mips. Preset slots no retail
//! chunk lets us fit decode as zeros and the block counts as unfitted. Blocks
//! come out in stream (Gfx10 tile) order.

use bcpack::rans;

/// `alphabet_bits` (symbol width - 1) of the 12 BC4 lanes.
pub const BC4_ALPHABET_BITS: [u8; 12] = [7, 7, 7, 7, 5, 5, 5, 5, 5, 5, 5, 5];
/// `alphabet_bits` (symbol width - 1) of the 12 BC5 lanes.
pub const BC5_ALPHABET_BITS: [u8; 12] = [3, 3, 7, 7, 5, 5, 5, 5, 5, 5, 5, 5];

include!("bc45pack_tables.rs");

#[inline]
fn unzigzag(v: u16) -> i32 {
    let v = v as i32;
    if v & 1 != 0 {
        -((v + 1) >> 1)
    } else {
        v >> 1
    }
}

/// Result of a BC4 or BC5 chunk decode.
pub struct Bc45Output {
    /// BC4 (8 bytes) or BC5 (16 bytes) blocks, stream order.
    pub blocks: Vec<u8>,
    /// Blocks whose reference chain starts at a preset slot not in the table.
    pub unfitted_blocks: usize,
}

/// One BC4 block: endpoints and 16 3-bit indices (pixel p at bits 3p).
fn put_bc4(b: &mut [u8], e: [u8; 2], idx: u64) {
    b[0] = e[0];
    b[1] = e[1];
    b[2..8].copy_from_slice(&idx.to_le_bytes()[..6]);
}

/// Prediction source for block `i` at distance `d`: `Ok(Some(j))` = block j,
/// `Ok(None)` = the 128 predictor, `Err(slot)` = preset slot.
#[inline]
fn source(i: usize, d: usize) -> Result<Option<usize>, usize> {
    if d == 0 {
        Ok(None)
    } else if d <= i {
        Ok(Some(i - d))
    } else {
        Err((i as isize - d as isize).rem_euclid(256) as usize)
    }
}

/// BCPack BC4 chunk bytes + block count (8-byte blocks) -> BC4 blocks.
/// `None` if the stream does not decode.
pub fn decode_bc4(chunk: &[u8], blocks: usize) -> Option<Bc45Output> {
    let lanes = rans::decode_slice(chunk, &BC4_ALPHABET_BITS, blocks).ok()?;
    let l: Vec<&[u16]> = (0..12).map(|k| lanes.lane(k)).collect();
    let mut ep = vec![[0u8; 2]; blocks];
    let mut tainted = vec![false; blocks];
    let mut out = vec![0u8; blocks * 8];
    let mut unfitted = 0;
    for i in 0..blocks {
        let (pred, t) = match source(i, unzigzag(l[0][i]).rem_euclid(256) as usize) {
            Ok(None) => ([128, 128], false),
            Ok(Some(j)) => (ep[j], tainted[j]),
            Err(s) => match BC4_PRESET[s] {
                Some(v) => (v, false),
                None => ([0, 0], true),
            },
        };
        let e = [
            (pred[0] as i32 + unzigzag(l[2][i])) as u8,
            (pred[1] as i32 + unzigzag(l[3][i])) as u8,
        ];
        ep[i] = e;
        tainted[i] = t;
        unfitted += t as usize;
        let mut idx = 0u64;
        for k in 0..8 {
            let (x, y) = BC4_PAIRS[l[4 + k][i] as usize & 63];
            idx |= (x as u64) << (6 * k) | (y as u64) << (6 * k + 3);
        }
        put_bc4(&mut out[i * 8..i * 8 + 8], e, idx);
    }
    Some(Bc45Output { blocks: out, unfitted_blocks: unfitted })
}

/// BCPack BC5 chunk bytes + block count (16-byte blocks) -> BC5 blocks.
/// `None` if the stream does not decode.
pub fn decode_bc5(chunk: &[u8], blocks: usize) -> Option<Bc45Output> {
    let lanes = rans::decode_slice(chunk, &BC5_ALPHABET_BITS, 2 * blocks).ok()?;
    let l: Vec<&[u16]> = (0..12).map(|k| lanes.lane(k)).collect();
    let mut ep = vec![[[0u8; 2]; 2]; blocks];
    let mut tainted = vec![false; blocks];
    let mut out = vec![0u8; blocks * 16];
    let mut unfitted = 0;
    for i in 0..blocks {
        let (e, o) = (2 * i, 2 * i + 1);
        let d = crate::bc3pack::back_distance(l[0][e], l[0][o]);
        let (pred, t) = match source(i, d) {
            Ok(None) => ([[128, 128]; 2], false),
            Ok(Some(j)) => (ep[j], tainted[j]),
            Err(s) => match BC5_PRESET[s] {
                Some(v) => (v, false),
                None => ([[0, 0]; 2], true),
            },
        };
        tainted[i] = t;
        unfitted += t as usize;
        for ch in 0..2 {
            let r = l[2 + ch];
            let v = [
                (pred[ch][0] as i32 + unzigzag(r[e])) as u8,
                (pred[ch][1] as i32 + unzigzag(r[o])) as u8,
            ];
            ep[i][ch] = v;
            let mut idx = 0u64;
            for k in 0..4 {
                for (h, sym) in [l[4 + 4 * ch + k][e], l[4 + 4 * ch + k][o]].into_iter().enumerate() {
                    let p = 2 * (2 * k + h);
                    let (x, y) = BC5_PAIRS[sym as usize & 63];
                    idx |= (x as u64) << (3 * p) | (y as u64) << (3 * (p + 1));
                }
            }
            put_bc4(&mut out[i * 16 + 8 * ch..i * 16 + 8 * ch + 8], v, idx);
        }
    }
    Some(Bc45Output { blocks: out, unfitted_blocks: unfitted })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_permutation(t: &[(u8, u8); 64]) -> bool {
        let mut seen = [false; 64];
        for &(x, y) in t.iter() {
            seen[(x * 8 + y) as usize] = true;
        }
        seen.iter().all(|&s| s)
    }

    #[test]
    fn pair_tables_are_permutations() {
        assert!(is_permutation(&BC4_PAIRS));
        assert!(is_permutation(&BC5_PAIRS));
    }

    #[test]
    fn bc4_distance_mapping() {
        let d = |v: u16| unzigzag(v).rem_euclid(256) as usize;
        assert_eq!(d(0), 0);
        assert_eq!(d(2), 1);
        assert_eq!(d(4), 2);
        assert_eq!(d(1), 255);
        assert_eq!(d(3), 254);
    }

    #[test]
    fn source_rules() {
        assert_eq!(source(5, 0), Ok(None));
        assert_eq!(source(5, 5), Ok(Some(0)));
        assert_eq!(source(5, 6), Err(255));
        assert_eq!(source(300, 301), Err(255));
    }

    #[test]
    fn zero_streams_do_not_panic() {
        let _ = decode_bc4(&[0u8; 64], 16);
        let _ = decode_bc5(&[0u8; 64], 16);
    }
}
