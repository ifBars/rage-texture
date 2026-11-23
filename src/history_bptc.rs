//! Leonida-compatible BC6H and BC7 BCPack block rebuilders.
//!
//! Adapted from Leonida `tools/bcpack.py` (`_bc6h_blocks`, `_bc7_check`,
//! `_bc7_rebuild`) and `tools/bcpack_tables_bptc.py` at revision
//! `aa2988cc555bad7ecd508b48a702adbb32fe7c95`. Leonida is GPL-3.0-or-later;
//! this combined distribution is GPL-3.0-only. The BPTC partition, anchor and
//! signature tables below are public D3D11/Khronos specification data plus the
//! BC7 lane-15 signature permutation from the same upstream file.
//!
//! This module owns no file access and performs no entropy decoding. The
//! caller supplies the already-decoded 16 lane streams. History JSON is parsed
//! and validated once by [`Tables::new`]; `decode` never touches JSON or paths.
//!
//! Strict/inferred policy: a dependency on a missing history entry always
//! fails, even when provisional decoding is allowed. `allow_provisional` only
//! permits entries whose data is present but whose confirmation class/bits are
//! unconfirmed, plus the known inferred BC7 signature ranks; every such use is
//! labelled in [`Rebuilt::provisional`]. No unknown history placeholders are
//! fabricated.

use crate::history::Rebuilt;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::OnceLock;

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn unsupported(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn loose_limit(nblocks: usize) -> usize {
    (nblocks / 1024).max(2)
}

fn unzig(value: u16) -> i32 {
    let value = value as i32;
    if value & 1 == 0 {
        value >> 1
    } else {
        -((value + 1) >> 1)
    }
}

fn inter(a: u16, b: u16, bits: u32) -> u32 {
    let mut value = 0u32;
    for k in 0..bits {
        value |= (((a as u32 >> k) & 1) << (2 * k)) | (((b as u32 >> k) & 1) << (2 * k + 1));
    }
    value
}

fn inter8(a: u16, b: u16) -> u16 {
    inter(a, b, 4) as u16
}

fn deint_even(value: u16) -> u16 {
    let mut out = 0u16;
    for k in 0..4 {
        out |= ((value >> (2 * k)) & 1) << k;
    }
    out
}

fn deint_odd(value: u16) -> u16 {
    let mut out = 0u16;
    for k in 0..4 {
        out |= ((value >> (2 * k + 1)) & 1) << k;
    }
    out
}

fn add_residual(base: u32, delta: i32, mask: u32) -> u32 {
    ((base as i64 + delta as i64) & mask as i64) as u32
}

fn put_bits(acc: &mut u128, pos: &mut u32, value: u32, bits: u32) -> io::Result<()> {
    if bits == 0 {
        return Ok(());
    }
    if bits > 32 || pos.checked_add(bits).is_none_or(|end| end > 128) {
        return Err(invalid_data("BC7 block bit position exceeds 128 bits"));
    }
    let mask = (1u32 << bits) - 1;
    *acc |= ((value & mask) as u128) << *pos;
    *pos += bits;
    Ok(())
}

fn each_entry<F>(
    section: &Map<String, Value>,
    key: &str,
    label: &str,
    mut visit: F,
) -> io::Result<()>
where
    F: FnMut(usize, &Value) -> io::Result<()>,
{
    let Some(value) = section.get(key) else {
        return Ok(());
    };
    if let Some(list) = value.as_array() {
        if list.len() != 256 {
            return Err(input(format!(
                "{label} must have 256 entries, got {}",
                list.len()
            )));
        }
        for (index, entry) in list.iter().enumerate() {
            visit(index, entry)?;
        }
        return Ok(());
    }
    if let Some(map) = value.as_object() {
        for (key, entry) in map {
            let index = key
                .parse::<usize>()
                .map_err(|_| input(format!("{label} key {key:?} is not an index")))?;
            if index >= 256 {
                return Err(input(format!(
                    "{label} entry index {index} is outside 0..255"
                )));
            }
            visit(index, entry)?;
        }
        return Ok(());
    }
    Err(input(format!(
        "{label} must be a 256-element list or index-keyed object"
    )))
}

fn json_u64(value: &Value, label: &str) -> io::Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| input(format!("{label} must be a non-negative integer")))
}

fn json_range(value: &Value, label: &str, max: u64) -> io::Result<u64> {
    let value = json_u64(value, label)?;
    if value > max {
        return Err(input(format!("{label} value {value} is outside 0..{max}")));
    }
    Ok(value)
}

fn json_array<'a>(value: &'a Value, label: &str, len: usize) -> io::Result<&'a [Value]> {
    let array = value
        .as_array()
        .ok_or_else(|| input(format!("{label} must be an array")))?;
    if array.len() != len {
        return Err(input(format!(
            "{label} must have {len} integers, got {}",
            array.len()
        )));
    }
    Ok(array)
}

fn flatten_ints(value: &Value, expected: usize, label: &str) -> io::Result<Vec<u64>> {
    fn walk(value: &Value, out: &mut Vec<u64>, label: &str) -> io::Result<()> {
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, out, label)?;
                }
                Ok(())
            }
            Value::Number(_) => {
                out.push(json_u64(value, label)?);
                Ok(())
            }
            _ => Err(input(format!(
                "{label} entries must contain only integers and arrays"
            ))),
        }
    }
    let mut out = Vec::with_capacity(expected);
    walk(value, &mut out, label)?;
    if out.len() != expected {
        return Err(input(format!(
            "{label} must contain {expected} integers, got {}",
            out.len()
        )));
    }
    Ok(out)
}

fn section<'a>(
    history: Option<&'a Value>,
    codec: &str,
    marker: &str,
) -> io::Result<Option<&'a Map<String, Value>>> {
    let Some(document) = history else {
        return Ok(None);
    };
    let object = document
        .as_object()
        .ok_or_else(|| input("history document must be a JSON object"))?;
    if let Some(value) = object.get(codec) {
        return value
            .as_object()
            .map(Some)
            .ok_or_else(|| input(format!("{codec} history section must be a JSON object")));
    }
    if object.contains_key(marker) {
        return Ok(Some(object));
    }
    Ok(None)
}

// BC6H ---------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Bc6Info {
    transformed: bool,
    base: u32,
    deltas: [u32; 3],
    regions: usize,
}

const BC6H_CODE: [u8; 15] = [
    0, 0x00, 0x01, 0x02, 0x06, 0x0A, 0x0E, 0x12, 0x16, 0x1A, 0x1E, 0x03, 0x07, 0x0B, 0x0F,
];

const BC6H_INFO: [Bc6Info; 15] = [
    Bc6Info {
        transformed: false,
        base: 0,
        deltas: [0; 3],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 10,
        deltas: [5, 5, 5],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 7,
        deltas: [6, 6, 6],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 11,
        deltas: [5, 4, 4],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 11,
        deltas: [4, 5, 4],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 11,
        deltas: [4, 4, 5],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 9,
        deltas: [5, 5, 5],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 8,
        deltas: [6, 5, 5],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 8,
        deltas: [5, 6, 5],
        regions: 2,
    },
    Bc6Info {
        transformed: true,
        base: 8,
        deltas: [5, 5, 6],
        regions: 2,
    },
    Bc6Info {
        transformed: false,
        base: 6,
        deltas: [6, 6, 6],
        regions: 2,
    },
    Bc6Info {
        transformed: false,
        base: 10,
        deltas: [10, 10, 10],
        regions: 1,
    },
    Bc6Info {
        transformed: true,
        base: 11,
        deltas: [9, 9, 9],
        regions: 1,
    },
    Bc6Info {
        transformed: true,
        base: 12,
        deltas: [8, 8, 8],
        regions: 1,
    },
    Bc6Info {
        transformed: true,
        base: 16,
        deltas: [4, 4, 4],
        regions: 1,
    },
];

