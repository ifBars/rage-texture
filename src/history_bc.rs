//! Classic BC1/BC3/BC4/BC5 BCPack initial-history reconstruction.
//!
//! Ported from Leonida's clean-room decoder, `tools/bcpack.py` and
//! `tools/bcpack_tables.py`, revision
//! `aa2988cc555bad7ecd508b48a702adbb32fe7c95`
//! (<https://github.com/ItsNotRexed/Leonida>), GPL-3.0. Those files state that
//! the codec constants were recovered clean-room from public specifications
//! (US patent 11,234,023 and the BCn block formats); no game assets are
//! included. The bundled `history_bc_tables.json` is generated from
//! `bcpack_tables.py` by the exporter recorded in `HISTORY_CLASSIC_REVIEW.md`.
//!
//! The entropy layer is decoded elsewhere; this module only rebuilds BCn blocks
//! from per-lane symbols. It mirrors Leonida's strict semantics: confirmed and
//! provisional built-in history stay separate, and an unknown history entry is
//! always rejected. Provisional entries are used only when the caller opts in,
//! and every use is labelled in `Rebuilt::provisional`.
use crate::history::Rebuilt;
use serde_json::Value;
use std::collections::BTreeSet;
use std::io;
use std::sync::OnceLock;

/// Per-lane symbol widths from Leonida's `LANE_WIDTHS` table.
const BC1_LANE_WIDTHS: [u8; 12] = [8, 8, 5, 6, 5, 5, 6, 5, 8, 8, 8, 8];
const BC3_LANE_WIDTHS: [u8; 12] = [4, 4, 8, 6, 6, 6, 6, 5, 6, 5, 8, 8];
const BC4_LANE_WIDTHS: [u8; 12] = [8, 8, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6];
const BC5_LANE_WIDTHS: [u8; 12] = [4, 4, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6];

/// Endpoint moduli from `bcpack_tables_bptc._BC_HISTORY_SPEC`.
const BC1_MODS: [u16; 6] = [32, 64, 32, 32, 64, 32];
const BC3_MODS: [u16; 8] = [256, 256, 32, 64, 32, 32, 64, 32];
const BC4_MODS: [u16; 2] = [256, 256];
const BC5_MODS: [u16; 4] = [256, 256, 256, 256];

/// Zero-delta bases from the Python rebuilders (`_BC3_NONE`, `(128, 128)`, ...).
const ZERO_BC1: [u16; 6] = [0; 6];
const NONE_BC3: [u16; 8] = [128, 128, 0, 0, 0, 0, 0, 0];
const NONE_BC4: [u16; 2] = [128, 128];
const NONE_BC5: [u16; 4] = [128, 128, 128, 128];

/// 256 possible built-in history slots; `None` means the slot has no entry.
type Entries = Vec<Option<Vec<u16>>>;

/// Validated, immutable history snapshot for the four classic codecs.
///
/// Construction parses and validates the bundled constants and the optional
/// external document exactly once. Decoding then allocates only per-block state
/// and the output buffer.
#[derive(Clone)]
pub(crate) struct Tables {
    bc1: CodecTables,
    bc3: CodecTables,
    bc4: CodecTables,
    bc5: CodecTables,
}

#[derive(Clone)]
struct CodecTables {
    codec: u8,
    /// Lower-case section name used in external documents (`bc1`, ...).
    key: &'static str,
    /// Upper-case name used in diagnostics and provenance labels.
    name: &'static str,
    mods: &'static [u16],
    confirmed: Entries,
    provisional: Entries,
    books: &'static Codebooks,
}

enum Codebooks {
    Bc1 {
        quadrant: Vec<u8>,
    },
    Bc3 {
        alpha_pair: Vec<(u8, u8)>,
        colour_quadrant: Vec<u8>,
    },
    Bc4 {
        pair: Vec<(u8, u8)>,
    },
    Bc5 {
        pair: Vec<(u8, u8)>,
    },
}

struct Builtins {
    bc1: BuiltinCodec,
    bc3: BuiltinCodec,
    bc4: BuiltinCodec,
    bc5: BuiltinCodec,
}

struct BuiltinCodec {
    books: Codebooks,
    confirmed: Entries,
    provisional: Entries,
}

#[derive(Clone, Copy)]
enum Kind {
    Bc1,
    Bc3,
    Bc4,
    Bc5,
}

