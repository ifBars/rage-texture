// Adapted from holdmysocks/SixNG, commit 7881042c294f3eeea1e8153e7bcafd84f9d787b9.
// Copyright SixNG contributors. SPDX-License-Identifier: GPL-3.0-only
// Source: https://github.com/holdmysocks/SixNG
//! BCPack BC6H (codec 0x18) chunks -> plain BC6H blocks.
//!
//! The first half of this file is the plain BC6H block codec (fields <->
//! 128-bit block, and a pixel decode for checks). The BCPack layer follows.
//!
//! The entropy layer is the one from `bcpack-rs`: 16 lanes, widths
//! 4,4,8,8,8,5,5,5,6,6,6,8,8,8,8,5, two symbols per lane per block (E = even
//! symbol, O = odd symbol). As in BC7, each block carries a state that later
//! blocks predict from:
//!
//! - **References.** `il(e, o)` interleaves two values (E bits on even
//!   positions). Index distance `di = il(E1, O1)`, endpoint distance
//!   `de = (di + unzigzag(il(E0, O0))) & 255`. `d = 0` predicts half range,
//!   `d <= i` predicts from block `i - d`, and `d > i` reads slot
//!   `(i - d) mod 256` of a fixed preset table (one for endpoints, one for
//!   indices).
//! - **Header.** Lane 15: E = mode (0..14 in the standard order), O = partition.
//! - **Endpoints.** The state holds the stored fields w, x, y, z per channel,
//!   as the block stores them (x, y, z are deltas in transformed modes),
//!   16-bit and MSB-aligned. A field of `b` bits is the top `b` bits of its
//!   state plus a zigzag residual, modulo `2^b`. Residuals: w = `il(E, O)` of
//!   lanes 2-4, x = `il(E, O)` of lanes 5-7, y = E and z = O of lanes 8-10.
//!   Modes with a 12- or 16-bit w keep bits 10.. of w reversed relative to
//!   the block field (the BC6H layout stores them that way). One-region
//!   blocks set z to x. For y, untransformed modes copy w; transformed modes
//!   write w into the y field's (delta) width, offset by half its range, so
//!   a two-region block that follows predicts y from the low bits of w.
//! - **Indices.** As BC7: lanes 11-14 hold rows 0-3, each symbol two zigzag
//!   deltas with interleaved bits (E = columns 0-1, O = columns 2-3; rows 0-2
//!   put the left pixel on the odd bits, row 3 on the even bits), on a 4-bit
//!   MSB-aligned state (3-bit reads in two-region modes).
//!
//! The preset tables were fitted against neighbouring mips. Slots no retail
//! chunk lets us fit start from half range and count as unfitted. Blocks come
//! out in stream (Gfx10 tile) order.

/// Per-mode facts, indexed by mode number 0..14 (BC6H modes 1..14).
pub struct ModeInfo {
    /// Mode bits as stored (2 bits for the first two modes, 5 for the rest).
    pub code: u8,
    pub code_bits: u32,
    /// Bits of the base endpoint (w) per channel.
    pub ep_bits: u32,
    /// Bits of the other stored endpoints (x, y, z) per channel R, G, B.
    pub delta_bits: [u32; 3],
    /// Other endpoints are stored as signed deltas from w.
    pub transformed: bool,
    /// Two regions (partition id + 3-bit indices) or one (4-bit indices).
    pub two_regions: bool,
}

const fn mi(code: u8, ep_bits: u32, d: [u32; 3], transformed: bool, two_regions: bool) -> ModeInfo {
    ModeInfo { code, code_bits: if code < 2 { 2 } else { 5 }, ep_bits, delta_bits: d, transformed, two_regions }
}

pub const MODES: [ModeInfo; 14] = [
    mi(0, 10, [5, 5, 5], true, true),
    mi(1, 7, [6, 6, 6], true, true),
    mi(2, 11, [5, 4, 4], true, true),
    mi(6, 11, [4, 5, 4], true, true),
    mi(10, 11, [4, 4, 5], true, true),
    mi(14, 9, [5, 5, 5], true, true),
    mi(18, 8, [6, 5, 5], true, true),
    mi(22, 8, [5, 6, 5], true, true),
    mi(26, 8, [5, 5, 6], true, true),
    mi(30, 6, [6, 6, 6], false, true),
    mi(3, 10, [10, 10, 10], false, false),
    mi(7, 11, [9, 9, 9], true, false),
    mi(11, 12, [8, 8, 8], true, false),
    mi(15, 16, [4, 4, 4], true, false),
];

