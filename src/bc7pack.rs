// Adapted from holdmysocks/SixNG, commit 7881042c294f3eeea1e8153e7bcafd84f9d787b9.
// Copyright SixNG contributors. SPDX-License-Identifier: GPL-3.0-only
// Source: https://github.com/holdmysocks/SixNG
//! BCPack BC7 (codec 0x1C) chunks -> plain BC7 blocks.
//!
//! The entropy layer is the one from `bcpack-rs` (16 lanes, two symbols per
//! lane per block; E = even symbol, O = odd symbol). Each block carries a
//! state that later blocks predict from:
//!
//! - **References.** `il(e, o)` interleaves two nibbles (E bits on even
//!   positions). The index reference distance is `di = il(E1, O1)` and the
//!   endpoint reference distance is `de = (di + unzigzag(il(E0, O0))) & 255`.
//!   `d = 0` predicts from zero, `d <= i` from block `i - d`, and `d > i` from
//!   slot `256 + i - d` of a fixed 256-slot history (separate tables for
//!   endpoints and indices).
//! - **Header.** Lane 15 (E, O) names the mode, partition, rotation and index
//!   selection (`HEADERS`).
//! - **Endpoints.** The state is 3 subsets x 2 endpoints x RGBA, 8-bit and
//!   MSB-aligned. A mode reads `w` bits per channel (channel 0 includes the
//!   p-bit when the mode has one) and adds the zigzag residual modulo `2^w`.
//!   Lanes 2-5 hold subset 0 RGBA, lanes 6-9 subset 1, lanes 10, 5, 9
//!   subset 2 RGB (E = endpoint 0, O = endpoint 1). RGB-only modes set alpha
//!   to 0, and subsets a mode does not use copy subset 0.
//! - **Indices.** Two sets of 16 indices, 4-bit and MSB-aligned. Lanes 11-14
//!   hold rows 0-3 of the primary set and lanes 6, 7, 8, 10 rows of the
//!   secondary set (modes 4 and 5). Each symbol packs two zigzag deltas with
//!   interleaved bits (E = columns 0-1, O = columns 2-3); rows 0-2 put the
//!   left pixel on the odd bits, row 3 on the even bits.
//!
//! The history tables and part of the header table were fitted against the
//! neighbouring mips of retail textures; entries no retail chunk uses are not
//! known and decode as zeros (counted in `unfitted_blocks`). Blocks come out
//! in stream (Gfx10 tile) order.

/// Standard BC7 field layout per mode: (subsets, partition bits, rotation
/// bits, index-selection bits, colour bits, alpha bits, p-bits per endpoint
/// (1 unique, 2 shared per subset, 0 none), index bits, secondary index bits).
struct ModeInfo { ns: usize, pb: u32, rb: u32, isb: u32, cb: u32, ab: u32, pbit: u8, ib: u32, ib2: u32 }
const MODES: [ModeInfo; 8] = [
    ModeInfo { ns: 3, pb: 4, rb: 0, isb: 0, cb: 4, ab: 0, pbit: 1, ib: 3, ib2: 0 },
    ModeInfo { ns: 2, pb: 6, rb: 0, isb: 0, cb: 6, ab: 0, pbit: 2, ib: 3, ib2: 0 },
    ModeInfo { ns: 3, pb: 6, rb: 0, isb: 0, cb: 5, ab: 0, pbit: 0, ib: 2, ib2: 0 },
    ModeInfo { ns: 2, pb: 6, rb: 0, isb: 0, cb: 7, ab: 0, pbit: 1, ib: 2, ib2: 0 },
    ModeInfo { ns: 1, pb: 0, rb: 2, isb: 1, cb: 5, ab: 6, pbit: 0, ib: 2, ib2: 3 },
    ModeInfo { ns: 1, pb: 0, rb: 2, isb: 0, cb: 7, ab: 8, pbit: 0, ib: 2, ib2: 2 },
    ModeInfo { ns: 1, pb: 0, rb: 0, isb: 0, cb: 7, ab: 7, pbit: 1, ib: 4, ib2: 0 },
    ModeInfo { ns: 2, pb: 6, rb: 0, isb: 0, cb: 5, ab: 5, pbit: 1, ib: 2, ib2: 0 },
];