impl Tables {
    /// Parse the bundled constants and apply an optional external history
    /// document. `history` is `None` or a full JSON object whose sections are
    /// `bc1`, `bc3`, `bc4`, `bc5` (a single-codec section carrying a matching
    /// `"codec"` marker is also accepted). No file is opened here.
    pub(crate) fn new(history: Option<&Value>) -> io::Result<Self> {
        let builtins = builtins()?;
        let mut tables = Self {
            bc1: CodecTables::from_builtin(1, "bc1", "BC1", &BC1_MODS, &builtins.bc1),
            bc3: CodecTables::from_builtin(3, "bc3", "BC3", &BC3_MODS, &builtins.bc3),
            bc4: CodecTables::from_builtin(4, "bc4", "BC4", &BC4_MODS, &builtins.bc4),
            bc5: CodecTables::from_builtin(5, "bc5", "BC5", &BC5_MODS, &builtins.bc5),
        };
        let Some(document) = history else {
            return Ok(tables);
        };
        let object = document
            .as_object()
            .ok_or_else(|| input("history document must be a JSON object"))?;
        for (key, codec) in [("bc1", 1u8), ("bc3", 3), ("bc4", 4), ("bc5", 5)] {
            let section = if let Some(section) = object.get(key) {
                Some(section)
            } else if object.get("codec").and_then(Value::as_str) == Some(key) {
                Some(document)
            } else {
                None
            };
            if let Some(section) = section {
                tables.codec_mut(codec)?.apply(section)?;
            }
        }
        Ok(tables)
    }

    /// Rebuild `nblocks` blocks of `codec` from its per-lane symbol arrays.
    ///
    /// Lane count, lane length, symbol width and the reserved lane are
    /// validated before any output is produced. Unknown or unconfirmed history
    /// entries fail with [`io::ErrorKind::Unsupported`]; malformed lane data
    /// fails with [`io::ErrorKind::InvalidData`].
    pub(crate) fn decode(
        &self,
        codec: u8,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        self.codec(codec)?.decode(lanes, nblocks, allow_provisional)
    }

    fn codec(&self, codec: u8) -> io::Result<&CodecTables> {
        match codec {
            1 => Ok(&self.bc1),
            3 => Ok(&self.bc3),
            4 => Ok(&self.bc4),
            5 => Ok(&self.bc5),
            _ => Err(unsupported(format!(
                "codec {codec} is not a classic BC rebuild codec"
            ))),
        }
    }

    fn codec_mut(&mut self, codec: u8) -> io::Result<&mut CodecTables> {
        match codec {
            1 => Ok(&mut self.bc1),
            3 => Ok(&mut self.bc3),
            4 => Ok(&mut self.bc4),
            5 => Ok(&mut self.bc5),
            _ => Err(unsupported(format!(
                "codec {codec} is not a classic BC rebuild codec"
            ))),
        }
    }
}

impl CodecTables {
    fn from_builtin(
        codec: u8,
        key: &'static str,
        name: &'static str,
        mods: &'static [u16],
        builtin: &'static BuiltinCodec,
    ) -> Self {
        Self {
            codec,
            key,
            name,
            mods,
            confirmed: builtin.confirmed.clone(),
            provisional: builtin.provisional.clone(),
            books: &builtin.books,
        }
    }