// Endpoint bit layout after the mode bits: (bits, channel, endpoint, shift),
// read LSB first. Channels R G B = 0 1 2; endpoints w x y z = 0 1 2 3.
type Op = (u8, u8, u8, u8);
const L0: &[Op] = &[(1,1,2,4),(1,2,2,4),(1,2,3,4),(10,0,0,0),(10,1,0,0),(10,2,0,0),(5,0,1,0),(1,1,3,4),(4,1,2,0),(5,1,1,0),(1,2,3,0),(4,1,3,0),(5,2,1,0),(1,2,3,1),(4,2,2,0),(5,0,2,0),(1,2,3,2),(5,0,3,0),(1,2,3,3)];
const L1: &[Op] = &[(1,1,2,5),(1,1,3,4),(1,1,3,5),(7,0,0,0),(1,2,3,0),(1,2,3,1),(1,2,2,4),(7,1,0,0),(1,2,2,5),(1,2,3,2),(1,1,2,4),(7,2,0,0),(1,2,3,3),(1,2,3,5),(1,2,3,4),(6,0,1,0),(4,1,2,0),(6,1,1,0),(4,1,3,0),(6,2,1,0),(4,2,2,0),(6,0,2,0),(6,0,3,0)];
const L2: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(5,0,1,0),(1,0,0,10),(4,1,2,0),(4,1,1,0),(1,1,0,10),(1,2,3,0),(4,1,3,0),(4,2,1,0),(1,2,0,10),(1,2,3,1),(4,2,2,0),(5,0,2,0),(1,2,3,2),(5,0,3,0),(1,2,3,3)];
const L6: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(4,0,1,0),(1,0,0,10),(1,1,3,4),(4,1,2,0),(5,1,1,0),(1,1,0,10),(4,1,3,0),(4,2,1,0),(1,2,0,10),(1,2,3,1),(4,2,2,0),(4,0,2,0),(1,2,3,0),(1,2,3,2),(4,0,3,0),(1,1,2,4),(1,2,3,3)];
const L10: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(4,0,1,0),(1,0,0,10),(1,2,2,4),(4,1,2,0),(4,1,1,0),(1,1,0,10),(1,2,3,0),(4,1,3,0),(5,2,1,0),(1,2,0,10),(4,2,2,0),(4,0,2,0),(1,2,3,1),(1,2,3,2),(4,0,3,0),(1,2,3,4),(1,2,3,3)];
const L14: &[Op] = &[(9,0,0,0),(1,2,2,4),(9,1,0,0),(1,1,2,4),(9,2,0,0),(1,2,3,4),(5,0,1,0),(1,1,3,4),(4,1,2,0),(5,1,1,0),(1,2,3,0),(4,1,3,0),(5,2,1,0),(1,2,3,1),(4,2,2,0),(5,0,2,0),(1,2,3,2),(5,0,3,0),(1,2,3,3)];
const L18: &[Op] = &[(8,0,0,0),(1,1,3,4),(1,2,2,4),(8,1,0,0),(1,2,3,2),(1,1,2,4),(8,2,0,0),(1,2,3,3),(1,2,3,4),(6,0,1,0),(4,1,2,0),(5,1,1,0),(1,2,3,0),(4,1,3,0),(5,2,1,0),(1,2,3,1),(4,2,2,0),(6,0,2,0),(6,0,3,0)];
const L22: &[Op] = &[(8,0,0,0),(1,2,3,0),(1,2,2,4),(8,1,0,0),(1,1,2,5),(1,1,2,4),(8,2,0,0),(1,1,3,5),(1,2,3,4),(5,0,1,0),(1,1,3,4),(4,1,2,0),(6,1,1,0),(4,1,3,0),(5,2,1,0),(1,2,3,1),(4,2,2,0),(5,0,2,0),(1,2,3,2),(5,0,3,0),(1,2,3,3)];
const L26: &[Op] = &[(8,0,0,0),(1,2,3,1),(1,2,2,4),(8,1,0,0),(1,2,2,5),(1,1,2,4),(8,2,0,0),(1,2,3,5),(1,2,3,4),(5,0,1,0),(1,1,3,4),(4,1,2,0),(5,1,1,0),(1,2,3,0),(4,1,3,0),(6,2,1,0),(4,2,2,0),(5,0,2,0),(1,2,3,2),(5,0,3,0),(1,2,3,3)];
const L30: &[Op] = &[(6,0,0,0),(1,1,3,4),(1,2,3,0),(1,2,3,1),(1,2,2,4),(6,1,0,0),(1,1,2,5),(1,2,2,5),(1,2,3,2),(1,1,2,4),(6,2,0,0),(1,1,3,5),(1,2,3,3),(1,2,3,5),(1,2,3,4),(6,0,1,0),(4,1,2,0),(6,1,1,0),(4,1,3,0),(6,2,1,0),(4,2,2,0),(6,0,2,0),(6,0,3,0)];
const L3: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(10,0,1,0),(10,1,1,0),(10,2,1,0)];
const L7: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(9,0,1,0),(1,0,0,10),(9,1,1,0),(1,1,0,10),(9,2,1,0),(1,2,0,10)];
const L11: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(8,0,1,0),(1,0,0,11),(1,0,0,10),(8,1,1,0),(1,1,0,11),(1,1,0,10),(8,2,1,0),(1,2,0,11),(1,2,0,10)];
const L15: &[Op] = &[(10,0,0,0),(10,1,0,0),(10,2,0,0),(4,0,1,0),(1,0,0,15),(1,0,0,14),(1,0,0,13),(1,0,0,12),(1,0,0,11),(1,0,0,10),(4,1,1,0),(1,1,0,15),(1,1,0,14),(1,1,0,13),(1,1,0,12),(1,1,0,11),(1,1,0,10),(4,2,1,0),(1,2,0,15),(1,2,0,14),(1,2,0,13),(1,2,0,12),(1,2,0,11),(1,2,0,10)];
const LAYOUTS: [&[Op]; 14] = [L0, L1, L2, L6, L10, L14, L18, L22, L26, L30, L3, L7, L11, L15];

