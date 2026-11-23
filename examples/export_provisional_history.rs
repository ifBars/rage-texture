//! Export the bundled SixNG fitted BC6H/BC7 initial history as a
//! Leonida-compatible provisional-history document.
//!
//! The values come from the public, already-vendored SixNG fitted tables in
//! `src/bc6pack_tables.rs` and `src/bc7pack_tables.rs` (SixNG revision
//! `7881042c294f3eeea1e8153e7bcafd84f9d787b9`, GPL-3.0-only). No game files,
//! decoded payloads or generated game-derived profiles are read.
//!
//! Every exported value is **provisional**:
//!
//! * BC6H entries get class `1` (fitted) and zero confirmed-bit rows. No entry
//!   is class `3`, and no confirmed bit is claimed. Endpoints are projected
//!   from SixNG's 16-bit MSB-aligned state into Leonida's `w:16, x:10, y:6,
//!   z:6` field widths (`x >> 6`, `y/z >> 10`); this is not a raw transpose.
//! * BC7 entries get stored depth `8` for endpoints and `4` for indices, but
//!   zero confirmed bits. No last-resort claims are made.
//! * `None` slots are omitted from the JSON. Absent entries stay unknown; they
//!   are never zero-filled and never given fabricated confidence.
//!
//! Strict decoders must refuse any chunk that references these entries;
//! `allow_provisional` decoders may use them and must label the dependency.
//!
//! Usage:
//!
//! ```text
//! cargo run --example export_provisional_history -- --out history.json
//! cargo run --example export_provisional_history              # stdout
//! cargo run --example export_provisional_history -- --compact
//! ```
//!
//! `--out` uses an exclusive create: an existing file is never replaced, so a
//! stale handoff cannot be clobbered. Choose a fresh path.
//!
//! The root document has `bc6h` and `bc7` sections accepted by
//! `rage_texture::HistoryTables::from_json`, Leonida's
//! `bcpack_tables_bptc.load_bc6h`/`load_bc7`, and `bcpack.decode(...,
//! allow_provisional=True)`.
use serde_json::{Map, Value, json};
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;

// The codec modules are private in the library. Include the attributed sources
// directly, exactly as the integration tests do, so the exporter always reads
// the bundled tables rather than a duplicate copy.
#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
#[path = "../src/bc6pack.rs"]
mod bc6pack;

#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
#[path = "../src/bc7pack.rs"]
mod bc7pack;

const SIXNG_REVISION: &str = "7881042c294f3eeea1e8153e7bcafd84f9d787b9";
const LEONIDA_REVISION: &str = "aa2988cc555bad7ecd508b48a702adbb32fe7c95";

fn main() -> Result<(), Box<dyn Error>> {
    let mut output = None;
    let mut compact = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => {
                output = Some(
                    args.next()
                        .ok_or("--out requires a path after it")?
                        .to_owned(),
                );
            }
            "--compact" => compact = true,
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument {other:?}; try --help").into());
            }
        }
    }

    let document = build_document();
    let text = if compact {
        serde_json::to_string(&document)?
    } else {
        serde_json::to_string_pretty(&document)?
    };
    match output {
        Some(path) => {
            // Refuse to replace an existing file: the caller chooses a fresh
            // path, and a stale handoff cannot be silently clobbered.
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| {
                    format!(
                        "cannot create {path}: {error}; choose a new path (existing files are not overwritten)"
                    )
                })?;
            file.write_all(format!("{text}\n").as_bytes())?;
            eprintln!("wrote {path}");
        }
        None => println!("{text}"),
    }
    Ok(())
}

fn print_help() {
    println!(
        "Export the bundled SixNG fitted BC6H/BC7 history as provisional JSON.\n\
         \n\
         Usage: export_provisional_history [--out PATH] [--compact]\n\
         \n\
         Without --out the document is written to stdout. --out refuses to\n\
         replace an existing file; choose a fresh path. The output is\n\
         accepted by HistoryTables::from_json and Leonida's load_bc6h/load_bc7.\n\
         All entries are provisional; strict decoders must refuse them."
    );
}