/// Decoded fields of one BC7 block, in BC7 terms.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Fields {
    pub mode: u8,
    pub partition: u8,
    pub rotation: u8,
    pub isel: u8,
    /// Stored endpoint values [subset][endpoint][R, G, B, A], without p-bits,
    /// in stored (pre-rotation) channel order.
    pub ep: [[[u8; 4]; 2]; 3],
    /// p-bits [subset][endpoint] (shared p-bit modes use endpoint 0).
    pub p: [[u8; 2]; 3],
    /// First stored index set (BC7 order), then the second (modes 4 and 5).
    pub idx: [[u8; 16]; 2],
}

struct BitWriter { v: u128, n: u32 }
impl BitWriter {
    fn put(&mut self, x: u32, bits: u32) { if bits > 0 { self.v |= ((x as u128) & ((1u128 << bits) - 1)) << self.n; self.n += bits; } }
}

/// Anchor pixel of each subset for a partition.
fn anchors(ns: usize, part: usize) -> [usize; 3] {
    match ns { 1 => [0, 16, 16], 2 => [0, ANCHOR2[part] as usize, 16], _ => [0, ANCHOR3[0][part] as usize, ANCHOR3[1][part] as usize] }
}

/// Subset of pixel `p` in partition `part` of an `ns`-subset mode.
fn subset_of(ns: usize, part: usize, p: usize) -> usize {
    match ns { 1 => 0, 2 => (PART2[part] >> (2 * p) & 3) as usize, _ => (PART3[part] >> (2 * p) & 3) as usize }
}

/// Makes every anchor index's top bit zero without changing any pixel, the
/// way BC7 encoders do: for each subset (or index set) whose anchor has the
/// top bit set, swap that subset's endpoints (and p-bits) in the channels
/// the index set drives and invert its indices.
pub fn normalize(f: &mut Fields) {
    let m = &MODES[f.mode as usize];
    let top = |b: u32| 1u8 << (b - 1);
    if m.ib2 > 0 {
        // modes 4/5: one subset, two index sets; idx[0] drives RGB unless isel = 1
        for set in 0..2 {
            let b = if set == 0 { m.ib } else { m.ib2 };
            if f.idx[set][0] & top(b) == 0 { continue; }
            let alpha = (set == 0) == (f.isel == 1);
            let chans: &[usize] = if alpha { &[3] } else { &[0, 1, 2] };
            for &c in chans { let t = f.ep[0][0][c]; f.ep[0][0][c] = f.ep[0][1][c]; f.ep[0][1][c] = t; }
            for p in 0..16 { f.idx[set][p] = ((1u8 << b) - 1) - f.idx[set][p]; }
        }
        return;
    }
    let anc = anchors(m.ns, f.partition as usize);
    for s in 0..m.ns {
        if f.idx[0][anc[s]] & top(m.ib) == 0 { continue; }
        f.ep[s].swap(0, 1);
        if m.pbit == 1 { f.p[s].swap(0, 1); }
        for p in 0..16 { if subset_of(m.ns, f.partition as usize, p) == s { f.idx[0][p] = ((1u8 << m.ib) - 1) - f.idx[0][p]; } }
    }
}

/// Packs fields into a standard 16-byte BC7 block. Returns `None` when an
/// anchor index has its top bit set (the block would not be representable).
pub fn pack(f: &Fields) -> Option<[u8; 16]> {
    let m = &MODES[f.mode as usize];
    let mut w = BitWriter { v: 0, n: 0 };
    w.put(1 << f.mode, f.mode as u32 + 1);
    w.put(f.partition as u32, m.pb);
    w.put(f.rotation as u32, m.rb);
    w.put(f.isel as u32, m.isb);
    for ch in 0..3 { for s in 0..m.ns { for e in 0..2 { w.put(f.ep[s][e][ch] as u32, m.cb); } } }
    if m.ab > 0 { for s in 0..m.ns { for e in 0..2 { w.put(f.ep[s][e][3] as u32, m.ab); } } }
    match m.pbit {
        1 => for s in 0..m.ns { for e in 0..2 { w.put(f.p[s][e] as u32, 1); } },
        2 => for s in 0..m.ns { w.put(f.p[s][0] as u32, 1); },
        _ => {}
    }
    let anc = anchors(m.ns, f.partition as usize);
    for (set, bits) in [(0usize, m.ib), (1, m.ib2)] {
        if bits == 0 { continue; }
        for p in 0..16 {
            let v = f.idx[set][p] as u32;
            let is_anchor = if set == 0 { anc[..m.ns].contains(&p) } else { p == 0 };
            if is_anchor {
                if v >> (bits - 1) != 0 { return None; }
                w.put(v, bits - 1);
            } else {
                w.put(v, bits);
            }
        }
    }
    debug_assert_eq!(w.n, 128);
    Some(w.v.to_le_bytes())
}