    /// Merge or replace this codec's tables with one external section, using
    /// `bcpack_tables_bptc.load_bc_history`'s overlay rules.
    fn apply(&mut self, section: &Value) -> io::Result<()> {
        let object = section.as_object().ok_or_else(|| {
            input(format!(
                "{} history section must be a JSON object",
                self.name
            ))
        })?;
        if let Some(marker) = object.get("codec") {
            let marker = marker
                .as_str()
                .ok_or_else(|| input(format!("{} history codec must be a string", self.name)))?;
            if marker != self.key {
                return Err(input(format!(
                    "expected codec '{}', got '{marker}'",
                    self.key
                )));
            }
        }
        let ext_confirmed = parse_map(object.get("confirmed"), self.mods, self.key, "confirmed")?;
        let ext_provisional = parse_map(
            object.get("provisional"),
            self.mods,
            self.key,
            "provisional",
        )?;
        let replace = match object.get("replace") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| input(format!("{} replace must be a boolean", self.name)))?,
            None => false,
        };
        if replace {
            self.confirmed = ext_confirmed;
            self.provisional = ext_provisional;
        } else {
            for (slot, entry) in self.confirmed.iter_mut().zip(ext_confirmed) {
                if entry.is_some() {
                    *slot = entry;
                }
            }
            for (slot, entry) in self.provisional.iter_mut().zip(ext_provisional) {
                if entry.is_some() {
                    *slot = entry;
                }
            }
        }
        // A confirmed entry always wins over a provisional one, whichever
        // source each came from.
        for index in 0..self.confirmed.len() {
            if self.confirmed[index].is_some() {
                self.provisional[index] = None;
            }
        }
        Ok(())
    }

    fn decode(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        let (widths, per_block, block_bytes) = match self.codec {
            1 => (&BC1_LANE_WIDTHS, 1, 8),
            3 => (&BC3_LANE_WIDTHS, 2, 16),
            4 => (&BC4_LANE_WIDTHS, 1, 8),
            5 => (&BC5_LANE_WIDTHS, 2, 16),
            _ => unreachable!("CodecTables is only constructed for 1, 3, 4 and 5"),
        };
        let expected = nblocks
            .checked_mul(per_block)
            .ok_or_else(|| invalid(format!("{} symbol count overflow", self.name)))?;
        if lanes.len() != widths.len() {
            return Err(invalid(format!(
                "{} expects {} lanes, got {}",
                self.name,
                widths.len(),
                lanes.len()
            )));
        }
        for (index, (lane, &width)) in lanes.iter().zip(widths.iter()).enumerate() {
            if lane.len() != expected {
                return Err(invalid(format!(
                    "{} lane {index} has {} symbols, expected {expected}",
                    self.name,
                    lane.len()
                )));
            }
            let limit = 1u32 << width;
            if let Some(&value) = lane.iter().find(|&&value| u32::from(value) >= limit) {
                return Err(invalid(format!(
                    "{} lane {index} symbol {value} does not fit in {width} bits",
                    self.name
                )));
            }
        }
        if lanes[1].iter().any(|&value| value != 0) {
            return Err(invalid(format!(
                "{} lane 1 is non-zero (unexpected)",
                self.name
            )));
        }
        let out_len = nblocks
            .checked_mul(block_bytes)
            .ok_or_else(|| invalid(format!("{} output size overflow", self.name)))?;
        let mut notes = BTreeSet::new();
        let bytes = match self.codec {
            1 => self.rebuild_bc1(lanes, nblocks, out_len, allow_provisional, &mut notes)?,
            3 => self.rebuild_bc3(lanes, nblocks, out_len, allow_provisional, &mut notes)?,
            4 => self.rebuild_bc4(lanes, nblocks, out_len, allow_provisional, &mut notes)?,
            5 => self.rebuild_bc5(lanes, nblocks, out_len, allow_provisional, &mut notes)?,
            _ => unreachable!("CodecTables is only constructed for 1, 3, 4 and 5"),
        };
        Ok(Rebuilt {
            bytes,
            provisional: notes.into_iter().collect(),
        })
    }

    fn zero_base(&self) -> &'static [u16] {
        match self.codec {
            1 => &ZERO_BC1,
            3 => &NONE_BC3,
            4 => &NONE_BC4,
            5 => &NONE_BC5,
            _ => &[],
        }
    }

    fn rebuild_bc1(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        out_len: usize,
        allow_provisional: bool,
        notes: &mut BTreeSet<String>,
    ) -> io::Result<Vec<u8>> {
        let Codebooks::Bc1 { quadrant } = self.books else {
            unreachable!("BC1 tables use the BC1 codebook")
        };
        let l0 = lanes[0];
        let columns = &lanes[2..8];
        let q = &lanes[8..12];
        let mut out = vec![0u8; out_len];
        let mut history = vec![[0u16; 8]; nblocks];
        for i in 0..nblocks {
            let d = (unzig(l0[i]) & 0xff) as usize;
            let mut base = [0u16; 8];
            base[..6].copy_from_slice(lookup_base(self, &history, i, d, allow_provisional, notes)?);
            let mut e = [0u16; 6];
            for (k, slot) in e.iter_mut().enumerate() {
                *slot = add_mod(base[k], unzig(columns[k][i]), self.mods[k]);
            }
            history[i][..6].copy_from_slice(&e);
            let c0 = (e[0] << 11) | (e[1] << 5) | e[2];
            let c1 = (e[3] << 11) | (e[4] << 5) | e[5];
            write_quad(
                &mut out,
                i * 8,
                c0,
                c1,
                [
                    quadrant[q[0][i] as usize],
                    quadrant[q[1][i] as usize],
                    quadrant[q[2][i] as usize],
                    quadrant[q[3][i] as usize],
                ],
            );
        }
        Ok(out)
    }

    fn rebuild_bc3(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        out_len: usize,
        allow_provisional: bool,
        notes: &mut BTreeSet<String>,
    ) -> io::Result<Vec<u8>> {
        let Codebooks::Bc3 {
            alpha_pair,
            colour_quadrant,
        } = self.books
        else {
            unreachable!("BC3 tables use the BC3 codebook")
        };
        let l0 = lanes[0];
        let alpha = lanes[2];
        let rows = &lanes[3..7];
        let (lr, lg, lb) = (lanes[7], lanes[8], lanes[9]);
        let (top, bottom) = (lanes[10], lanes[11]);
        let mut out = vec![0u8; out_len];
        let mut history = vec![[0u16; 8]; nblocks];
        for i in 0..nblocks {
            let (a, b) = (2 * i, 2 * i + 1);
            let d = (unzig(inter8(l0[a], l0[b])) & 0xff) as usize;
            let mut base = [0u16; 8];
            base.copy_from_slice(lookup_base(self, &history, i, d, allow_provisional, notes)?);
            let deltas = [alpha[a], alpha[b], lr[a], lg[a], lb[a], lr[b], lg[b], lb[b]];
            let mut e = [0u16; 8];
            for (k, slot) in e.iter_mut().enumerate() {
                *slot = add_mod(base[k], unzig(deltas[k]), self.mods[k]);
            }
            history[i] = e;
            let mut bits = 0u64;
            for (r, row) in rows.iter().enumerate() {
                let left = alpha_pair[row[a] as usize];
                let right = alpha_pair[row[b] as usize];
                bits |= index_bits(left, right) << (12 * r);
            }
            let at = i * 16;
            out[at] = e[0] as u8;
            out[at + 1] = e[1] as u8;
            out[at + 2..at + 8].copy_from_slice(&bits.to_le_bytes()[..6]);
            let c0 = (e[2] << 11) | (e[3] << 5) | e[4];
            let c1 = (e[5] << 11) | (e[6] << 5) | e[7];
            write_quad(
                &mut out,
                at + 8,
                c0,
                c1,
                [
                    colour_quadrant[top[a] as usize],
                    colour_quadrant[top[b] as usize],
                    colour_quadrant[bottom[a] as usize],
                    colour_quadrant[bottom[b] as usize],
                ],
            );
        }
        Ok(out)
    }

    fn rebuild_bc4(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        out_len: usize,
        allow_provisional: bool,
        notes: &mut BTreeSet<String>,
    ) -> io::Result<Vec<u8>> {
        let Codebooks::Bc4 { pair } = self.books else {
            unreachable!("BC4 tables use the BC4 codebook")
        };
        let (l0, l2, l3) = (lanes[0], lanes[2], lanes[3]);
        let rows = &lanes[4..12];
        let mut out = vec![0u8; out_len];
        let mut history = vec![[0u16; 8]; nblocks];
        for i in 0..nblocks {
            let d = (unzig(l0[i]) & 0xff) as usize;
            let mut base = [0u16; 8];
            base[..2].copy_from_slice(lookup_base(self, &history, i, d, allow_provisional, notes)?);
            let a0 = add_mod(base[0], unzig(l2[i]), self.mods[0]);
            let a1 = add_mod(base[1], unzig(l3[i]), self.mods[1]);
            history[i][0] = a0;
            history[i][1] = a1;
            let mut bits = 0u64;
            for (r, pair_rows) in rows.chunks_exact(2).enumerate() {
                let left = pair[pair_rows[0][i] as usize];
                let right = pair[pair_rows[1][i] as usize];
                bits |= index_bits(left, right) << (12 * r);
            }
            let at = i * 8;
            out[at] = a0 as u8;
            out[at + 1] = a1 as u8;
            out[at + 2..at + 8].copy_from_slice(&bits.to_le_bytes()[..6]);
        }
        Ok(out)
    }

    fn rebuild_bc5(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        out_len: usize,
        allow_provisional: bool,
        notes: &mut BTreeSet<String>,
    ) -> io::Result<Vec<u8>> {
        let Codebooks::Bc5 { pair } = self.books else {
            unreachable!("BC5 tables use the BC5 codebook")
        };
        let l0 = lanes[0];
        let (red, green) = (lanes[2], lanes[3]);
        let rows = &lanes[4..12];
        let mut out = vec![0u8; out_len];
        let mut history = vec![[0u16; 8]; nblocks];
        for i in 0..nblocks {
            let (a, b) = (2 * i, 2 * i + 1);
            let d = (unzig(inter8(l0[a], l0[b])) & 0xff) as usize;
            let mut base = [0u16; 8];
            base[..4].copy_from_slice(lookup_base(self, &history, i, d, allow_provisional, notes)?);
            let e = [
                add_mod(base[0], unzig(red[a]), self.mods[0]),
                add_mod(base[1], unzig(red[b]), self.mods[1]),
                add_mod(base[2], unzig(green[a]), self.mods[2]),
                add_mod(base[3], unzig(green[b]), self.mods[3]),
            ];
            history[i][..4].copy_from_slice(&e);
            let at = i * 16;
            for (channel, channel_rows) in rows.chunks_exact(4).enumerate() {
                let mut bits = 0u64;
                for (r, row) in channel_rows.iter().enumerate() {
                    let left = pair[row[a] as usize];
                    let right = pair[row[b] as usize];
                    bits |= index_bits(left, right) << (12 * r);
                }
                let offset = at + 8 * channel;
                out[offset] = e[2 * channel] as u8;
                out[offset + 1] = e[2 * channel + 1] as u8;
                out[offset + 2..offset + 8].copy_from_slice(&bits.to_le_bytes()[..6]);
            }
        }
        Ok(out)
    }
}