/// One BC6H block in stored terms.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Fields {
    /// Mode number 0..14.
    pub mode: u8,
    /// Partition 0..32 (two-region modes).
    pub partition: u8,
    /// Stored endpoint fields [w, x, y, z][R, G, B], as the block holds them
    /// (x, y, z are deltas in transformed modes).
    pub ep: [[u16; 3]; 4],
    /// Indices in pixel order (anchors have one bit less).
    pub idx: [u8; 16],
}

/// Second anchor pixel of a two-region partition.
fn anchor2(part: u8) -> usize { crate::bc7pack::ANCHOR2[part as usize & 31] as usize }
/// Region of pixel `p` in a two-region partition.
fn region(part: u8, p: usize) -> usize { (crate::bc7pack::PART2[part as usize & 31] >> (2 * p) & 3) as usize }

struct Bits { v: u128, n: u32 }
impl Bits {
    fn put(&mut self, x: u32, bits: u32) { if bits > 0 { self.v |= ((x as u128) & ((1u128 << bits) - 1)) << self.n; self.n += bits; } }
    fn get(&mut self, bits: u32) -> u32 { let x = ((self.v >> self.n) & ((1u128 << bits) - 1)) as u32; self.n += bits; x }
}

/// Fields -> block. Values wider than their field are truncated; `None` for a bad mode.
pub fn pack(f: &Fields) -> Option<[u8; 16]> {
    let m = MODES.get(f.mode as usize)?;
    let mut b = Bits { v: 0, n: 0 };
    b.put(m.code as u32, m.code_bits);
    for &(bits, ch, e, sh) in LAYOUTS[f.mode as usize] { b.put((f.ep[e as usize][ch as usize] >> sh) as u32, bits as u32); }
    let ib = if m.two_regions { b.put(f.partition as u32, 5); 3 } else { 4 };
    let a2 = if m.two_regions { anchor2(f.partition) } else { 99 };
    for p in 0..16 { b.put(f.idx[p] as u32, if p == 0 || p == a2 { ib - 1 } else { ib }); }
    debug_assert_eq!(b.n, 128);
    Some(b.v.to_le_bytes())
}