/// Build the complete JSON document. Kept separate from `main` so the tests
/// can validate the same bytes that users export.
fn build_document() -> Value {
    let (bc6h, bc6h_ep_count, bc6h_idx_count) = build_bc6h();
    let (bc7, bc7_ep_count, bc7_idx_count) = build_bc7();

    let provenance = json!({
        "format": "leonida-bcpack-history-v1",
        "generator": "rage-texture/examples/export_provisional_history.rs",
        "source": "SixNG BCPack fitted initial history tables",
        "upstream": "https://github.com/holdmysocks/SixNG",
        "source_revision": SIXNG_REVISION,
        "source_files": ["src/bc6pack_tables.rs", "src/bc7pack_tables.rs"],
        "license": "GPL-3.0-only",
        "target": "Leonida tools/bcpack_tables_bptc.py load_bc6h/load_bc7",
        "target_revision": LEONIDA_REVISION,
        "confidence": "provisional-fitted",
        "strict_policy": "refuse every referenced fitted entry",
        "provisional_policy": "allow known fitted entries and label each dependency",
        "confirmed_bits": 0,
        "notes": "Known fitted values only. Absent slots stay unknown; there is no zero fill, no class 3, no full confirmed-bit claim and no last-resort claim. No game data or generated game-derived profile is read.",
        "counts": {
            "bc6h_endpoint_entries": bc6h_ep_count,
            "bc6h_index_entries": bc6h_idx_count,
            "bc7_endpoint_entries": bc7_ep_count,
            "bc7_index_entries": bc7_idx_count,
        },
    });

    json!({
        "_provenance": provenance,
        "bc6h": bc6h,
        "bc7": bc7,
    })
}

/// BC6H: project `EpState[4][3]` (`[w,x,y,z][R,G,B]`, all 16-bit MSB-aligned)
/// into Leonida's `endpointHistory[i][channel][w,x,y,z]` field widths
/// (`w:16, x:10, y:6, z:6`). This is not a plain transpose: `x >> 6`,
/// `y >> 10`, `z >> 10`. Index state is already the 16-entry, 4-bit
/// MSB-aligned form Leonida expects.
fn build_bc6h() -> (Value, usize, usize) {
    let tables = bc6pack::tables();

    let mut endpoints = Map::new();
    let mut endpoint_class = Map::new();
    let mut endpoint_confirmed = Map::new();
    let mut endpoint_count = 0usize;
    for (index, entry) in tables.hist_ep.iter().enumerate() {
        let Some(state) = entry else {
            continue;
        };
        endpoint_count += 1;
        let key = index.to_string();
        let channels: Vec<Value> = (0..3)
            .map(|channel| {
                // SixNG stores every component MSB-aligned to 16 bits. Leonida
                // stores the fields at their widths: w 16, x 10, y 6, z 6.
                json!([
                    state[0][channel],
                    state[1][channel] >> 6,
                    state[2][channel] >> 10,
                    state[3][channel] >> 10
                ])
            })
            .collect();
        endpoints.insert(key.clone(), Value::Array(channels));
        endpoint_class.insert(key.clone(), json!(1));
        endpoint_confirmed.insert(
            key,
            json!([[0u16, 0, 0, 0], [0u16, 0, 0, 0], [0u16, 0, 0, 0]]),
        );
    }

    let mut indices = Map::new();
    let mut index_class = Map::new();
    let mut index_confirmed = Map::new();
    let mut index_count = 0usize;
    for (index, entry) in tables.hist_idx.iter().enumerate() {
        let Some(state) = entry else {
            continue;
        };
        index_count += 1;
        let key = index.to_string();
        indices.insert(key.clone(), json!(state));
        index_class.insert(key.clone(), json!(1));
        index_confirmed.insert(key, json!(vec![0u8; 16]));
    }

    let section = json!({
        "endpointHistory": endpoints,
        "indexHistory": indices,
        "endpointClass": endpoint_class,
        "indexClass": index_class,
        "endpointConfirmedBits": endpoint_confirmed,
        "indexConfirmedBits": index_confirmed,
    });
    (section, endpoint_count, index_count)
}