/// Resolve one block's base endpoint tuple: the zero base, an earlier block in
/// this chunk, or a built-in/external history entry.
fn lookup_base<'a>(
    tables: &'a CodecTables,
    history: &'a [[u16; 8]],
    i: usize,
    d: usize,
    allow_provisional: bool,
    notes: &mut BTreeSet<String>,
) -> io::Result<&'a [u16]> {
    if d == 0 {
        return Ok(tables.zero_base());
    }
    if i >= d {
        return Ok(&history[i - d][..tables.mods.len()]);
    }
    let entry = 256 + i - d;
    if let Some(value) = tables.confirmed.get(entry).and_then(Option::as_deref) {
        return Ok(value);
    }
    if let Some(value) = tables.provisional.get(entry).and_then(Option::as_deref) {
        if !allow_provisional {
            return Err(unsupported(format!(
                "{} block {i} references unconfirmed built-in history entry {entry}",
                tables.name
            )));
        }
        notes.insert(format!("{} history {entry}", tables.name));
        return Ok(value);
    }
    Err(unsupported(format!(
        "{} block {i} references unknown built-in history entry {entry}",
        tables.name
    )))
}

/// Leonida's `_unzig`: zig-zag decode a lane delta into a signed value.
fn unzig(value: u16) -> i32 {
    if value & 1 == 0 {
        i32::from(value >> 1)
    } else {
        -((i32::from(value) + 1) >> 1)
    }
}