/// Block -> fields; `None` for the reserved mode codes.
pub fn unpack(block: &[u8; 16]) -> Option<Fields> {
    let mut b = Bits { v: u128::from_le_bytes(*block), n: 0 };
    let c2 = b.get(2) as u8;
    let code = if c2 < 2 { c2 } else { c2 | (b.get(3) as u8) << 2 };
    let mode = MODES.iter().position(|m| m.code == code)? as u8;
    let m = &MODES[mode as usize];
    let mut f = Fields { mode, ..Default::default() };
    for &(bits, ch, e, sh) in LAYOUTS[mode as usize] { f.ep[e as usize][ch as usize] |= (b.get(bits as u32) as u16) << sh; }
    let ib = if m.two_regions { f.partition = b.get(5) as u8; 3 } else { 4 };
    let a2 = if m.two_regions { anchor2(f.partition) } else { 99 };
    for p in 0..16 { f.idx[p] = b.get(if p == 0 || p == a2 { ib - 1 } else { ib }) as u8; }
    Some(f)
}

fn sext(v: u16, bits: u32) -> i32 { let s = 32 - bits; ((v as i32) << s) >> s }

/// Endpoints as unsigned values of `ep_bits` bits [w, x, y, z][RGB] (deltas resolved), unsigned formats.
pub fn endpoints(f: &Fields) -> [[u32; 3]; 4] {
    let m = &MODES[f.mode as usize];
    let n = if m.two_regions { 4 } else { 2 };
    let mut e = [[0u32; 3]; 4];
    let mask = (1u32 << m.ep_bits) - 1;
    for ch in 0..3 {
        e[0][ch] = f.ep[0][ch] as u32 & mask;
        for k in 1..n {
            e[k][ch] = if m.transformed {
                ((e[0][ch] as i32 + sext(f.ep[k][ch], m.delta_bits[ch])) as u32) & mask
            } else { f.ep[k][ch] as u32 & mask };
        }
    }
    e
}

fn unquantize(x: u32, bits: u32) -> u32 {
    if bits >= 15 { x } else if x == 0 { 0 } else if x == (1 << bits) - 1 { 0xFFFF } else { ((x << 16) + 0x8000) >> bits }
}

fn half_to_f32(h: u16) -> f32 {
    let e = (h >> 10) & 31; let m = (h & 1023) as f32;
    if e == 0 { m * (1.0 / 16_777_216.0) } else if e == 31 { 65504.0 } else { (1.0 + m / 1024.0) * 2f32.powi(e as i32 - 15) }
}

/// Unsigned (UF16) block -> 16 RGB pixels as f32.
pub fn decode_pixels(f: &Fields) -> [[f32; 3]; 16] {
    const W3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
    const W4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
    let m = &MODES[f.mode as usize];
    let e = endpoints(f);
    let mut out = [[0f32; 3]; 16];
    for p in 0..16 {
        let (r, w) = if m.two_regions { (region(f.partition, p), W3[f.idx[p] as usize & 7]) } else { (0, W4[f.idx[p] as usize & 15]) };
        for ch in 0..3 {
            let a = unquantize(e[2 * r][ch], m.ep_bits);
            let b = unquantize(e[2 * r + 1][ch], m.ep_bits);
            let v = (a * (64 - w) + b * w + 32) >> 6;
            out[p][ch] = half_to_f32(((v * 31) >> 6) as u16);
        }
    }
    out
}


// ---------------------------------------------------------------------------
// BCPack layer
// ---------------------------------------------------------------------------

/// `alphabet_bits` (symbol width - 1) of the 16 BC6H lanes.
pub const BC6_ALPHABET_BITS: [u8; 16] = [3, 3, 7, 7, 7, 4, 4, 4, 5, 5, 5, 7, 7, 7, 7, 4];