const BC6H_LAYOUT: [&str; 15] = [
    "",
    "gy4 by4 bz4 rw10 gw10 bw10 rx5 gz4 gy4_ gx5 bz0 gz4_ bx5 bz1 by4_ ry5 bz2 rz5 bz3 d",
    "gy5 gz4 gz5 rw7 bz0 bz1 by4 gw7 by5 bz2 gy4 bw7 bz3 bz5 bz4 rx6 gy4_ gx6 gz4_ bx6 by4_ ry6 rz6 d",
    "rw10 gw10 bw10 rx5 rw:10 gy4_ gx4 gw:10 bz0 gz4_ bx4 bw:10 bz1 by4_ ry5 bz2 rz5 bz3 d",
    "rw10 gw10 bw10 rx4 rw:10 gz4 gy4_ gx5 gw:10 gz4_ bx4 bw:10 bz1 by4_ ry4 bz0 bz2 rz4 gy4 bz3 d",
    "rw10 gw10 bw10 rx4 rw:10 by4 gy4_ gx4 gw:10 bz0 gz4_ bx5 bw:10 by4_ ry4 bz1 bz2 rz4 bz4 bz3 d",
    "rw9 by4 gw9 gy4 bw9 bz4 rx5 gz4 gy4_ gx5 bz0 gz4_ bx5 bz1 by4_ ry5 bz2 rz5 bz3 d",
    "rw8 gz4 by4 gw8 bz2 gy4 bw8 bz3 bz4 rx6 gy4_ gx5 bz0 gz4_ bx5 bz1 by4_ ry6 rz6 d",
    "rw8 bz0 by4 gw8 gy5 gy4 bw8 gz5 bz4 rx5 gz4 gy4_ gx6 gz4_ bx5 bz1 by4_ ry5 bz2 rz5 bz3 d",
    "rw8 bz1 by4 gw8 by5 gy4 bw8 bz5 bz4 rx5 gz4 gy4_ gx5 bz0 gz4_ bx6 by4_ ry5 bz2 rz5 bz3 d",
    "rw6 gz4 bz0 bz1 by4 gw6 gy5 by5 bz2 gy4 bw6 gz5 bz3 bz5 bz4 rx6 gy4_ gx6 gz4_ bx6 by4_ ry6 rz6 d",
    "rw10 gw10 bw10 rx10 gx10 bx10",
    "rw10 gw10 bw10 rx9 rw:10 gx9 gw:10 bx9 bw:10",
    "rw10 gw10 bw10 rx8 rw:11-10 gx8 gw:11-10 bx8 bw:11-10",
    "rw10 gw10 bw10 rx4 rw:15-10 gx4 gw:15-10 bx4 bw:15-10",
];