/// Add a signed delta to an endpoint component modulo its bit width.
fn add_mod(base: u16, delta: i32, modulus: u16) -> u16 {
    let modulus = i32::from(modulus);
    (i32::from(base) + delta).rem_euclid(modulus) as u16
}

/// Leonida's `_inter8`: interleave two 4-bit values into one byte.
fn inter8(a: u16, b: u16) -> u16 {
    let mut value = 0u16;
    for k in 0..4 {
        value |= (((a >> k) & 1) << (2 * k)) | (((b >> k) & 1) << (2 * k + 1));
    }
    value
}

/// Pack four 3-bit index pairs into the 12-bit group used by BC3/BC4/BC5.
fn index_bits(left: (u8, u8), right: (u8, u8)) -> u64 {
    u64::from(left.0)
        | (u64::from(left.1) << 3)
        | (u64::from(right.0) << 6)
        | (u64::from(right.1) << 9)
}

/// Write two colour endpoints and the four quadrant bytes shared by BC1/BC3.
fn write_quad(out: &mut [u8], at: usize, c0: u16, c1: u16, quads: [u8; 4]) {
    out[at..at + 2].copy_from_slice(&c0.to_le_bytes());
    out[at + 2..at + 4].copy_from_slice(&c1.to_le_bytes());
    out[at + 4] = (quads[0] & 15) | ((quads[1] & 15) << 4);
    out[at + 5] = (quads[0] >> 4) | ((quads[1] >> 4) << 4);
    out[at + 6] = (quads[2] & 15) | ((quads[3] & 15) << 4);
    out[at + 7] = (quads[2] >> 4) | ((quads[3] >> 4) << 4);
}

fn builtins() -> io::Result<&'static Builtins> {
    static BUILTINS: OnceLock<Result<Builtins, String>> = OnceLock::new();
    BUILTINS
        .get_or_init(|| {
            Builtins::parse(include_str!("history_bc_tables.json")).map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| input(format!("bundled classic BC tables are invalid: {e}")))
}

impl Builtins {
    fn parse(text: &str) -> io::Result<Self> {
        let document: Value = serde_json::from_str(text)
            .map_err(|e| input(format!("bundled classic BC tables: {e}")))?;
        let codecs = document
            .get("codecs")
            .and_then(Value::as_object)
            .ok_or_else(|| input("bundled classic BC tables: missing codecs object"))?;
        Ok(Self {
            bc1: parse_builtin(codecs.get("bc1"), "bc1", &BC1_MODS, Kind::Bc1)?,
            bc3: parse_builtin(codecs.get("bc3"), "bc3", &BC3_MODS, Kind::Bc3)?,
            bc4: parse_builtin(codecs.get("bc4"), "bc4", &BC4_MODS, Kind::Bc4)?,
            bc5: parse_builtin(codecs.get("bc5"), "bc5", &BC5_MODS, Kind::Bc5)?,
        })
    }
}

fn parse_builtin(
    value: Option<&Value>,
    key: &str,
    mods: &[u16],
    kind: Kind,
) -> io::Result<BuiltinCodec> {
    let section = value
        .and_then(Value::as_object)
        .ok_or_else(|| input(format!("bundled {key} history section is missing")))?;
    let confirmed = parse_map(section.get("confirmed"), mods, key, "confirmed")?;
    let provisional = parse_map(section.get("provisional"), mods, key, "provisional")?;
    let books = match kind {
        Kind::Bc1 => Codebooks::Bc1 {
            quadrant: parse_u8_list(section.get("quadrant"), key, "quadrant", 256)?,
        },
        Kind::Bc3 => Codebooks::Bc3 {
            alpha_pair: parse_pairs(section.get("alpha_pair"), key, "alpha_pair", 64)?,
            colour_quadrant: parse_u8_list(
                section.get("colour_quadrant"),
                key,
                "colour_quadrant",
                256,
            )?,
        },
        Kind::Bc4 => Codebooks::Bc4 {
            pair: parse_pairs(section.get("pair"), key, "pair", 64)?,
        },
        Kind::Bc5 => Codebooks::Bc5 {
            pair: parse_pairs(section.get("pair"), key, "pair", 64)?,
        },
    };
    Ok(BuiltinCodec {
        books,
        confirmed,
        provisional,
    })
}