/// Endpoint state: the stored fields [w, x, y, z][R, G, B], 16-bit MSB-aligned.
pub type EpState = [[u16; 3]; 4];
/// Index state: 16 indices, 4-bit MSB-aligned.
pub type IdxState = [u8; 16];

/// Preset history tables (slot `(i - d) mod 256` for references before block 0).
pub struct Tables {
    pub hist_ep: [Option<EpState>; 256],
    pub hist_idx: [Option<IdxState>; 256],
}

/// Result of a BC6H chunk decode.
pub struct Bc6Output {
    /// BC6H blocks, stream order.
    pub blocks: Vec<u8>,
    /// Blocks whose endpoint or index chain starts at a preset slot not in the table.
    pub unfitted_blocks: usize,
}

#[inline]
fn uz(v: u32) -> i32 { if v & 1 == 0 { (v >> 1) as i32 } else { -(((v + 1) >> 1) as i32) } }
#[inline]
fn il(e: u32, o: u32) -> u32 { let mut v = 0; for k in 0..8 { v |= ((e >> k) & 1) << (2 * k) | ((o >> k) & 1) << (2 * k + 1); } v }
/// Modes with more than 11 base bits store bits 10.. of w in reverse order.
#[inline]
fn rev_hi(v: u32, bits: u32) -> u32 {
    if bits <= 11 { return v; }
    let k = bits - 10;
    let hi = (v >> 10) & ((1 << k) - 1);
    let mut r = 0;
    for j in 0..k { r |= ((hi >> j) & 1) << (k - 1 - j); }
    (v & 1023) | (r << 10)
}

/// Decodes the BC6H lane symbols (`lane[l][2 * i + h]`, h = 0 for E, 1 for O)
/// of `blocks` blocks. Returns the blocks, the per-block states and whether
/// each block's endpoint / index chain reached an unknown preset slot.
pub fn decode_lanes(lane: &[&[u16]], blocks: usize, t: &Tables) -> (Bc6Output, Vec<EpState>, Vec<IdxState>, Vec<(bool, bool)>) {
    let sym = |l: usize, i: usize, h: usize| lane[l][2 * i + h] as u32;
    let mut ep = vec![[[0u16; 3]; 4]; blocks];
    let mut ix = vec![[0u8; 16]; blocks];
    let mut taint = vec![(false, false); blocks];
    let mut out = vec![0u8; blocks * 16];
    let mut unfitted = 0;
    for i in 0..blocks {
        let di = il(sym(1, i, 0), sym(1, i, 1)) as usize;
        let de = ((di as i32 + uz(il(sym(0, i, 0), sym(0, i, 1)))) & 255) as usize;
        let (pe, te) = if de == 0 { ([[0x8000u16; 3]; 4], false) } else if de <= i { (ep[i - de], taint[i - de].0) } else {
            match t.hist_ep[(i as isize - de as isize).rem_euclid(256) as usize] { Some(v) => (v, false), None => ([[0x8000; 3]; 4], true) }
        };
        let (pq, ti) = if di == 0 { ([8u8; 16], false) } else if di <= i { (ix[i - di], taint[i - di].1) } else {
            match t.hist_idx[(i as isize - di as isize).rem_euclid(256) as usize] { Some(v) => (v, false), None => ([8; 16], true) }
        };
        let mode = sym(15, i, 0) as usize;
        let part = (sym(15, i, 1) & 31) as u8;
        let Some(m) = MODES.get(mode) else {
            ep[i] = pe; ix[i] = pq; taint[i] = (true, true); unfitted += 1; continue;
        };
        let b = m.ep_bits;
        let ne = if m.two_regions { 4 } else { 2 };
        let mut f = Fields { mode: mode as u8, partition: if m.two_regions { part } else { 0 }, ..Default::default() };
        let mut st = pe;
        for e in 0..ne {
            for c in 0..3 {
                let w = if e == 0 || !m.transformed { b } else { m.delta_bits[c] };
                let raw = if e < 2 { let l = 2 + 3 * e + c; il(sym(l, i, 0), sym(l, i, 1)) } else { sym(8 + c, i, e - 2) };
                let v = (((pe[e][c] >> (16 - w)) as i32 + uz(raw)).rem_euclid(1 << w)) as u32;
                st[e][c] = (v << (16 - w)) as u16;
                f.ep[e][c] = if e == 0 { rev_hi(v, b) } else { v } as u16;
            }
        }
        if !m.two_regions { st[2] = one_region_y(m, &st[0]); st[3] = st[1]; }
        let ib = if m.two_regions { 3 } else { 4 };
        let mut q = pq;
        for k in 0..4 {
            for h in 0..2 {
                let s = sym(11 + k, i, h);
                let (mut x, mut y) = (0u32, 0u32);
                for j in 0..ib { x |= ((s >> (2 * j)) & 1) << j; y |= ((s >> (2 * j + 1)) & 1) << j; }
                let (left, right) = if k < 3 { (y, x) } else { (x, y) };
                for (p, d) in [(4 * k + 2 * h, left), (4 * k + 2 * h + 1, right)] {
                    let v = (((pq[p] >> (4 - ib)) as i32 + uz(d)).rem_euclid(1 << ib)) as u8;
                    q[p] = v << (4 - ib);
                    f.idx[p] = v;
                }
            }
        }
        ep[i] = st; ix[i] = q; taint[i] = (te, ti);
        if te || ti { unfitted += 1; }
        if let Some(blk) = pack(&f) { out[i * 16..i * 16 + 16].copy_from_slice(&blk); }
    }
    (Bc6Output { blocks: out, unfitted_blocks: unfitted }, ep, ix, taint)
}