include!("bc7pack_tables.rs");

/// `alphabet_bits` (symbol width - 1) of the 16 BC7 lanes, as `bcpack::rans` takes them.
pub const BC7_ALPHABET_BITS: [u8; 16] = [3, 3, 7, 7, 7, 7, 7, 6, 6, 4, 5, 7, 7, 7, 7, 4];

/// Endpoint state: [subset][endpoint][R, G, B, A], 8-bit MSB-aligned.
pub type EpState = [[[u8; 4]; 2]; 3];
/// Index state: two sets of 16 indices, 4-bit MSB-aligned.
pub type IdxState = [[u8; 16]; 2];

/// Lane-15 header and history tables.
pub struct Tables {
    /// `(mode, rotation, isel, partition)` per lane-15 (E, O) pair, `None` if unknown.
    pub headers: [[Option<(u8, u8, u8, u8)>; 32]; 32],
    pub hist_ep: [Option<EpState>; 256],
    pub hist_idx: [Option<IdxState>; 256],
}

/// Built-in tables.
pub fn tables() -> Tables {
    let mut headers = [[None; 32]; 32];
    for &(a, b, mode, rot, isel, part) in HEADERS.iter() { headers[a as usize][b as usize] = Some((mode, rot, isel, part)); }
    Tables { headers, hist_ep: HIST_EP, hist_idx: HIST_IDX }
}

/// Result of a BC7 chunk decode.
pub struct Bc7Output {
    /// BC7 blocks, 16 bytes each, stream order.
    pub blocks: Vec<u8>,
    /// Blocks that depend on a history slot or header not in the tables.
    pub unfitted_blocks: usize,
    /// Blocks whose decoded anchor indices have the top bit set (not valid
    /// BC7; written with the bit cleared).
    pub invalid_blocks: usize,
    /// `invalid_blocks` split by BC7 mode.
    pub invalid_by_mode: [usize; 8],
    /// Blocks whose anchors needed an endpoint swap (pixels unchanged).
    pub normalized_blocks: usize,
}

#[inline]
fn uz(v: u16) -> i32 { let v = v as i32; if v & 1 == 0 { v >> 1 } else { -((v + 1) >> 1) } }
#[inline]
fn il(e: u16, o: u16) -> usize {
    let mut v = 0usize;
    for k in 0..4 { v |= ((e as usize >> k) & 1) << (2 * k) | ((o as usize >> k) & 1) << (2 * k + 1); }
    v
}
fn rot_channel(rot: u8) -> usize { [3, 0, 1, 2][rot as usize & 3] }
/// Stored bits per output channel (channel 0 includes the p-bit when the mode has one).
fn widths(mode: u8, rot: u8) -> [u32; 4] {
    match mode {
        0 => [5, 4, 4, 0], 1 => [7, 6, 6, 0], 2 => [5, 5, 5, 0], 3 => [8, 7, 7, 0],
        6 => [8, 7, 7, 7], 7 => [6, 5, 5, 5],
        m => { let mut w = if m == 5 { [7; 4] } else { [5; 4] }; w[rot_channel(rot)] = if m == 5 { 8 } else { 6 }; w }
    }
}
fn index_bits(mode: u8) -> (u32, u32) { match mode { 6 => (4, 0), 5 => (2, 2), 4 => (3, 2), 0 | 1 => (3, 0), _ => (2, 0) } }
fn ep_lane(nsub: usize, s: usize, ch: usize) -> usize {
    match (nsub, s) { (3, 2) => [10, 5, 9][ch], (_, 0) => 2 + ch, _ => 6 + ch }
}
/// BCPack BC7 chunk bytes + block count -> BC7 blocks, with the built-in tables.
pub fn decode(chunk: &[u8], blocks: usize) -> Option<Bc7Output> {
    decode_with(chunk, blocks, &tables())
}

/// As [`decode`], with caller-supplied tables.
pub fn decode_with(chunk: &[u8], blocks: usize, t: &Tables) -> Option<Bc7Output> {
    decode_states(chunk, blocks, t).map(|r| r.0)
}