/// Parse a 256-entry history table from either a dense array or a sparse
/// index-keyed object. Missing tables are empty; malformed tables are
/// `InvalidInput`.
fn parse_map(value: Option<&Value>, mods: &[u16], codec: &str, label: &str) -> io::Result<Entries> {
    let mut entries: Entries = vec![None; 256];
    let Some(value) = value else {
        return Ok(entries);
    };
    match value {
        Value::Array(items) => {
            if items.len() != entries.len() {
                return Err(input(format!(
                    "{codec} {label} must have 256 entries, got {}",
                    items.len()
                )));
            }
            for (index, item) in items.iter().enumerate() {
                entries[index] = Some(parse_entry(item, mods, codec, label, index)?);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let index: usize = key.parse().map_err(|_| {
                    input(format!("{codec} {label} index {key:?} is not an integer"))
                })?;
                if index >= entries.len() {
                    return Err(input(format!(
                        "{codec} {label} index {index} outside 0..255"
                    )));
                }
                entries[index] = Some(parse_entry(item, mods, codec, label, index)?);
            }
        }
        _ => {
            return Err(input(format!(
                "{codec} {label} must be a 256-element array or index-keyed object"
            )));
        }
    }
    Ok(entries)
}

fn parse_entry(
    value: &Value,
    mods: &[u16],
    codec: &str,
    label: &str,
    index: usize,
) -> io::Result<Vec<u16>> {
    let items = value.as_array().ok_or_else(|| {
        input(format!(
            "{codec} {label}[{index}] must have {} integers",
            mods.len()
        ))
    })?;
    if items.len() != mods.len() {
        return Err(input(format!(
            "{codec} {label}[{index}] must have {} integers, got {}",
            mods.len(),
            items.len()
        )));
    }
    let mut entry = Vec::with_capacity(mods.len());
    for (item, &modulus) in items.iter().zip(mods) {
        let value = item.as_u64().ok_or_else(|| {
            input(format!(
                "{codec} {label}[{index}] entries must be non-negative integers"
            ))
        })?;
        if value >= u64::from(modulus) {
            return Err(input(format!(
                "{codec} {label}[{index}] value {value} outside 0..{}",
                modulus - 1
            )));
        }
        entry.push(value as u16);
    }
    Ok(entry)
}

fn parse_u8_list(
    value: Option<&Value>,
    codec: &str,
    label: &str,
    len: usize,
) -> io::Result<Vec<u8>> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| input(format!("bundled {codec} {label} must be an array")))?;
    if items.len() != len {
        return Err(input(format!(
            "bundled {codec} {label} must have {len} entries, got {}",
            items.len()
        )));
    }
    items
        .iter()
        .map(|item| {
            let value = item.as_u64().ok_or_else(|| {
                input(format!("bundled {codec} {label} entries must be integers"))
            })?;
            u8::try_from(value).map_err(|_| {
                input(format!(
                    "bundled {codec} {label} entry {value} is not a byte"
                ))
            })
        })
        .collect()
}