/// Region-1 first-endpoint state left by a one-region block. Untransformed: a copy of w.
/// Transformed: w written into a delta-width field, offset by half its range.
fn one_region_y(m: &ModeInfo, w: &[u16; 3]) -> [u16; 3] {
    if !m.transformed { return *w; }
    std::array::from_fn(|c| {
        let v = (w[c] >> (16 - m.ep_bits)) as u32;
        ((v << (16 - m.delta_bits[c])) as u16) ^ 0x8000
    })
}

/// BCPack BC6H chunk bytes + block count -> BC6H blocks, with given tables.
pub fn decode_with(chunk: &[u8], blocks: usize, t: &Tables) -> Option<Bc6Output> {
    let lanes = bcpack::rans::decode_slice(chunk, &BC6_ALPHABET_BITS, 2 * blocks).ok()?;
    let l: Vec<&[u16]> = (0..16).map(|k| lanes.lane(k)).collect();
    Some(decode_lanes(&l, blocks, t).0)
}

/// The fitted preset tables.
pub fn tables() -> Tables { Tables { hist_ep: HIST_EP, hist_idx: HIST_IDX } }

/// BCPack BC6H chunk bytes + block count (16-byte blocks) -> BC6H blocks.
/// `None` if the stream does not decode.
pub fn decode_bc6h(chunk: &[u8], blocks: usize) -> Option<Bc6Output> { decode_with(chunk, blocks, &tables()) }