/// As [`decode_with`], also returning each block's endpoint and index state.
pub fn decode_states(chunk: &[u8], blocks: usize, t: &Tables) -> Option<(Bc7Output, Vec<EpState>, Vec<IdxState>)> {
    let lanes = bcpack::rans::decode_slice(chunk, &BC7_ALPHABET_BITS, 2 * blocks).ok()?;
    let lane: Vec<&[u16]> = (0..16).map(|k| lanes.lane(k)).collect();
    let sym = |l: usize, i: usize, h: usize| lane[l][2 * i + h];
    let mut ep: Vec<EpState> = vec![[[[0; 4]; 2]; 3]; blocks];
    let mut ix: Vec<IdxState> = vec![[[0; 16]; 2]; blocks];
    let mut tainted_ep = vec![false; blocks];
    let mut tainted_ix = vec![false; blocks];
    let mut out = vec![0u8; blocks * 16];
    let (mut unfitted, mut invalid) = (0, 0);
    let mut invalid_by_mode = [0usize; 8];
    let mut normalized_count = 0usize;
    for i in 0..blocks {
        let di = il(sym(1, i, 0), sym(1, i, 1));
        let de = ((di as i32 + uz(il(sym(0, i, 0), sym(0, i, 1)) as u16)) & 255) as usize;
        let hdr = t.headers[sym(15, i, 0) as usize & 31][sym(15, i, 1) as usize & 31];
        let (mut taint_ep, mut taint_ix) = (false, false);
        let pe: EpState = if de == 0 { [[[0; 4]; 2]; 3] } else if de <= i { taint_ep |= tainted_ep[i - de]; ep[i - de] }
            else { match t.hist_ep[256 + i - de] { Some(v) => v, None => { taint_ep = true; [[[0; 4]; 2]; 3] } } };
        let pq: IdxState = if di == 0 { [[0; 16]; 2] } else if di <= i { taint_ix |= tainted_ix[i - di]; ix[i - di] }
            else { match t.hist_idx[256 + i - di] { Some(v) => v, None => { taint_ix = true; [[0; 16]; 2] } } };
        let Some((mode, rot, isel, part)) = hdr else {
            ep[i] = pe; ix[i] = pq; tainted_ep[i] = true; tainted_ix[i] = true; unfitted += 1; continue;
        };
        // endpoints
        let ns = MODES[mode as usize].ns;
        let nc = if mode <= 3 { 3 } else { 4 };
        let w = widths(mode, rot);
        let mut st = pe;
        for s in 0..ns { for ch in 0..nc {
            let l = ep_lane(ns, s, ch);
            for e in 0..2 {
                let m = 1i32 << w[ch];
                let v = (pe[s][e][ch] as i32) >> (8 - w[ch]);
                st[s][e][ch] = (((v + uz(sym(l, i, e))).rem_euclid(m)) << (8 - w[ch])) as u8;
            }
        } }
        if nc == 3 { for s in 0..3 { for e in 0..2 { st[s][e][3] = 0; } } }
        for s in ns..3 { st[s] = st[0]; }
        ep[i] = st;
        // indices
        let (b1, b2) = index_bits(mode);
        let mut q = pq;
        for (set, lanes_, b) in [(0usize, [11usize, 12, 13, 14], b1), (1, [6, 7, 8, 10], b2)] {
            if b == 0 { continue; }
            for (k, &l) in lanes_.iter().enumerate() {
                for h in 0..2 {
                    let s16 = sym(l, i, h) as u32;
                    let (mut a, mut c) = (0u16, 0u16);
                    for j in 0..b { a |= (((s16 >> (2 * j)) & 1) as u16) << j; c |= (((s16 >> (2 * j + 1)) & 1) as u16) << j; }
                    let (left, right) = if k < 3 { (c, a) } else { (a, c) };
                    for (px, d) in [(4 * k + 2 * h, left), (4 * k + 2 * h + 1, right)] {
                        let m = 1i32 << b;
                        let v = (pq[set][px] as i32) >> (4 - b);
                        q[set][px] = (((v + uz(d)).rem_euclid(m)) << (4 - b)) as u8;
                    }
                }
            }
        }
        ix[i] = q;
        tainted_ep[i] = taint_ep;
        tainted_ix[i] = taint_ix;
        unfitted += (taint_ep || taint_ix) as usize;
        // pack
        let mi = &MODES[mode as usize];
        let mut f = Fields { mode, partition: part, rotation: rot, isel, ..Default::default() };
        let rc = rot_channel(rot);
        for s in 0..ns { for e in 0..2 {
            let val = |ch: usize| (st[s][e][ch] >> (8 - w[ch])) as u8;
            let mut v = [0u8; 4];
            for ch in 0..nc { v[ch] = val(ch); }
            if mi.pbit != 0 { f.p[s][e] = v[0] & 1; v[0] >>= 1; }
            if (mode == 4 || mode == 5) && rc != 3 { v.swap(rc, 3); }
            f.ep[s][e] = v;
        } }
        let ival = |set: usize, b: u32, p: usize| q[set][p] >> (4 - b);
        if mode == 4 {
            for p in 0..16 { f.idx[0][p] = ival(1, 2, p); f.idx[1][p] = ival(0, 3, p); }
        } else {
            for p in 0..16 { f.idx[0][p] = ival(0, b1, p); if b2 > 0 { f.idx[1][p] = ival(1, b2, p); } }
        }
        let normalized = { let before = f.idx; normalize(&mut f); before != f.idx };
        if normalized { normalized_count += 1; }
        let block = match pack(&f) {
            Some(b) => b,
            None => {
                invalid += 1;
                invalid_by_mode[mode as usize] += 1;
                let anc = anchors(ns, part as usize);
                for &p in &anc[..ns] { f.idx[0][p] &= (1 << (mi.ib - 1)) - 1; }
                if mi.ib2 > 0 { f.idx[1][0] &= (1 << (mi.ib2 - 1)) - 1; }
                pack(&f).unwrap_or([0; 16])
            }
        };
        out[i * 16..i * 16 + 16].copy_from_slice(&block);
    }
    Some((Bc7Output { blocks: out, unfitted_blocks: unfitted, invalid_blocks: invalid, invalid_by_mode, normalized_blocks: normalized_count }, ep, ix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_block(b: &[u8; 16]) -> [u32; 16] {
        let mut out = [0u32; 16];
        texture2ddecoder::decode_bc7_block(b, &mut out);
        out
    }

    #[test]
    fn packs_every_mode_like_the_reference_decoder() {
        // Endpoints and indices that touch every field; the decoded colours of
        // pixel 5 (not an anchor in partition 0) must follow the BC7 interpolation.
        for mode in 0..8u8 {
            let m = &MODES[mode as usize];
            let mut f = Fields { mode, ..Default::default() };
            for s in 0..m.ns { for e in 0..2 { for c in 0..4 {
                let bits = if c == 3 { m.ab } else { m.cb };
                if bits > 0 { f.ep[s][e][c] = if e == 0 { 0 } else { ((1u32 << bits) - 1) as u8 }; }
            } } }
            for s in 0..3 { f.p[s] = [0, 1]; }
            if m.pbit == 2 { for s in 0..3 { f.p[s] = [1, 1]; } }
            f.idx[0][5] = ((1u32 << m.ib) - 1) as u8;
            if m.ib2 > 0 { f.idx[1][5] = ((1u32 << m.ib2) - 1) as u8; }
            let b = pack(&f).expect("packable");
            let px = decode_block(&b);
            let [_, g, _, a] = px[5].to_le_bytes();
            // index max -> endpoint 1, which is all ones -> 255
            assert_eq!(g, 255, "mode {mode} green");
            if m.ab > 0 && m.ib2 == 0 { assert_eq!(a, 255, "mode {mode} alpha"); }
            // pixel 0 (anchor, index 0) -> endpoint 0 -> 0 (or the p-bit only)
            let [_, g0, _, _] = px[0].to_le_bytes();
            assert!(g0 <= 8, "mode {mode} pixel 0 green {g0}");
        }
    }

    #[test]
    fn header_table_covers_every_mode_once_per_config() {
        let t = tables();
        let mut seen = std::collections::HashSet::new();
        let mut per_mode = [0usize; 8];
        for a in 0..32 { for b in 0..32 { if let Some(h) = t.headers[a][b] {
            let (mode, rot, isel, part) = h;
            assert!(mode < 8 && rot < 4 && isel < 2);
            assert!((part as usize) < if mode == 0 { 16 } else { 64 });
            assert!(seen.insert(h), "duplicate header {h:?}");
            per_mode[mode as usize] += 1;
        } } }
        assert_eq!(per_mode, [16, 64, 64, 64, 8, 4, 1, 64]);
    }

    #[test]
    fn decodes_a_rans_stream_of_zero_blocks() {
        // Garbage input must not panic; it either fails cleanly or yields 16 bytes per block.
        if let Some(o) = decode(&[0u8; 64], 4) { assert_eq!(o.blocks.len(), 64); }
    }

    #[test]
    fn normalize_keeps_pixels() {
        // A random-ish block per mode with every anchor's top bit set decodes to the
        // same pixels after normalising.
        for mode in 0..8u8 {
            let m = &MODES[mode as usize];
            for isel in 0..=(m.isb as u8) {
                let mut f = Fields { mode, isel, partition: if m.pb > 0 { 13 } else { 0 }, ..Default::default() };
                for s in 0..3 { for e in 0..2 { for c in 0..4 {
                    let bits = if c == 3 { m.ab } else { m.cb };
                    if bits > 0 { f.ep[s][e][c] = ((s * 7 + e * 13 + c * 5 + 3) as u32 % (1u32 << bits)) as u8; }
                } } f.p[s] = [1, 0]; }
                for p in 0..16 { f.idx[0][p] = ((p * 5 + 3) as u32 % (1u32 << m.ib)) as u8; if m.ib2 > 0 { f.idx[1][p] = ((p * 3 + 1) as u32 % (1u32 << m.ib2)) as u8; } }
                let anc = anchors(m.ns, f.partition as usize);
                for s in 0..m.ns { f.idx[0][anc[s]] |= 1 << (m.ib - 1); }
                if m.ib2 > 0 { f.idx[1][0] |= 1 << (m.ib2 - 1); }
                // reference: decode with the raw (unpackable) fields by packing with the top bits forced through
                let mut g = f; normalize(&mut g);
                let b = pack(&g).expect("normalized block packs");
                let px = decode_block(&b);
                // pixel-level check against an independent BC7 evaluation of f
                for p in 0..16 {
                    let want = eval_pixel(&f, p);
                    let [bb, gg, rr, aa] = px[p].to_le_bytes();
                    assert_eq!([rr, gg, bb, aa], want, "mode {mode} isel {isel} pixel {p}");
                }
            }
        }
    }

    /// Straight BC7 pixel evaluation of unpacked fields (no anchor rules).
    fn eval_pixel(f: &Fields, p: usize) -> [u8; 4] {
        const W2: [u32; 4] = [0, 21, 43, 64];
        const W3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
        const W4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
        let wt = |b: u32, i: u8| match b { 2 => W2[i as usize], 3 => W3[i as usize], _ => W4[i as usize] };
        let m = &MODES[f.mode as usize];
        let s = subset_of(m.ns, f.partition as usize, p);
        let mut ep = [[0u32; 4]; 2];
        for e in 0..2 { for c in 0..4 {
            let (mut v, mut bits) = (f.ep[s][e][c] as u32, if c == 3 { m.ab } else { m.cb });
            if bits == 0 { ep[e][c] = 255; continue; }
            if m.pbit != 0 { let pb = if m.pbit == 2 { f.p[s][0] } else { f.p[s][e] }; v = (v << 1) | pb as u32; bits += 1; }
            v <<= 8 - bits; ep[e][c] = v | (v >> bits);
        } }
        let (ci, cb, ai, ab) = if m.ib2 > 0 {
            if f.isel == 1 { (f.idx[1][p], m.ib2, f.idx[0][p], m.ib) } else { (f.idx[0][p], m.ib, f.idx[1][p], m.ib2) }
        } else { (f.idx[0][p], m.ib, f.idx[0][p], m.ib) };
        let mut out = [0u8; 4];
        for c in 0..4 {
            let w = if c == 3 { wt(ab, ai) } else { wt(cb, ci) };
            out[c] = (((64 - w) * ep[0][c] + w * ep[1][c] + 32) >> 6) as u8;
        }
        if m.ab == 0 { out[3] = 255; }
        let r = f.rotation as usize;
        if r > 0 { out.swap(r - 1, 3); }
        out
    }

    #[test]
    fn anchor_with_top_bit_is_rejected() {
        let mut f = Fields { mode: 6, ..Default::default() };
        f.idx[0][0] = 8;
        assert!(pack(&f).is_none());
    }
}