fn parse_pairs(
    value: Option<&Value>,
    codec: &str,
    label: &str,
    len: usize,
) -> io::Result<Vec<(u8, u8)>> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| input(format!("bundled {codec} {label} must be an array")))?;
    if items.len() != len {
        return Err(input(format!(
            "bundled {codec} {label} must have {len} entries, got {}",
            items.len()
        )));
    }
    items
        .iter()
        .map(|item| {
            let pair = item
                .as_array()
                .ok_or_else(|| input(format!("bundled {codec} {label} entries must be pairs")))?;
            if pair.len() != 2 {
                return Err(input(format!(
                    "bundled {codec} {label} entries must be pairs"
                )));
            }
            let mut values = [0u8; 2];
            for (slot, item) in values.iter_mut().zip(pair) {
                let value = item.as_u64().ok_or_else(|| {
                    input(format!("bundled {codec} {label} entries must be integers"))
                })?;
                if value > 7 {
                    return Err(input(format!(
                        "bundled {codec} {label} rank {value} outside 0..7"
                    )));
                }
                *slot = value as u8;
            }
            Ok((values[0], values[1]))
        })
        .collect()
}

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn unsupported(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_lanes(codec: u8, nblocks: usize) -> Vec<Vec<u16>> {
        let widths: &[u8; 12] = match codec {
            1 => &BC1_LANE_WIDTHS,
            3 => &BC3_LANE_WIDTHS,
            4 => &BC4_LANE_WIDTHS,
            5 => &BC5_LANE_WIDTHS,
            _ => panic!("unsupported test codec"),
        };
        let per_block = if codec == 1 || codec == 4 { 1 } else { 2 };
        let len = nblocks * per_block;
        widths.iter().map(|_| vec![0u16; len]).collect()
    }

    fn decode(
        tables: &Tables,
        codec: u8,
        lanes: &[Vec<u16>],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        let refs: Vec<&[u16]> = lanes.iter().map(Vec::as_slice).collect();
        tables.decode(codec, &refs, nblocks, allow_provisional)
    }

    fn builtin() -> Tables {
        Tables::new(None).expect("bundled tables must parse")
    }

    fn bc1_color_bytes(entry: [u16; 6]) -> [u8; 4] {
        let c0 = (entry[0] << 11) | (entry[1] << 5) | entry[2];
        let c1 = (entry[3] << 11) | (entry[4] << 5) | entry[5];
        [c0 as u8, (c0 >> 8) as u8, c1 as u8, (c1 >> 8) as u8]
    }

    #[test]
    fn bc1_zero_delta_block_uses_the_zero_base_and_rank_zero() {
        let lanes = empty_lanes(1, 1);
        let rebuilt = decode(&builtin(), 1, &lanes, 1, false).unwrap();
        assert_eq!(rebuilt.bytes, [0, 0, 0, 0, 0xaa, 0xaa, 0xaa, 0xaa]);
        assert!(rebuilt.provisional.is_empty());
    }

    #[test]
    fn bc1_confirmed_history_entry_1_is_used_verbatim() {
        let mut lanes = empty_lanes(1, 1);
        lanes[0][0] = 1; // unzig -> -1, & 0xff = 255, entry = 256 - 255 = 1
        let rebuilt = decode(&builtin(), 1, &lanes, 1, false).unwrap();
        assert_eq!(&rebuilt.bytes[..4], &bc1_color_bytes([7, 29, 6, 8, 30, 7]));
        assert!(rebuilt.provisional.is_empty());
    }

    #[test]
    fn bc1_provisional_entry_requires_opt_in_and_is_labelled() {
        let mut lanes = empty_lanes(1, 1);
        lanes[0][0] = 11; // unzig -> -6, & 0xff = 250, entry = 6
        let tables = builtin();
        let strict = decode(&tables, 1, &lanes, 1, false)
            .err()
            .expect("expected an error");
        assert_eq!(strict.kind(), io::ErrorKind::Unsupported);
        assert!(strict.to_string().contains("unconfirmed"));
        let rebuilt = decode(&tables, 1, &lanes, 1, true).unwrap();
        assert_eq!(
            &rebuilt.bytes[..4],
            &bc1_color_bytes([12, 24, 11, 7, 14, 6])
        );
        assert_eq!(rebuilt.provisional, ["BC1 history 6"]);
    }

    #[test]
    fn replaced_history_can_create_an_unknown_entry_that_is_never_fabricated() {
        let tables = Tables::new(Some(&json!({
            "bc1": {"replace": true, "confirmed": {}}
        })))
        .unwrap();
        let mut lanes = empty_lanes(1, 1);
        lanes[0][0] = 1; // would be confirmed entry 1 in the bundled tables
        for allow in [false, true] {
            let error = decode(&tables, 1, &lanes, 1, allow)
                .err()
                .expect("expected an error");
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("unknown"));
        }
    }

    #[test]
    fn bc1_overlay_and_replace_change_the_reconstructed_block() {
        let mut lanes = empty_lanes(1, 1);
        lanes[0][0] = 1;
        let custom = [1, 2, 3, 4, 5, 6];
        let overlay = Tables::new(Some(&json!({
            "bc1": {"confirmed": {"1": custom}}
        })))
        .unwrap();
        let replaced = Tables::new(Some(&json!({
            "bc1": {"replace": true, "confirmed": {"1": custom}}
        })))
        .unwrap();
        let expected = bc1_color_bytes(custom);
        assert_eq!(
            &decode(&overlay, 1, &lanes, 1, false).unwrap().bytes[..4],
            &expected
        );
        assert_eq!(
            &decode(&replaced, 1, &lanes, 1, false).unwrap().bytes[..4],
            &expected
        );
        // Replacement drops the other bundled entries, overlay keeps them.
        lanes[0][0] = 3; // entry 2 in the bundled tables
        assert_eq!(
            decode(&overlay, 1, &lanes, 1, false).unwrap().bytes[..4],
            bc1_color_bytes([15, 29, 15, 16, 36, 17])
        );
        assert_eq!(
            decode(&replaced, 1, &lanes, 1, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn bc4_confirmed_entry_52_is_used_with_the_pair_rank_table() {
        let mut lanes = empty_lanes(4, 1);
        lanes[0][0] = 103; // unzig -> -52, & 0xff = 204, entry = 52
        let rebuilt = decode(&builtin(), 4, &lanes, 1, false).unwrap();
        assert_eq!(&rebuilt.bytes[..2], &[217, 182]);
        // BC4_PAIR[0] = (1, 1) for all eight index lanes.
        let group = 1u64 | (1 << 3) | (1 << 6) | (1 << 9);
        let bits = (0..4).fold(0u64, |acc, r| acc | (group << (12 * r)));
        assert_eq!(&rebuilt.bytes[2..8], &bits.to_le_bytes()[..6]);
    }

    #[test]
    fn bc3_provisional_entry_2_requires_opt_in() {
        let mut lanes = empty_lanes(3, 1);
        // inter8(1, 1) = 3, unzig -> -2, & 0xff = 254, entry = 2.
        lanes[0][0] = 1;
        lanes[0][1] = 1;
        let tables = builtin();
        let strict = decode(&tables, 3, &lanes, 1, false)
            .err()
            .expect("expected an error");
        assert_eq!(strict.kind(), io::ErrorKind::Unsupported);
        assert!(strict.to_string().contains("unconfirmed"));
        let rebuilt = decode(&tables, 3, &lanes, 1, true).unwrap();
        assert_eq!(&rebuilt.bytes[..2], &[177, 134]);
        let c0 = (27u16 << 11) | (55 << 5) | 24;
        let c1 = (14u16 << 11) | (54 << 5) | 24;
        assert_eq!(&rebuilt.bytes[8..10], &c0.to_le_bytes());
        assert_eq!(&rebuilt.bytes[10..12], &c1.to_le_bytes());
        assert_eq!(&rebuilt.bytes[12..16], &[0x55, 0x55, 0x55, 0x55]);
        assert_eq!(rebuilt.provisional, ["BC3 history 2"]);
    }

    #[test]
    fn bc5_confirmed_entry_105_is_used_for_both_channels() {
        let mut lanes = empty_lanes(5, 1);
        // inter8(13, 8) = 209, unzig -> -105, & 0xff = 151, entry = 105.
        lanes[0][0] = 13;
        lanes[0][1] = 8;
        let rebuilt = decode(&builtin(), 5, &lanes, 1, false).unwrap();
        assert_eq!(&rebuilt.bytes[..2], &[219, 153]);
        assert_eq!(&rebuilt.bytes[8..10], &[195, 135]);
        // BC5_PAIR[0] = (0, 0), so every index group is zero.
        assert_eq!(&rebuilt.bytes[2..8], &[0; 6]);
        assert_eq!(&rebuilt.bytes[10..16], &[0; 6]);
    }

    #[test]
    fn provisional_overlay_is_labelled_and_replace_drops_defaults() {
        let mut lanes = empty_lanes(5, 1);
        // inter8(3, 0) = 5, unzig -> -3, & 0xff = 253, entry = 3.
        lanes[0][0] = 3;
        lanes[0][1] = 0;
        let overlay = Tables::new(Some(&json!({
            "bc5": {"provisional": {"3": [1, 2, 3, 4]}}
        })))
        .unwrap();
        let rebuilt = decode(&overlay, 5, &lanes, 1, true).unwrap();
        assert_eq!(&rebuilt.bytes[..2], &[1, 2]);
        assert_eq!(&rebuilt.bytes[8..10], &[3, 4]);
        assert_eq!(rebuilt.provisional, ["BC5 history 3"]);
        let replaced = Tables::new(Some(&json!({
            "bc5": {"replace": true, "confirmed": {}}
        })))
        .unwrap();
        assert_eq!(
            decode(&replaced, 5, &lanes, 1, true)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn lane_shape_and_reserved_lane_are_validated_before_output() {
        let tables = builtin();
        let lanes = empty_lanes(1, 1);
        let mut refs: Vec<&[u16]> = lanes.iter().map(Vec::as_slice).collect();
        refs.pop();
        assert_eq!(
            tables
                .decode(1, &refs, 1, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let mut lanes = empty_lanes(1, 1);
        lanes[1][0] = 1;
        assert_eq!(
            decode(&tables, 1, &lanes, 1, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let mut lanes = empty_lanes(1, 1);
        lanes[0].push(0);
        assert_eq!(
            decode(&tables, 1, &lanes, 1, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let mut lanes = empty_lanes(1, 1);
        lanes[2][0] = 32; // BC1 lane 2 is 5 bits
        assert_eq!(
            decode(&tables, 1, &lanes, 1, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::InvalidData
        );

        assert_eq!(
            tables
                .decode(6, &[], 0, false)
                .err()
                .expect("expected an error")
                .kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn malformed_external_sections_are_invalid_input() {
        for document in [
            json!({"bc1": {"confirmed": {"1": [0, 0, 0, 0, 0]}}}),
            json!({"bc1": {"confirmed": {"256": [0, 0, 0, 0, 0, 0]}}}),
            json!({"bc1": {"confirmed": {"1": [32, 0, 0, 0, 0, 0]}}}),
            json!({"bc1": {"replace": 1}}),
            json!({"bc1": {"codec": "bc3"}}),
            json!({"bc3": {"confirmed": {"1": [0, 0, 0, 0, 0, 0, 64, 0]}}}),
            json!({"bc4": {"confirmed": {"1": [256, 0]}}}),
            json!({"bc5": {"provisional": {"1": [0, 0, 0]}}}),
        ] {
            let error = Tables::new(Some(&document))
                .err()
                .expect("expected an error");
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidInput,
                "document {document}"
            );
        }
    }
}