include!("bc6pack_tables.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_fill_the_block() {
        for (i, m) in MODES.iter().enumerate() {
            let ep: u32 = LAYOUTS[i].iter().map(|o| o.0 as u32).sum();
            let total = m.code_bits + ep + if m.two_regions { 5 + 46 } else { 63 };
            assert_eq!(total, 128, "mode {i}");
            // every endpoint bit covered exactly once
            let n = if m.two_regions { 4 } else { 2 };
            for e in 0..n { for ch in 0..3 {
                let bits = if e == 0 || !m.transformed && !m.two_regions { m.ep_bits } else if m.transformed { m.delta_bits[ch] } else { m.ep_bits };
                let mut seen = 0u32;
                for &(b, c, ee, sh) in LAYOUTS[i] { if c as usize == ch && ee as usize == e { for k in 0..b { let bit = 1u32 << (sh + k); assert_eq!(seen & bit, 0); seen |= bit; } } }
                assert_eq!(seen, (1u32 << bits) - 1, "mode {i} ep {e} ch {ch}");
            } }
        }
    }

    #[test]
    fn pack_unpack_round_trip_and_reference_pixels() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed };
        for _ in 0..2000 {
            let mut blk = [0u8; 16];
            for b in blk.iter_mut() { *b = rnd() as u8; }
            let Some(f) = unpack(&blk) else { continue };
            assert_eq!(pack(&f), Some(blk));
            // The reference decoder maps 2^(bits-1) instead of 2^bits - 1 to
            // the maximum on unsigned formats; skip blocks that hit either.
            let m = &MODES[f.mode as usize];
            let special = |v: u32| m.ep_bits < 15 && (v == 1 << (m.ep_bits - 1) || v == (1 << m.ep_bits) - 1);
            if endpoints(&f).iter().any(|e| e.iter().any(|&v| special(v))) { continue; }
            // Its 16-bit mode also computes the delta mask in u16, which overflows.
            if m.ep_bits == 16 { continue; }
            let mut px = [0u32; 16];
            texture2ddecoder::decode_bc6_block(&blk, &mut px, false);
            let mine = decode_pixels(&f);
            for p in 0..16 {
                let [b, g, r, _] = px[p].to_le_bytes();
                for (ch, v) in [r, g, b].into_iter().enumerate() {
                    let want = (mine[p][ch] * 255.0).clamp(0.0, 255.0) as u8;
                    assert!((want as i32 - v as i32).abs() <= 1, "mode {} pixel {p} ch {ch}: {want} vs {v}", f.mode);
                }
            }
        }
    }

    #[test]
    fn one_region_y_state() {
        // untransformed (10-bit, mode index 10): y state is w
        assert_eq!(one_region_y(&MODES[10], &[0x5600, 0x5640, 0x8000]), [0x5600, 0x5640, 0x8000]);
        // 11-bit w = 0x328 into the 9-bit delta width: low 9 bits 0x128, MSB-aligned, offset by half
        let w = (0x328u16 << 5, 0u16, 0xffe0u16);
        let y = one_region_y(&MODES[11], &[w.0, w.1, w.2]);
        assert_eq!(y, [(0x128 << 7) ^ 0x8000, 0x8000, (0x1ff << 7) ^ 0x8000]);
        // 6-bit read of that state (mode index 1): bits 8..13 of w with the top bit flipped
        assert_eq!(y[0] >> 10, 5);
    }

    #[test]
    fn rev_hi_reverses_only_high_bits() {
        assert_eq!(rev_hi(0x3ff, 10), 0x3ff);
        assert_eq!(rev_hi(1 << 10, 11), 1 << 10);
        assert_eq!(rev_hi(1 << 10, 12), 1 << 11);
        assert_eq!(rev_hi((1 << 11) | 5, 12), (1 << 10) | 5);
        assert_eq!(rev_hi(1 << 10, 16), 1 << 15);
        for v in [0u32, 1234, 0xabcd, 0xffff] { assert_eq!(rev_hi(rev_hi(v, 16), 16), v); }
    }

    #[test]
    fn zero_lanes_decode_to_half_range_blocks() {
        // All-zero symbols: mode 0, every field at the d = 0 predictor (half range).
        let z = vec![0u16; 2 * 4];
        let lanes: Vec<&[u16]> = (0..16).map(|_| &z[..]).collect();
        let t = Tables { hist_ep: [None; 256], hist_idx: [None; 256] };
        let (out, ep, ix, taint) = decode_lanes(&lanes, 4, &t);
        assert_eq!(out.unfitted_blocks, 0);
        assert!(taint.iter().all(|&(a, b)| !a && !b));
        let f = unpack(&out.blocks[..16].try_into().unwrap()).unwrap();
        assert_eq!(f.mode, 0);
        assert_eq!(f.ep[0], [512; 3]);
        assert_eq!(f.ep[1], [16; 3]);
        assert_eq!(ep[3][0], [0x8000; 3]);
        assert_eq!(f.idx[1], 4);
        assert_eq!(ix[3][1], 8);
        let _ = decode_bc6h(&[0u8; 64], 16);
    }

}