fn bc6_field(name: &str) -> Option<(i8, u8)> {
    Some(match name {
        "rw" => (0, 0),
        "gw" => (1, 0),
        "bw" => (2, 0),
        "rx" => (0, 1),
        "gx" => (1, 1),
        "bx" => (2, 1),
        "ry" => (0, 2),
        "gy" => (1, 2),
        "by" => (2, 2),
        "rz" => (0, 3),
        "gz" => (1, 3),
        "bz" => (2, 3),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct PlanOp {
    channel: i8,
    slot: u8,
    bit: u8,
    pos: u8,
}

struct Bc6Plan {
    ops: Vec<PlanOp>,
    header: u8,
}

fn build_bc6h_plan(mode: usize, layout: &str) -> io::Result<Bc6Plan> {
    let mut pos = if BC6H_CODE[mode] < 2 { 2u32 } else { 5u32 };
    let mut ops = Vec::new();
    for token in layout.split_whitespace() {
        if token == "d" {
            for bit in 0..5 {
                ops.push(PlanOp {
                    channel: -1,
                    slot: 0,
                    bit,
                    pos: u8::try_from(pos + bit as u32)
                        .map_err(|_| input("BC6H plan position overflow"))?,
                });
            }
            pos += 5;
            continue;
        }
        if token.len() < 3 {
            return Err(input(format!("malformed BC6H layout token {token:?}")));
        }
        if let Some((name, span)) = token.split_once(':') {
            let (channel, slot) =
                bc6_field(name).ok_or_else(|| input(format!("unknown BC6H field {name:?}")))?;
            let (hi, lo) = if let Some((hi, lo)) = span.split_once('-') {
                (
                    hi.parse::<u32>()
                        .map_err(|_| input(format!("malformed BC6H layout token {token:?}")))?,
                    lo.parse::<u32>()
                        .map_err(|_| input(format!("malformed BC6H layout token {token:?}")))?,
                )
            } else {
                let bit = span
                    .parse::<u32>()
                    .map_err(|_| input(format!("malformed BC6H layout token {token:?}")))?;
                (bit, bit)
            };
            if hi < lo || hi > 31 {
                return Err(input(format!("malformed BC6H layout token {token:?}")));
            }
            for bit in (lo..=hi).rev() {
                ops.push(PlanOp {
                    channel,
                    slot,
                    bit: bit as u8,
                    pos: u8::try_from(pos).map_err(|_| input("BC6H plan position overflow"))?,
                });
                pos += 1;
            }
        } else {
            let name = &token[..2];
            let (channel, slot) =
                bc6_field(name).ok_or_else(|| input(format!("unknown BC6H field {name:?}")))?;
            if token.ends_with('_') {
                let n = token[2..token.len() - 1]
                    .parse::<u32>()
                    .map_err(|_| input(format!("malformed BC6H layout token {token:?}")))?;
                for bit in 0..n {
                    ops.push(PlanOp {
                        channel,
                        slot,
                        bit: bit as u8,
                        pos: u8::try_from(pos + bit)
                            .map_err(|_| input("BC6H plan position overflow"))?,
                    });
                }
                pos += n;
            } else {
                let n = token[2..]
                    .parse::<u32>()
                    .map_err(|_| input(format!("malformed BC6H layout token {token:?}")))?;
                if name.as_bytes()[1] == b'w'
                    || name.as_bytes()[1] == b'x'
                    || name == "ry"
                    || name == "rz"
                {
                    for bit in 0..n {
                        ops.push(PlanOp {
                            channel,
                            slot,
                            bit: bit as u8,
                            pos: u8::try_from(pos + bit)
                                .map_err(|_| input("BC6H plan position overflow"))?,
                        });
                    }
                    pos += n;
                } else {
                    ops.push(PlanOp {
                        channel,
                        slot,
                        bit: n as u8,
                        pos: u8::try_from(pos).map_err(|_| input("BC6H plan position overflow"))?,
                    });
                    pos += 1;
                }
            }
        }
    }
    Ok(Bc6Plan {
        ops,
        header: u8::try_from(pos).map_err(|_| input("BC6H header position overflow"))?,
    })
}

fn bc6h_plans() -> &'static Vec<Bc6Plan> {
    static PLANS: OnceLock<Vec<Bc6Plan>> = OnceLock::new();
    PLANS.get_or_init(|| {
        BC6H_LAYOUT
            .iter()
            .enumerate()
            .map(|(mode, layout)| {
                build_bc6h_plan(mode, layout).expect("static BC6H plan must validate")
            })
            .collect()
    })
}

fn bc6h_stream_w(mode: usize, w: u32) -> u32 {
    match mode {
        13 => (w & 0x3FF) | (((w >> 10) & 1) << 11) | (((w >> 11) & 1) << 10),
        14 => {
            let mut out = w & 0x3FF;
            for bit in 10..16 {
                out |= ((w >> bit) & 1) << (25 - bit);
            }
            out
        }
        _ => w,
    }
}

const BC6H_NONE_E: [[u32; 4]; 3] = [[0x8000, 0x200, 0x20, 0x20]; 3];
const BC6H_NONE_I: [u8; 16] = [8; 16];

#[derive(Clone)]
struct Bc6hTables {
    endpoints: Vec<[[u16; 4]; 3]>,
    indices: Vec<[u8; 16]>,
    endpoint_class: Vec<u8>,
    index_class: Vec<u8>,
    endpoint_confirmed: Vec<[[u8; 4]; 3]>,
    index_confirmed: Vec<[u8; 16]>,
}

impl Bc6hTables {
    fn empty() -> Self {
        Self {
            endpoints: vec![[[0; 4]; 3]; 256],
            indices: vec![[0; 16]; 256],
            endpoint_class: vec![0; 256],
            index_class: vec![0; 256],
            endpoint_confirmed: vec![[[0; 4]; 3]; 256],
            index_confirmed: vec![[0; 16]; 256],
        }
    }

    fn new(history: Option<&Value>) -> io::Result<Self> {
        let mut tables = Self::empty();
        let Some(section) = section(history, "bc6h", "endpointHistory")? else {
            return Ok(tables);
        };

        each_entry(
            section,
            "endpointHistory",
            "BC6H endpointHistory",
            |i, value| {
                let channels = json_array(value, &format!("BC6H endpointHistory[{i}]"), 3)?;
                for (c, channel) in channels.iter().enumerate() {
                    let values =
                        json_array(channel, &format!("BC6H endpointHistory[{i}][{c}]"), 4)?;
                    for (q, value) in values.iter().enumerate() {
                        tables.endpoints[i][c][q] =
                            u16::try_from(json_range(value, "BC6H endpointHistory value", 0xFFFF)?)
                                .map_err(|_| input("BC6H endpoint history value overflow"))?;
                    }
                }
                Ok(())
            },
        )?;

        each_entry(section, "indexHistory", "BC6H indexHistory", |i, value| {
            let values = json_array(value, &format!("BC6H indexHistory[{i}]"), 16)?;
            for (p, value) in values.iter().enumerate() {
                tables.indices[i][p] = json_range(value, "BC6H indexHistory value", 15)? as u8;
            }
            Ok(())
        })?;

        each_entry(
            section,
            "endpointClass",
            "BC6H endpointClass",
            |i, value| {
                tables.endpoint_class[i] = json_range(value, "BC6H endpointClass", 3)? as u8;
                Ok(())
            },
        )?;
        each_entry(section, "indexClass", "BC6H indexClass", |i, value| {
            tables.index_class[i] = json_range(value, "BC6H indexClass", 3)? as u8;
            Ok(())
        })?;

        for i in 0..256 {
            if tables.endpoint_class[i] == 3 {
                tables.endpoint_confirmed[i] = [[16, 10, 6, 6]; 3];
            }
            if tables.index_class[i] == 3 {
                tables.index_confirmed[i] = [4; 16];
            }
        }

        if section.contains_key("endpointConfirmedBits") {
            tables.endpoint_confirmed = vec![[[0; 4]; 3]; 256];
            each_entry(
                section,
                "endpointConfirmedBits",
                "BC6H endpointConfirmedBits",
                |i, value| {
                    let channels =
                        json_array(value, &format!("BC6H endpointConfirmedBits[{i}]"), 3)?;
                    for (c, channel) in channels.iter().enumerate() {
                        let values = json_array(
                            channel,
                            &format!("BC6H endpointConfirmedBits[{i}][{c}]"),
                            4,
                        )?;
                        for (q, value) in values.iter().enumerate() {
                            let max = [16, 10, 6, 6][q];
                            tables.endpoint_confirmed[i][c][q] =
                                json_range(value, "BC6H endpointConfirmedBits value", max)? as u8;
                        }
                    }
                    Ok(())
                },
            )?;
        }
        if section.contains_key("indexConfirmedBits") {
            tables.index_confirmed = vec![[0; 16]; 256];
            each_entry(
                section,
                "indexConfirmedBits",
                "BC6H indexConfirmedBits",
                |i, value| {
                    let values = json_array(value, &format!("BC6H indexConfirmedBits[{i}]"), 16)?;
                    for (p, value) in values.iter().enumerate() {
                        tables.index_confirmed[i][p] =
                            json_range(value, "BC6H indexConfirmedBits value", 4)? as u8;
                    }
                    Ok(())
                },
            )?;
        }
        Ok(tables)
    }
}

// BC7 tables and rebuilders -------------------------------------------------

const BC7_NS: [usize; 8] = [3, 2, 3, 2, 1, 1, 1, 2];
const BC7_CB: [u32; 8] = [4, 6, 5, 7, 5, 7, 7, 5];
const BC7_AB: [u32; 8] = [0, 0, 0, 0, 6, 8, 7, 5];
const BC7_PK: [u8; 8] = [1, 2, 0, 1, 0, 0, 1, 1];
const BC7_IB: [u32; 8] = [3, 3, 2, 2, 3, 2, 4, 2];
const BC7_PB: [u32; 8] = [4, 6, 6, 6, 0, 0, 0, 6];
const BC7_LANE: [[i8; 4]; 3] = [[2, 3, 4, 5], [6, 7, 8, 9], [10, 5, 9, -1]];
const BC7_ANCHOR_BIT: u8 = 8;
const BC7_PBIT_BIT: u8 = 16;
const BC7_SIGNATURE_BIT: u8 = 32;

struct Bc7Static {
    config: Vec<(u8, u8, u8, u8)>,
    rank_of: [i16; 512],
    inferred: Vec<bool>,
}

fn make_bc7_config() -> Vec<(u8, u8, u8, u8)> {
    let mut head = [false; 64];
    for &partition in BC7_MODE2_HEAD.iter() {
        head[partition as usize] = true;
    }
    let mut config = Vec::with_capacity(286);
    for rank in 0..64 {
        config.push((7, 63 - rank, 0, 0));
    }
    config.push((6, 0, 0, 0));
    for rank in 0..4 {
        config.push((5, 0, 3 - rank, 0));
    }
    for rank in 0..8 {
        let value = 7 - rank;
        config.push((4, 0, value >> 1, value & 1));
    }
    for rank in 0..64 {
        config.push((3, 63 - rank, 0, 0));
    }
    for &partition in BC7_MODE2_HEAD.iter() {
        config.push((2, partition, 0, 0));
    }
    for rank in 0..64 {
        config.push((1, 63 - rank, 0, 0));
    }
    for partition in (8..=15).rev() {
        config.push((0, partition as u8, 0, 0));
    }
    for partition in (0..=5).rev() {
        config.push((0, partition as u8, 0, 0));
    }
    for partition in (0..64).rev() {
        if !head[partition] {
            config.push((2, partition as u8, 0, 0));
        }
    }
    config.push((0, 7, 0, 0));
    config.push((0, 6, 0, 0));
    config.push((255, 255, 0, 0));
    config
}

fn bc7_static() -> &'static Bc7Static {
    static STATIC: OnceLock<Bc7Static> = OnceLock::new();
    STATIC.get_or_init(|| {
        let config = make_bc7_config();
        let mut cells: Vec<(u8, u8)> = Vec::with_capacity(512);
        for a in 0..32u8 {
            for b in 0..16u8 {
                cells.push((a, b));
            }
        }
        cells.sort_by_key(|&(a, b)| ((a as u32 + 1) * (b as u32 + 1), a));
        let mut rank_of = [-1i16; 512];
        for (rank, &(a, b)) in cells.iter().enumerate() {
            if rank < config.len() {
                rank_of[(a as usize) * 16 + b as usize] = rank as i16;
            }
        }
        let mut inferred = vec![false; config.len()];
        for &rank in BC7_CONFIG_INFERRED.iter() {
            inferred[rank as usize] = true;
        }
        Bc7Static {
            config,
            rank_of,
            inferred,
        }
    })
}

fn bc7_rank(static_tables: &Bc7Static, a: u16, b: u16) -> Option<usize> {
    if a >= 32 || b >= 16 {
        return None;
    }
    let rank = static_tables.rank_of[(a as usize) * 16 + b as usize];
    (rank >= 0).then_some(rank as usize)
}

fn bc7_field_bits(mode: usize, rotation: u8) -> [u8; 4] {
    let mut widths = [BC7_CB[mode], BC7_CB[mode], BC7_CB[mode], BC7_AB[mode]];
    if rotation != 0 {
        let index = rotation as usize - 1;
        widths[index] = BC7_AB[mode];
        widths[3] = BC7_CB[mode];
    }
    let pbit = BC7_PK[mode];
    let mut out = [0u8; 4];
    for channel in 0..4 {
        if widths[channel] != 0 {
            out[channel] = (widths[channel] + u32::from(pbit != 0 && channel == 0)) as u8;
        }
    }
    out
}

#[derive(Clone)]
struct Bc7Tables {
    endpoints: Vec<[u8; 24]>,
    endpoint_bits: Vec<[u8; 24]>,
    endpoint_confirmed: Vec<[u8; 24]>,
    indices: Vec<[u8; 32]>,
    index_bits: Vec<[u8; 32]>,
    index_confirmed: Vec<[u8; 32]>,
    last_resort: Vec<[bool; 24]>,
}

impl Bc7Tables {
    fn empty() -> Self {
        Self {
            endpoints: vec![[0; 24]; 256],
            endpoint_bits: vec![[0; 24]; 256],
            endpoint_confirmed: vec![[0; 24]; 256],
            indices: vec![[0; 32]; 256],
            index_bits: vec![[0; 32]; 256],
            index_confirmed: vec![[0; 32]; 256],
            last_resort: vec![[false; 24]; 256],
        }
    }

    fn new(history: Option<&Value>) -> io::Result<Self> {
        let mut tables = Self::empty();
        let Some(section) = section(history, "bc7", "historyEndpoints")? else {
            return Ok(tables);
        };

        each_entry(
            section,
            "historyEndpoints",
            "BC7 historyEndpoints",
            |i, value| {
                let values = flatten_ints(value, 24, &format!("BC7 historyEndpoints[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 255 {
                        return Err(input(format!(
                            "BC7 historyEndpoints[{i}][{slot}] value {value} is outside 0..255"
                        )));
                    }
                    tables.endpoints[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;
        each_entry(
            section,
            "historyEndpointBits",
            "BC7 historyEndpointBits",
            |i, value| {
                let values = flatten_ints(value, 24, &format!("BC7 historyEndpointBits[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 8 {
                        return Err(input(format!(
                            "BC7 historyEndpointBits[{i}][{slot}] depth {value} is outside 0..8"
                        )));
                    }
                    tables.endpoint_bits[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;
        each_entry(
            section,
            "historyEndpointConfirmedBits",
            "BC7 historyEndpointConfirmedBits",
            |i, value| {
                let values =
                    flatten_ints(value, 24, &format!("BC7 historyEndpointConfirmedBits[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 8 {
                        return Err(input(format!(
                            "BC7 historyEndpointConfirmedBits[{i}][{slot}] depth {value} \
                             is outside 0..8"
                        )));
                    }
                    tables.endpoint_confirmed[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;
        each_entry(
            section,
            "historyIndices",
            "BC7 historyIndices",
            |i, value| {
                let values = flatten_ints(value, 32, &format!("BC7 historyIndices[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 15 {
                        return Err(input(format!(
                            "BC7 historyIndices[{i}][{slot}] value {value} is outside 0..15"
                        )));
                    }
                    tables.indices[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;
        each_entry(
            section,
            "historyIndexBits",
            "BC7 historyIndexBits",
            |i, value| {
                let values = flatten_ints(value, 32, &format!("BC7 historyIndexBits[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 4 {
                        return Err(input(format!(
                            "BC7 historyIndexBits[{i}][{slot}] depth {value} is outside 0..4"
                        )));
                    }
                    tables.index_bits[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;
        each_entry(
            section,
            "historyIndexConfirmedBits",
            "BC7 historyIndexConfirmedBits",
            |i, value| {
                let values =
                    flatten_ints(value, 32, &format!("BC7 historyIndexConfirmedBits[{i}]"))?;
                for (slot, value) in values.into_iter().enumerate() {
                    if value > 4 {
                        return Err(input(format!(
                            "BC7 historyIndexConfirmedBits[{i}][{slot}] depth {value} \
                             is outside 0..4"
                        )));
                    }
                    tables.index_confirmed[i][slot] = value as u8;
                }
                Ok(())
            },
        )?;

        // Leonida's mid-range fill: known 0<d<8 leading bits get a sentinel
        // below the confirmed prefix so readers taking <=d bits stay exact.
        for i in 0..256 {
            for slot in 0..24 {
                let depth = tables.endpoint_bits[i][slot];
                if depth > 0 && depth < 8 {
                    let value = tables.endpoints[i][slot];
                    let shift = 8 - depth;
                    tables.endpoints[i][slot] = ((value >> shift) << shift) | (1 << (7 - depth));
                }
            }
        }

        if let Some(value) = section
            .get("historyEndpointLastResort")
            .filter(|value| !value.is_null())
        {
            let items = value
                .as_array()
                .ok_or_else(|| input("BC7 historyEndpointLastResort must be an array"))?;
            for item in items {
                let fields = item
                    .as_array()
                    .ok_or_else(|| input("BC7 historyEndpointLastResort entries must be arrays"))?;
                let (entry, slot) = match fields.len() {
                    2 => (
                        usize::try_from(json_range(&fields[0], "BC7 last-resort entry", 255)?)
                            .unwrap(),
                        usize::try_from(json_range(&fields[1], "BC7 last-resort slot", 23)?)
                            .unwrap(),
                    ),
                    4 => {
                        let entry =
                            usize::try_from(json_range(&fields[0], "BC7 last-resort entry", 255)?)
                                .unwrap();
                        let subset =
                            usize::try_from(json_range(&fields[1], "BC7 last-resort subset", 2)?)
                                .unwrap();
                        let endpoint =
                            usize::try_from(json_range(&fields[2], "BC7 last-resort endpoint", 1)?)
                                .unwrap();
                        let channel =
                            usize::try_from(json_range(&fields[3], "BC7 last-resort channel", 3)?)
                                .unwrap();
                        (entry, subset * 8 + endpoint * 4 + channel)
                    }
                    _ => {
                        return Err(input(
                            "BC7 historyEndpointLastResort entries need 2 or 4 integers",
                        ));
                    }
                };
                tables.last_resort[entry][slot] = true;
            }
        }
        Ok(tables)
    }
}

fn bc7_subset(ns: usize, partition: usize, pixel: usize) -> usize {
    match ns {
        1 => 0,
        2 => ((BC7_PART2_PACKED[partition] >> pixel) & 1) as usize,
        _ => ((BC7_PART3_PACKED[partition] >> (2 * pixel)) & 3) as usize,
    }
}

fn bc7_anchors(ns: usize, partition: usize) -> [usize; 3] {
    match ns {
        1 => [0, 0, 0],
        2 => [0, BC7_ANCHOR2[partition] as usize, 0],
        _ => [
            0,
            BC7_ANCHOR3A[partition] as usize,
            BC7_ANCHOR3B[partition] as usize,
        ],
    }
}

/// Immutable parsed history tables for BC6H and BC7.
#[derive(Clone)]
pub(crate) struct Tables {
    bc6h: Bc6hTables,
    bc7: Bc7Tables,
}

impl Tables {
    /// Parse and validate one optional full history JSON document. The
    /// document may contain `bc6h`/`bc7` sections or be a single-codec
    /// document with `endpointHistory`/`historyEndpoints` markers.
    pub(crate) fn new(history: Option<&Value>) -> io::Result<Self> {
        // Build the fixed tables once here so malformed static data can never
        // be blamed on a later stream.
        let _ = bc6h_plans();
        let static_tables = bc7_static();
        if static_tables.config.len() != 286 {
            return Err(invalid_data(
                "BC7 signature configuration has an unexpected size",
            ));
        }
        Ok(Self {
            bc6h: Bc6hTables::new(history)?,
            bc7: Bc7Tables::new(history)?,
        })
    }

    pub(crate) fn decode(
        &self,
        codec: u8,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        if lanes.len() != 16 {
            return Err(invalid_data(format!(
                "BCPack BPTC rebuild needs 16 lanes, got {}",
                lanes.len()
            )));
        }
        let expected = nblocks
            .checked_mul(2)
            .ok_or_else(|| invalid_data("BCPack lane length overflow"))?;
        for (index, lane) in lanes.iter().enumerate() {
            if lane.len() != expected {
                return Err(invalid_data(format!(
                    "BCPack lane {index} has {} symbols, expected {expected}",
                    lane.len()
                )));
            }
        }
        match codec {
            6 => self.bc6h(lanes, nblocks, allow_provisional),
            7 => self.bc7(lanes, nblocks, allow_provisional),
            other => Err(unsupported(format!(
                "history BPTC rebuild does not handle codec {other}"
            ))),
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn bc6h(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        let l0 = lanes[0];
        let l1 = lanes[1];
        let l15 = lanes[15];
        let mut out = vec![0u8; nblocks * 16];
        let mut endpoint_state: Vec<[[u32; 4]; 3]> = vec![[[0; 4]; 3]; nblocks];
        let mut index_state: Vec<[u8; 16]> = vec![[0; 16]; nblocks];
        let mut violations = 0usize;
        let mut weak_e = BTreeSet::new();
        let mut weak_i = BTreeSet::new();
        let mut fitted_e = BTreeSet::new();
        let mut rough_e = BTreeSet::new();
        let mut fitted_i = BTreeSet::new();
        let mut rough_i = BTreeSet::new();

        for i in 0..nblocks {
            let a = 2 * i;
            let b = a + 1;
            let mode_symbol = l15[a];
            if mode_symbol > 13 {
                return Err(invalid_data(format!(
                    "BC6H block {i} mode symbol {mode_symbol} outside 0..13"
                )));
            }
            let mode = mode_symbol as usize + 1;
            let info = BC6H_INFO[mode];
            let code = BC6H_CODE[mode];
            let partition = usize::from(l15[b].min(31));

            let di = inter8(l1[a], l1[b]) as usize;
            let de = ((di as i32 + unzig(inter8(l0[a], l0[b]))) & 255) as usize;

            let eref: [[u32; 4]; 3] = if de == 0 {
                BC6H_NONE_E
            } else if i >= de {
                endpoint_state[i - de]
            } else {
                let entry = 256 + i - de;
                let class = self.bc6h.endpoint_class[entry];
                if class == 0 {
                    return Err(unsupported(format!(
                        "BC6H block {i} references unknown built-in endpoint history entry \
                         {entry}; refusing to fabricate placeholder history"
                    )));
                }
                let weak = (0..3).any(|c| {
                    let delta = info.deltas[c];
                    let need = if info.regions == 2 {
                        [info.base, delta, delta, delta]
                    } else {
                        [info.base, delta, 0, 0]
                    };
                    (0..4).any(|q| {
                        need[q] != 0
                            && u32::from(self.bc6h.endpoint_confirmed[entry][c][q]) < need[q]
                    })
                });
                match class {
                    1 => {
                        fitted_e.insert(entry);
                    }
                    2 => {
                        rough_e.insert(entry);
                    }
                    3 if weak => {
                        weak_e.insert(entry);
                    }
                    _ => {}
                }
                if (class != 3 || weak) && !allow_provisional {
                    return Err(unsupported(format!(
                        "BC6H block {i} references unconfirmed built-in endpoint history entry \
                         {entry}"
                    )));
                }
                self.bc6h.endpoints[entry].map(|channel| channel.map(u32::from))
            };

            let iref: [u8; 16] = if di == 0 {
                BC6H_NONE_I
            } else if i >= di {
                index_state[i - di]
            } else {
                let entry = 256 + i - di;
                let class = self.bc6h.index_class[entry];
                if class == 0 {
                    return Err(unsupported(format!(
                        "BC6H block {i} references unknown built-in index history entry {entry}; \
                         refusing to fabricate placeholder history"
                    )));
                }
                let required = if info.regions == 2 { 3 } else { 4 };
                let weak = self.bc6h.index_confirmed[entry]
                    .iter()
                    .any(|&depth| u32::from(depth) < required);
                match class {
                    1 => {
                        fitted_i.insert(entry);
                    }
                    2 => {
                        rough_i.insert(entry);
                    }
                    3 if weak => {
                        weak_i.insert(entry);
                    }
                    _ => {}
                }
                if (class != 3 || weak) && !allow_provisional {
                    return Err(unsupported(format!(
                        "BC6H block {i} references unconfirmed built-in index history entry {entry}"
                    )));
                }
                self.bc6h.indices[entry]
            };

            let mut block_fields = [[0u32; 4]; 3];
            let mut block_state = [[0u32; 4]; 3];
            let mw = (1u32 << info.base) - 1;
            let w_shift = 16 - info.base;
            for c in 0..3 {
                let dc = info.deltas[c];
                let md = (1u32 << dc) - 1;
                let sh = 6u32.saturating_sub(dc);
                let dz0 = unzig(inter(lanes[2 + c][a], lanes[2 + c][b], 8) as u16);
                let dz1 = unzig(inter(lanes[5 + c][a], lanes[5 + c][b], 5) as u16);
                let dz2 = unzig(lanes[8 + c][a]);
                let dz3 = unzig(lanes[8 + c][b]);
                let [r0, r1, r2, r3] = eref[c];
                let w = add_residual(r0 >> w_shift, dz0, mw);
                let x = add_residual(r1 >> (10 - dc), dz1, md);
                let y = add_residual(r2 >> sh, dz2, md);
                let z = add_residual(r3 >> sh, dz3, md);
                let x10 = x << (10 - dc);
                if info.regions == 2 {
                    block_state[c] = [w << w_shift, x10, y << sh, z << sh];
                    block_fields[c] = [bc6h_stream_w(mode, w), x, y, z];
                } else {
                    let mut v = w & md;
                    if info.transformed {
                        v ^= 1 << (dc - 1);
                    }
                    let low_y = if dc >= 6 { v >> (dc - 6) } else { v << sh };
                    block_state[c] = [w << w_shift, x10, low_y, x10 >> 4];
                    block_fields[c] = [bc6h_stream_w(mode, w), x, 0, 0];
                }
            }
            endpoint_state[i] = block_state;

            let ibits = if info.regions == 2 { 3 } else { 4 };
            let imask = (1u32 << ibits) - 1;
            let ishift = 4 - ibits;
            let mut ix_block = [0u8; 16];
            let mut si_block = [0u8; 16];
            for yy in 0..4 {
                let sa = lanes[11 + yy][a];
                let sb = lanes[11 + yy][b];
                let row = yy * 4;
                let (d0, d1, d2, d3) = if yy < 3 {
                    (deint_odd(sa), deint_even(sa), deint_odd(sb), deint_even(sb))
                } else {
                    (deint_even(sa), deint_odd(sa), deint_even(sb), deint_odd(sb))
                };
                for (off, delta) in [(0, d0), (1, d1), (2, d2), (3, d3)] {
                    let p = row + off;
                    let ix = add_residual(u32::from(iref[p]) >> ishift, unzig(delta), imask);
                    ix_block[p] = ix as u8;
                    si_block[p] = (ix << ishift) as u8;
                }
            }
            index_state[i] = si_block;

            let second = if info.regions == 2 {
                BC7_ANCHOR2[partition] as usize
            } else {
                usize::MAX
            };
            if ix_block[0] & (if info.regions == 2 { 4 } else { 8 }) != 0 {
                violations += 1;
            }
            if info.regions == 2 && ix_block[second] & 4 != 0 {
                violations += 1;
            }

            let plan = &bc6h_plans()[mode];
            let mut acc = code as u128;
            for op in &plan.ops {
                if op.channel < 0 {
                    if (partition >> op.bit) & 1 != 0 {
                        acc |= 1u128 << op.pos;
                    }
                } else if (block_fields[op.channel as usize][op.slot as usize] >> op.bit) & 1 != 0 {
                    acc |= 1u128 << op.pos;
                }
            }
            let mut at = u32::from(plan.header);
            for p in 0..16 {
                let bits = if p == 0 || p == second {
                    ibits - 1
                } else {
                    ibits
                };
                if at.checked_add(bits).is_none_or(|end| end > 128) {
                    return Err(invalid_data("BC6H block bit position exceeds 128 bits"));
                }
                acc |= ((ix_block[p] as u128) & ((1u128 << bits) - 1)) << at;
                at += bits;
            }
            if at != 128 {
                return Err(invalid_data(format!(
                    "BC6H block {i} wrote {at} bits instead of 128"
                )));
            }
            out[i * 16..i * 16 + 16].copy_from_slice(&acc.to_le_bytes());
        }

        let mut provisional = BTreeSet::new();
        if violations != 0 {
            if !allow_provisional {
                return Err(invalid_data(format!(
                    "BC6H chunk has {violations} anchor pixels with an inconsistent index"
                )));
            }
            if violations > loose_limit(nblocks) {
                return Err(invalid_data(format!(
                    "BC6H chunk has {violations} anchor pixels with an inconsistent index \
                     (provisional limit {})",
                    loose_limit(nblocks)
                )));
            }
            provisional.insert(format!(
                "BC6H anchor index inconsistent in {violations} pixels"
            ));
        }
        for (kind, entries, suffix) in [
            ("endpoint", &fitted_e, ""),
            ("endpoint", &rough_e, " (last-resort)"),
            ("endpoint", &weak_e, " (unconfirmed bits)"),
            ("index", &fitted_i, ""),
            ("index", &rough_i, " (last-resort)"),
            ("index", &weak_i, " (unconfirmed bits)"),
        ] {
            if !entries.is_empty() {
                provisional.insert(format!(
                    "BC6H {kind} history {}{suffix}",
                    entries
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(Rebuilt {
            bytes: out,
            provisional: provisional.into_iter().collect(),
        })
    }

    fn bc7(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        let static_tables = bc7_static();
        let mut provisional = BTreeSet::new();
        self.bc7_check(
            lanes,
            nblocks,
            allow_provisional,
            static_tables,
            &mut provisional,
        )?;
        let (out, status) = self.bc7_rebuild(lanes, nblocks, static_tables)?;
        let bad: Vec<usize> = status
            .iter()
            .enumerate()
            .filter_map(|(index, status)| (*status != 0).then_some(index))
            .collect();
        if !bad.is_empty() {
            let first = status[bad[0]];
            if first & (BC7_ANCHOR_BIT | BC7_PBIT_BIT) == 0 {
                return Err(invalid_data(format!(
                    "BC7 chunk failed signature check ({} / {nblocks} blocks; first: block {})",
                    bad.len(),
                    bad[0]
                )));
            }
            if !allow_provisional || provisional.is_empty() {
                return Err(invalid_data(format!(
                    "BC7 chunk failed anchor/p-bit consistency check ({} / {nblocks} blocks; \
                     first: block {})",
                    bad.len(),
                    bad[0]
                )));
            }
            if bad.len() > loose_limit(nblocks) {
                return Err(invalid_data(format!(
                    "BC7 chunk failed anchor/p-bit consistency check in {} / {nblocks} blocks \
                     (provisional limit {})",
                    bad.len(),
                    loose_limit(nblocks)
                )));
            }
            provisional.insert(format!(
                "BC7 consistency check failed in {} blocks",
                bad.len()
            ));
        }
        Ok(Rebuilt {
            bytes: out,
            provisional: provisional.into_iter().collect(),
        })
    }

    #[allow(clippy::needless_range_loop)]
    fn bc7_check(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        allow_provisional: bool,
        static_tables: &Bc7Static,
        provisional: &mut BTreeSet<String>,
    ) -> io::Result<()> {
        let l0 = lanes[0];
        let l1 = lanes[1];
        let l15 = lanes[15];
        let mut ranks = BTreeSet::new();
        for i in 0..nblocks {
            let a = l15[2 * i];
            let b = l15[2 * i + 1];
            let rank = bc7_rank(static_tables, a, b).ok_or_else(|| {
                invalid_data(format!(
                    "BC7 signature ({a}, {b}) is outside the signature table"
                ))
            })?;
            let config = static_tables.config[rank];
            if config.0 > 7 || config.1 == 255 {
                return Err(invalid_data(format!(
                    "BC7 signature ({a}, {b}) is outside the signature table"
                )));
            }
            ranks.insert(rank);
        }
        for &rank in &ranks {
            if static_tables.inferred[rank] {
                if !allow_provisional {
                    return Err(unsupported(format!(
                        "BC7 signature rank {rank} is inferred-only (unconfirmed)"
                    )));
                }
                provisional.insert(format!("BC7 signature {rank}"));
            }
        }

        let mut need_e: BTreeMap<usize, [[u8; 4]; 3]> = BTreeMap::new();
        let mut need_i: BTreeMap<usize, [u8; 2]> = BTreeMap::new();
        for i in 0..nblocks.min(256) {
            let a = 2 * i;
            let b = a + 1;
            let rank = bc7_rank(static_tables, l15[a], l15[b]).ok_or_else(|| {
                invalid_data(format!(
                    "BC7 signature ({}, {}) is outside the signature table",
                    l15[a], l15[b]
                ))
            })?;
            let (mode, _, rotation, _) = static_tables.config[rank];
            let mode = mode as usize;
            let di = inter8(l1[a], l1[b]) as usize;
            let de = ((di as i32 + unzig(inter8(l0[a], l0[b]))) & 255) as usize;
            if de > i {
                let entry = 256 + i - de;
                let bits = bc7_field_bits(mode, rotation);
                let row = need_e.entry(entry).or_insert([[0; 4]; 3]);
                for sub in 0..BC7_NS[mode] {
                    for c in 0..4 {
                        row[sub][c] = row[sub][c].max(bits[c]);
                    }
                }
            }
            if di > i {
                let entry = 256 + i - di;
                let row = need_i.entry(entry).or_insert([0; 2]);
                row[0] = row[0].max(BC7_IB[mode] as u8);
                if mode == 4 || mode == 5 {
                    row[1] = 2;
                }
            }
        }

        for (entry, want) in need_e {
            let mut missing = false;
            let mut weak = false;
            let mut last_resort = false;
            for sub in 0..3 {
                for endpoint in 0..2 {
                    for channel in 0..4 {
                        let required = want[sub][channel];
                        if required == 0 {
                            continue;
                        }
                        let slot = sub * 8 + endpoint * 4 + channel;
                        if self.bc7.endpoint_bits[entry][slot] < required {
                            missing = true;
                        }
                        if self.bc7.endpoint_confirmed[entry][slot] < required {
                            weak = true;
                        }
                        if self.bc7.last_resort[entry][slot] {
                            last_resort = true;
                        }
                    }
                }
            }
            if missing {
                return Err(unsupported(format!(
                    "BC7 chunk references unknown built-in endpoint history entry {entry}; \
                     refusing to fabricate placeholder history"
                )));
            }
            if weak {
                if !allow_provisional {
                    return Err(unsupported(format!(
                        "BC7 chunk references unconfirmed built-in endpoint history entry {entry}"
                    )));
                }
                provisional.insert(format!("BC7 endpoint history {entry}"));
            }
            if last_resort && allow_provisional {
                provisional.insert(format!("BC7 endpoint history {entry} (last-resort)"));
            }
        }

        for (entry, want) in need_i {
            let mut missing = false;
            let mut weak = false;
            for set in 0..2 {
                let required = want[set];
                if required == 0 {
                    continue;
                }
                for pixel in 0..16 {
                    let slot = set * 16 + pixel;
                    if self.bc7.index_bits[entry][slot] < required {
                        missing = true;
                    }
                    if self.bc7.index_confirmed[entry][slot] < required {
                        weak = true;
                    }
                }
            }
            if missing {
                return Err(unsupported(format!(
                    "BC7 chunk references unknown built-in index history entry {entry}; \
                     refusing to fabricate placeholder history"
                )));
            }
            if weak {
                if !allow_provisional {
                    return Err(unsupported(format!(
                        "BC7 chunk references unconfirmed built-in index history entry {entry}"
                    )));
                }
                provisional.insert(format!("BC7 index history {entry}"));
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_range_loop)]
    fn bc7_rebuild(
        &self,
        lanes: &[&[u16]],
        nblocks: usize,
        static_tables: &Bc7Static,
    ) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let zero_e = [0u8; 24];
        let zero_i = [0u8; 32];
        let l0 = lanes[0];
        let l1 = lanes[1];
        let l15 = lanes[15];
        let rows1 = [lanes[11], lanes[12], lanes[13], lanes[14]];
        let rows2 = [lanes[6], lanes[7], lanes[8], lanes[10]];
        let mut out = vec![0u8; nblocks * 16];
        let mut status = vec![0u8; nblocks];
        let mut eps: Vec<[u8; 24]> = vec![zero_e; nblocks];
        let mut ixs: Vec<[u8; 32]> = vec![zero_i; nblocks];

        for i in 0..nblocks {
            let a = 2 * i;
            let b = a + 1;
            let s15a = l15[a];
            let s15b = l15[b];
            let Some(rank) = bc7_rank(static_tables, s15a, s15b) else {
                status[i] = BC7_SIGNATURE_BIT;
                continue;
            };
            let (mode, part, rotation, isel) = static_tables.config[rank];
            if mode > 7 {
                status[i] = BC7_SIGNATURE_BIT;
                continue;
            }
            let mode = mode as usize;
            let mut st = 0u8;
            let di = inter8(l1[a], l1[b]) as usize;
            let de = ((di as i32 + unzig(inter8(l0[a], l0[b]))) & 255) as usize;
            let rep = if de == 0 {
                zero_e
            } else if i >= de {
                eps[i - de]
            } else {
                self.bc7.endpoints[256 + i - de]
            };
            let rix = if di == 0 {
                zero_i
            } else if i >= di {
                ixs[i - di]
            } else {
                self.bc7.indices[256 + i - di]
            };

            let ns = BC7_NS[mode];
            let cb = BC7_CB[mode];
            let ab = BC7_AB[mode];
            let pk = BC7_PK[mode];
            let mut widths = [cb, cb, cb, ab];
            if rotation != 0 {
                let index = rotation as usize - 1;
                widths[index] = ab;
                widths[3] = cb;
            }
            let mut val = [[[0u8; 4]; 2]; 3];
            let mut sep = [0u8; 24];
            let mut pb = [[0u8; 2]; 3];
            for sub in 0..ns {
                let lane_row = BC7_LANE[sub];
                let base_sub = sub * 8;
                for c in 0..4 {
                    let lane_index = lane_row[c];
                    let wc = widths[c];
                    if wc == 0 || lane_index < 0 {
                        continue;
                    }
                    let bits = wc + u32::from(pk != 0 && c == 0);
                    let shift = 8 - bits;
                    let mask = (1u32 << bits) - 1;
                    let lane = lanes[lane_index as usize];
                    let v0 =
                        add_residual(u32::from(rep[base_sub + c]) >> shift, unzig(lane[a]), mask)
                            as u8;
                    let v1 = add_residual(
                        u32::from(rep[base_sub + 4 + c]) >> shift,
                        unzig(lane[b]),
                        mask,
                    ) as u8;
                    val[sub][0][c] = v0;
                    val[sub][1][c] = v1;
                    sep[base_sub + c] = (u32::from(v0) << shift) as u8;
                    sep[base_sub + 4 + c] = (u32::from(v1) << shift) as u8;
                }
                if pk == 1 {
                    pb[sub][0] = val[sub][0][0] & 1;
                    pb[sub][1] = val[sub][1][0] & 1;
                } else if pk == 2 {
                    let p0 = val[sub][0][0] & 1;
                    pb[sub][0] = p0;
                    pb[sub][1] = p0;
                    if val[sub][1][0] & 1 != p0 {
                        st |= BC7_PBIT_BIT;
                    }
                }
            }
            let first: [u8; 8] = sep[0..8].try_into().unwrap();
            for sub in ns..3 {
                let base = sub * 8;
                sep[base..base + 8].copy_from_slice(&first);
            }
            eps[i] = sep;

            let b1 = BC7_IB[mode];
            let b1_shift = 4 - b1;
            let b1_mask = (1u32 << b1) - 1;
            let two = mode == 4 || mode == 5;
            let mut s1 = [0u8; 16];
            let mut s2 = [0u8; 16];
            let mut six = [0u8; 32];
            for y in 0..4 {
                let r1 = rows1[y];
                let (sa, sb) = (r1[a], r1[b]);
                let row = y * 4;
                let (d0, d1, d2, d3) = if y < 3 {
                    (deint_odd(sa), deint_even(sa), deint_odd(sb), deint_even(sb))
                } else {
                    (deint_even(sa), deint_odd(sa), deint_even(sb), deint_odd(sb))
                };
                for (off, delta) in [(0, d0), (1, d1), (2, d2), (3, d3)] {
                    let p = row + off;
                    let iv =
                        add_residual(u32::from(rix[p]) >> b1_shift, unzig(delta), b1_mask) as u8;
                    s1[p] = iv;
                    six[p] = (u32::from(iv) << b1_shift) as u8;
                }
                if two {
                    let r2 = rows2[y];
                    let (sa, sb) = (r2[a], r2[b]);
                    let (e0, e1, e2, e3) = if y < 3 {
                        (deint_odd(sa), deint_even(sa), deint_odd(sb), deint_even(sb))
                    } else {
                        (deint_even(sa), deint_odd(sa), deint_even(sb), deint_odd(sb))
                    };
                    for (off, delta) in [(0, e0), (1, e1), (2, e2), (3, e3)] {
                        let p = row + off;
                        let iv = add_residual(u32::from(rix[16 + p]) >> 2, unzig(delta), 3) as u8;
                        s2[p] = iv;
                        six[16 + p] = iv << 2;
                    }
                }
            }
            ixs[i] = six;

            if mode == 0 && part == 0 && sep.iter().all(|&v| v == 0) && s1.iter().all(|&v| v == 0) {
                status[i] = st;
                continue;
            }

            let partition = part as usize;
            let anchors = bc7_anchors(ns, partition);
            let (primary, ib, secondary, ib2): (&[u8; 16], u32, Option<&[u8; 16]>, u32) = match mode
            {
                4 => (&s2, 2, Some(&s1), 3),
                5 => (&s1, 2, Some(&s2), 2),
                _ => (&s1, b1, None, 0),
            };

            let mut acc = 1u128 << mode;
            let mut pos = mode as u32 + 1;
            let pbits = BC7_PB[mode];
            if pbits != 0 {
                put_bits(&mut acc, &mut pos, u32::from(part), pbits)?;
            }
            if mode == 4 || mode == 5 {
                put_bits(&mut acc, &mut pos, u32::from(rotation), 2)?;
            }
            if mode == 4 {
                put_bits(&mut acc, &mut pos, u32::from(isel), 1)?;
            }
            for c in 0..3 {
                let tc = if rotation != 0 && c == rotation as usize - 1 {
                    3
                } else {
                    c
                };
                let drop = pk != 0 && tc == 0;
                for sub in 0..ns {
                    for endpoint in 0..2 {
                        let mut value = u32::from(val[sub][endpoint][tc]);
                        if drop {
                            value >>= 1;
                        }
                        put_bits(&mut acc, &mut pos, value, cb)?;
                    }
                }
            }
            if ab != 0 {
                let tc = if rotation != 0 {
                    rotation as usize - 1
                } else {
                    3
                };
                let drop = pk != 0 && tc == 0;
                for sub in 0..ns {
                    for endpoint in 0..2 {
                        let mut value = u32::from(val[sub][endpoint][tc]);
                        if drop {
                            value >>= 1;
                        }
                        put_bits(&mut acc, &mut pos, value, ab)?;
                    }
                }
            }
            if pk == 1 {
                for sub in 0..ns {
                    put_bits(&mut acc, &mut pos, u32::from(pb[sub][0]), 1)?;
                    put_bits(&mut acc, &mut pos, u32::from(pb[sub][1]), 1)?;
                }
            } else if pk == 2 {
                for sub in 0..ns {
                    put_bits(&mut acc, &mut pos, u32::from(pb[sub][0]), 1)?;
                }
            }

            let ib_mask = (1u32 << ib) - 1;
            let ib_amask = (1u32 << (ib - 1)) - 1;
            for pixel in 0..16 {
                let subset = bc7_subset(ns, partition, pixel);
                let value = u32::from(primary[pixel]);
                if anchors[subset] == pixel {
                    if value >> (ib - 1) != 0 {
                        st |= BC7_ANCHOR_BIT;
                    }
                    put_bits(&mut acc, &mut pos, value & ib_amask, ib - 1)?;
                } else {
                    put_bits(&mut acc, &mut pos, value & ib_mask, ib)?;
                }
            }
            if let Some(secondary) = secondary {
                let mask = (1u32 << ib2) - 1;
                let amask = (1u32 << (ib2 - 1)) - 1;
                for pixel in 0..16 {
                    let value = u32::from(secondary[pixel]);
                    if pixel == 0 {
                        if value >> (ib2 - 1) != 0 {
                            st |= BC7_ANCHOR_BIT;
                        }
                        put_bits(&mut acc, &mut pos, value & amask, ib2 - 1)?;
                    } else {
                        put_bits(&mut acc, &mut pos, value & mask, ib2)?;
                    }
                }
            }
            if pos != 128 {
                st |= BC7_SIGNATURE_BIT;
            }
            out[i * 16..i * 16 + 16].copy_from_slice(&acc.to_le_bytes());
            status[i] = st;
        }
        Ok((out, status))
    }
}

// Public BPTC specification tables and the lane-15 signature permutation from
// Leonida tools/bcpack_tables_bptc.py (GPL-3.0-or-later, revision aa2988cc).
const BC7_PART2_PACKED: [u16; 64] = [
    0xcccc, 0x8888, 0xeeee, 0xecc8, 0xc880, 0xfeec, 0xfec8, 0xec80, 0xc800, 0xffec, 0xfe80, 0xe800,
    0xffe8, 0xff00, 0xfff0, 0xf000, 0xf710, 0x8e, 0x7100, 0x8ce, 0x8c, 0x7310, 0x3100, 0x8cce,
    0x88c, 0x3110, 0x6666, 0x366c, 0x17e8, 0xff0, 0x718e, 0x399c, 0xaaaa, 0xf0f0, 0x5a5a, 0x33cc,
    0x3c3c, 0x55aa, 0x9696, 0xa55a, 0x73ce, 0x13c8, 0x324c, 0x3bdc, 0x6996, 0xc33c, 0x9966, 0x660,
    0x272, 0x4e4, 0x4e40, 0x2720, 0xc936, 0x936c, 0x39c6, 0x639c, 0x9336, 0x9cc6, 0x817e, 0xe718,
    0xccf0, 0xfcc, 0x7744, 0xee22,
];
const BC7_PART3_PACKED: [u32; 64] = [
    0xaa685050, 0x6a5a5040, 0x5a5a4200, 0x5450a0a8, 0xa5a50000, 0xa0a05050, 0x5555a0a0, 0x5a5a5050,
    0xaa550000, 0xaa555500, 0xaaaa5500, 0x90909090, 0x94949494, 0xa4a4a4a4, 0xa9a59450, 0x2a0a4250,
    0xa5945040, 0xa425054, 0xa5a5a500, 0x55a0a0a0, 0xa8a85454, 0x6a6a4040, 0xa4a45000, 0x1a1a0500,
    0x50a4a4, 0xaaa59090, 0x14696914, 0x69691400, 0xa08585a0, 0xaa821414, 0x50a4a450, 0x6a5a0200,
    0xa9a58000, 0x5090a0a8, 0xa8a09050, 0x24242424, 0xaa5500, 0x24924924, 0x24499224, 0x50a50a50,
    0x500aa550, 0xaaaa4444, 0x66660000, 0xa5a0a5a0, 0x50a050a0, 0x69286928, 0x44aaaa44, 0x66666600,
    0xaa444444, 0x54a854a8, 0x95809580, 0x96969600, 0xa85454a8, 0x80959580, 0xaa141414, 0x96960000,
    0xaaaa1414, 0xa05050a0, 0xa0a5a5a0, 0x96000000, 0x40804080, 0xa9a8a9a8, 0xaaaaaa44, 0x2a4a5254,
];
const BC7_ANCHOR2: [u8; 64] = [
    0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0x2, 0x8,
    0x2, 0x2, 0x8, 0x8, 0xf, 0x2, 0x8, 0x2, 0x2, 0x8, 0x8, 0x2, 0x2, 0xf, 0xf, 0x6, 0x8, 0x2, 0x8,
    0xf, 0xf, 0x2, 0x8, 0x2, 0x2, 0x2, 0xf, 0xf, 0x6, 0x6, 0x2, 0x6, 0x8, 0xf, 0xf, 0x2, 0x2, 0xf,
    0xf, 0xf, 0xf, 0xf, 0x2, 0x2, 0xf,
];
const BC7_ANCHOR3A: [u8; 64] = [
    0x3, 0x3, 0xf, 0xf, 0x8, 0x3, 0xf, 0xf, 0x8, 0x8, 0x6, 0x6, 0x6, 0x5, 0x3, 0x3, 0x3, 0x3, 0x8,
    0xf, 0x3, 0x3, 0x6, 0xa, 0x5, 0x8, 0x8, 0x6, 0x8, 0x5, 0xf, 0xf, 0x8, 0xf, 0x3, 0x5, 0x6, 0xa,
    0x8, 0xf, 0xf, 0x3, 0xf, 0x5, 0xf, 0xf, 0xf, 0xf, 0x3, 0xf, 0x5, 0x5, 0x5, 0x8, 0x5, 0xa, 0x5,
    0xa, 0x8, 0xd, 0xf, 0xc, 0x3, 0x3,
];
const BC7_ANCHOR3B: [u8; 64] = [
    0xf, 0x8, 0x8, 0x3, 0xf, 0xf, 0x3, 0x8, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0x8, 0xf, 0x8, 0xf,
    0x3, 0xf, 0x8, 0xf, 0x8, 0x3, 0xf, 0x6, 0xa, 0xf, 0xf, 0xa, 0x8, 0xf, 0x3, 0xf, 0xa, 0xa, 0x8,
    0x9, 0xa, 0x6, 0xf, 0x8, 0xf, 0x3, 0x6, 0x6, 0x8, 0xf, 0x3, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf, 0xf,
    0xf, 0xf, 0xf, 0x3, 0xf, 0xf, 0x8,
];
const BC7_MODE2_HEAD: [u8; 22] = [
    0x3f, 0x3b, 0x3a, 0x39, 0x36, 0x35, 0x34, 0x2f, 0x2b, 0x2a, 0x28, 0x26, 0x25, 0x23, 0x22, 0x20,
    0x17, 0x15, 0x12, 0x11, 0x10, 0xb,
];
const BC7_CONFIG_INFERRED: [u8; 10] = [0x95, 0x97, 0x98, 0x99, 0xf6, 0xf8, 0xfb, 0xfc, 0xfd, 0xfe];