/// BC7: flatten `[subset][endpoint][RGBA]` to 24 slots and `[set][pixel]` to
/// 32 slots. The stored values are full 8-bit endpoints and 4-bit indices, so
/// the known depth is 8/4, but the confirmed-bit tables are zero.
fn build_bc7() -> (Value, usize, usize) {
    let tables = bc7pack::tables();

    let mut endpoints = Map::new();
    let mut endpoint_bits = Map::new();
    let mut endpoint_confirmed = Map::new();
    let mut endpoint_count = 0usize;
    for (index, entry) in tables.hist_ep.iter().enumerate() {
        let Some(state) = entry else {
            continue;
        };
        endpoint_count += 1;
        let key = index.to_string();
        let flattened: Vec<u8> = state
            .iter()
            .flat_map(|subset| subset.iter().flat_map(|endpoint| endpoint.iter().copied()))
            .collect();
        endpoints.insert(key.clone(), json!(flattened));
        endpoint_bits.insert(key.clone(), json!(vec![8u8; 24]));
        endpoint_confirmed.insert(key, json!(vec![0u8; 24]));
    }

    let mut indices = Map::new();
    let mut index_bits = Map::new();
    let mut index_confirmed = Map::new();
    let mut index_count = 0usize;
    for (index, entry) in tables.hist_idx.iter().enumerate() {
        let Some(state) = entry else {
            continue;
        };
        index_count += 1;
        let key = index.to_string();
        let flattened: Vec<u8> = state.iter().flat_map(|set| set.iter().copied()).collect();
        indices.insert(key.clone(), json!(flattened));
        index_bits.insert(key.clone(), json!(vec![4u8; 32]));
        index_confirmed.insert(key, json!(vec![0u8; 32]));
    }

    let section = json!({
        "historyEndpoints": endpoints,
        "historyEndpointBits": endpoint_bits,
        "historyEndpointConfirmedBits": endpoint_confirmed,
        "historyIndices": indices,
        "historyIndexBits": index_bits,
        "historyIndexConfirmedBits": index_confirmed,
    });
    (section, endpoint_count, index_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rage_texture::HistoryTables;

    fn object<'a>(value: &'a Value, key: &str) -> &'a Map<String, Value> {
        value[key]
            .as_object()
            .unwrap_or_else(|| panic!("{key} must be an object"))
    }

    fn field<'a>(map: &'a Map<String, Value>, key: &str) -> &'a Map<String, Value> {
        map[key]
            .as_object()
            .unwrap_or_else(|| panic!("{key} must be an object"))
    }

    fn array(value: &Value) -> &[Value] {
        value.as_array().expect("expected a JSON array")
    }

    fn zero_array(value: &Value) -> bool {
        array(value).iter().all(|bit| bit.as_u64() == Some(0))
    }

    fn zero_rows(value: &Value) -> bool {
        array(value).iter().all(zero_array)
    }

    #[test]
    fn output_is_accepted_by_history_tables() {
        let document = build_document();
        let bytes = serde_json::to_vec(&document).unwrap();
        let tables = HistoryTables::from_json(&bytes).unwrap();
        assert_eq!(tables.digest().len(), 32);
    }

    #[test]
    fn bc6h_projects_fitted_values_into_leonida_field_widths() {
        let document = build_document();
        let section = object(&document, "bc6h");
        let endpoints = field(section, "endpointHistory");
        let indices = field(section, "indexHistory");
        let endpoint_class = field(section, "endpointClass");
        let index_class = field(section, "indexClass");
        let endpoint_confirmed = field(section, "endpointConfirmedBits");
        let index_confirmed = field(section, "indexConfirmedBits");
        let source = bc6pack::tables();

        let mut endpoint_count = 0;
        let mut projection_changed_a_field = false;
        for (index, entry) in source.hist_ep.iter().enumerate() {
            let key = index.to_string();
            match entry {
                Some(state) => {
                    endpoint_count += 1;
                    let channels = array(&endpoints[&key]);
                    assert_eq!(channels.len(), 3);
                    for (channel, row) in channels.iter().enumerate() {
                        let row = array(row);
                        assert_eq!(row.len(), 4);
                        // SixNG stores all four components MSB-aligned to 16
                        // bits; Leonida uses field widths 16/10/6/6.
                        let expected = [
                            state[0][channel],
                            state[1][channel] >> 6,
                            state[2][channel] >> 10,
                            state[3][channel] >> 10,
                        ];
                        for q in 0..4 {
                            assert_eq!(
                                row[q].as_u64(),
                                Some(u64::from(expected[q])),
                                "entry {index} channel {channel} field {q}"
                            );
                        }
                        assert!(expected[1] <= 0x03ff, "x is a 10-bit field");
                        assert!(expected[2] <= 0x003f, "y is a 6-bit field");
                        assert!(expected[3] <= 0x003f, "z is a 6-bit field");
                        if expected[1] != state[1][channel]
                            || expected[2] != state[2][channel]
                            || expected[3] != state[3][channel]
                        {
                            projection_changed_a_field = true;
                        }
                    }
                    assert_eq!(endpoint_class[&key].as_u64(), Some(1));
                    assert!(zero_rows(&endpoint_confirmed[&key]));
                }
                None => {
                    assert!(!endpoints.contains_key(&key));
                    assert!(!endpoint_class.contains_key(&key));
                    assert!(!endpoint_confirmed.contains_key(&key));
                }
            }
        }
        assert_eq!(endpoint_count, endpoints.len());
        assert!(
            projection_changed_a_field,
            "BC6H export must project, not raw-transpose, the 16-bit state"
        );

        let mut index_count = 0;
        for (index, entry) in source.hist_idx.iter().enumerate() {
            let key = index.to_string();
            match entry {
                Some(state) => {
                    index_count += 1;
                    let values: Vec<u64> = array(&indices[&key])
                        .iter()
                        .map(|value| value.as_u64().unwrap())
                        .collect();
                    assert_eq!(
                        values,
                        state.iter().map(|v| u64::from(*v)).collect::<Vec<_>>()
                    );
                    assert_eq!(index_class[&key].as_u64(), Some(1));
                    assert!(zero_array(&index_confirmed[&key]));
                }
                None => {
                    assert!(!indices.contains_key(&key));
                    assert!(!index_class.contains_key(&key));
                    assert!(!index_confirmed.contains_key(&key));
                }
            }
        }
        assert_eq!(index_count, indices.len());

        // No class 3 and no confirmed bit anywhere in the exported section.
        assert!(
            endpoint_class
                .values()
                .all(|value| value.as_u64() == Some(1))
        );
        assert!(index_class.values().all(|value| value.as_u64() == Some(1)));
        assert!(endpoint_confirmed.values().all(zero_rows));
        assert!(index_confirmed.values().all(zero_array));
    }

    #[test]
    fn bc7_flattens_fitted_values_and_keeps_them_provisional() {
        let document = build_document();
        let section = object(&document, "bc7");
        let endpoints = field(section, "historyEndpoints");
        let endpoint_bits = field(section, "historyEndpointBits");
        let endpoint_confirmed = field(section, "historyEndpointConfirmedBits");
        let indices = field(section, "historyIndices");
        let index_bits = field(section, "historyIndexBits");
        let index_confirmed = field(section, "historyIndexConfirmedBits");
        let source = bc7pack::tables();

        let mut endpoint_count = 0;
        for (index, entry) in source.hist_ep.iter().enumerate() {
            let key = index.to_string();
            match entry {
                Some(state) => {
                    endpoint_count += 1;
                    let expected: Vec<u64> = state
                        .iter()
                        .flat_map(|subset| {
                            subset.iter().flat_map(|endpoint| endpoint.iter().copied())
                        })
                        .map(u64::from)
                        .collect();
                    let actual: Vec<u64> = array(&endpoints[&key])
                        .iter()
                        .map(|value| value.as_u64().unwrap())
                        .collect();
                    assert_eq!(actual, expected, "entry {index}");
                    assert_eq!(array(&endpoint_bits[&key]).len(), 24);
                    assert!(
                        array(&endpoint_bits[&key])
                            .iter()
                            .all(|value| value.as_u64() == Some(8))
                    );
                    assert!(zero_array(&endpoint_confirmed[&key]));
                }
                None => {
                    assert!(!endpoints.contains_key(&key));
                    assert!(!endpoint_bits.contains_key(&key));
                    assert!(!endpoint_confirmed.contains_key(&key));
                }
            }
        }
        assert_eq!(endpoint_count, endpoints.len());

        let mut index_count = 0;
        for (index, entry) in source.hist_idx.iter().enumerate() {
            let key = index.to_string();
            match entry {
                Some(state) => {
                    index_count += 1;
                    let expected: Vec<u64> = state
                        .iter()
                        .flat_map(|set| set.iter().copied())
                        .map(u64::from)
                        .collect();
                    let actual: Vec<u64> = array(&indices[&key])
                        .iter()
                        .map(|value| value.as_u64().unwrap())
                        .collect();
                    assert_eq!(actual, expected, "entry {index}");
                    assert_eq!(array(&index_bits[&key]).len(), 32);
                    assert!(
                        array(&index_bits[&key])
                            .iter()
                            .all(|value| value.as_u64() == Some(4))
                    );
                    assert!(zero_array(&index_confirmed[&key]));
                }
                None => {
                    assert!(!indices.contains_key(&key));
                    assert!(!index_bits.contains_key(&key));
                    assert!(!index_confirmed.contains_key(&key));
                }
            }
        }
        assert_eq!(index_count, indices.len());

        assert!(
            endpoint_bits
                .values()
                .all(|value| { array(value).iter().all(|depth| depth.as_u64() == Some(8)) })
        );
        assert!(
            index_bits
                .values()
                .all(|value| { array(value).iter().all(|depth| depth.as_u64() == Some(4)) })
        );
        assert!(endpoint_confirmed.values().all(zero_array));
        assert!(index_confirmed.values().all(zero_array));
    }

    #[test]
    fn provenance_counts_match_the_exported_sparse_tables() {
        let document = build_document();
        let counts = object(&document["_provenance"], "counts");
        let bc6h = object(&document, "bc6h");
        let bc7 = object(&document, "bc7");
        assert_eq!(
            counts["bc6h_endpoint_entries"].as_u64(),
            Some(field(bc6h, "endpointHistory").len() as u64)
        );
        assert_eq!(
            counts["bc6h_index_entries"].as_u64(),
            Some(field(bc6h, "indexHistory").len() as u64)
        );
        assert_eq!(
            counts["bc7_endpoint_entries"].as_u64(),
            Some(field(bc7, "historyEndpoints").len() as u64)
        );
        assert_eq!(
            counts["bc7_index_entries"].as_u64(),
            Some(field(bc7, "historyIndices").len() as u64)
        );
    }
}
