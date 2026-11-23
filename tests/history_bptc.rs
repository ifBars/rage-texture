//! Synthetic tests for the BC6H/BC7 history-dependent rebuilders.
//!
//! The fixture lanes, history JSON and expected blocks were produced with
//! Leonida's public Python `_bc6h_blocks` / `_bc7_blocks` on synthetic data.
//! No game files are used. This file includes the module directly so it can run
//! before the parent wires the private module into the crate test graph.

mod history {
    #[derive(Debug)]
    pub(crate) struct Rebuilt {
        pub bytes: Vec<u8>,
        pub provisional: Vec<String>,
    }
}

#[path = "../src/history_bptc.rs"]
mod history_bptc;

use history::Rebuilt;
use history_bptc::Tables;
use serde_json::{Value, json};
use std::io::{self, ErrorKind};

#[test]
fn bc6h_confirmed_depth_is_checked_per_colour_channel() {
    // Mode 4 has 11 base bits and R/G/B deltas of 4/5/4 bits. All three
    // delta endpoints of G need five bits; R and B each need only four.
    let mut history = json!({"bc6h":{
        "endpointHistory":{"255":[[0,0,0,0],[0,0,0,0],[0,0,0,0]]},
        "indexHistory":{"255":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]},
        "endpointClass":{"255":3}, "indexClass":{"255":3},
        "endpointConfirmedBits":{"255":[[11,4,4,4],[11,5,5,5],[11,4,4,4]]}
    }});
    let mut lanes = [[0u16; 2]; 16];
    lanes[1][0] = 1;
    lanes[15][0] = 3;
    let confirmed = decode(6, &lanes, Some(&history), false).unwrap();
    assert!(confirmed.provisional.is_empty());
    history["bc6h"]["endpointConfirmedBits"]["255"] =
        json!([[11, 6, 6, 6], [11, 4, 6, 6], [11, 6, 6, 6]]);
    assert_eq!(
        decode(6, &lanes, Some(&history), false).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert!(
        decode(6, &lanes, Some(&history), true)
            .unwrap()
            .provisional
            .iter()
            .any(|label| label.contains("unconfirmed bits"))
    );
}

fn refs(lanes: &[[u16; 2]; 16]) -> Vec<&[u16]> {
    lanes.iter().map(|lane| lane.as_slice()).collect()
}

fn decode(
    codec: u8,
    lanes: &[[u16; 2]; 16],
    history: Option<&Value>,
    allow: bool,
) -> io::Result<Rebuilt> {
    let tables = Tables::new(history)?;
    let lanes = refs(lanes);
    tables.decode(codec, &lanes, 1, allow)
}

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn bc6h_history() -> Value {
    serde_json::from_str(
        r#"{"bc6h":{
            "endpointHistory":{"255":[[100,200,300,400],[500,600,700,800],[900,1000,1100,1200]]},
            "indexHistory":{"255":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]},
            "endpointClass":{"255":3},
            "indexClass":{"255":3},
            "endpointConfirmedBits":{"255":[[0,0,0,0],[0,0,0,0],[0,0,0,0]]},
            "indexConfirmedBits":{"255":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}
        }}"#,
    )
    .unwrap()
}

fn bc7_history() -> Value {
    serde_json::from_str(
        r#"{"bc7":{
            "historyEndpoints":{"255":[0,7,14,21,28,35,42,49,56,63,70,77,84,91,98,105,112,119,126,133,140,147,154,161]},
            "historyEndpointBits":{"255":[8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8,8]},
            "historyEndpointConfirmedBits":{"255":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]},
            "historyIndices":{"255":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]},
            "historyIndexBits":{"255":[4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4,4]},
            "historyIndexConfirmedBits":{"255":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}
        }}"#,
    )
    .unwrap()
}

fn bc6h_history_lanes() -> [[u16; 2]; 16] {
    let mut lanes = [[0u16; 2]; 16];
    lanes[1][0] = 1; // di = de = 1 -> history entry 255
    lanes[11][0] = 0xAA; // keep the one-region anchor index's top bit clear
    lanes[15][0] = 13; // BC6H mode 14
    lanes
}

fn bc7_history_lanes() -> [[u16; 2]; 16] {
    let mut lanes = [[0u16; 2]; 16];
    lanes[1][0] = 1; // di = de = 1 -> history entry 255
    lanes[15][0] = 20; // BC7 signature rank 64 (mode 6)
    lanes
}

#[test]
fn bc6h_self_contained_strict() {
    let mut lanes = [[0u16; 2]; 16];
    lanes[11][0] = 0xAA;
    lanes[15][0] = 13;
    let result = decode(6, &lanes, None, false).unwrap();
    assert_eq!(result.bytes, bytes("0f000000401041048188888888888888"));
    assert!(result.provisional.is_empty());
}

#[test]
fn bc7_self_contained_strict() {
    let mut lanes = [[0u16; 2]; 16];
    lanes[15][0] = 20;
    let result = decode(7, &lanes, None, false).unwrap();
    assert_eq!(result.bytes, bytes("40000000000000000000000000000000"));
    assert!(result.provisional.is_empty());
}

#[test]
fn bc6h_unconfirmed_history_needs_provisional() {
    let history = bc6h_history();
    let lanes = bc6h_history_lanes();
    let strict = decode(6, &lanes, Some(&history), false).unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Unsupported);

    let result = decode(6, &lanes, Some(&history), true).unwrap();
    assert_eq!(result.bytes, bytes("8f0cfa081f2081071032547698badcfe"));
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label.contains("endpoint history 255 (unconfirmed bits)"))
    );
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label.contains("index history 255 (unconfirmed bits)"))
    );
}

#[test]
fn bc7_unconfirmed_history_needs_provisional() {
    let history = bc7_history();
    let lanes = bc7_history_lanes();
    let strict = decode(7, &lanes, Some(&history), false).unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Unsupported);

    let result = decode(7, &lanes, Some(&history), true).unwrap();
    assert_eq!(result.bytes, bytes("40806310395414181032547698badcfe"));
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label == "BC7 endpoint history 255")
    );
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label == "BC7 index history 255")
    );
}

#[test]
fn unknown_history_is_never_fabricated() {
    let bc6h = bc6h_history_lanes();
    let error = decode(6, &bc6h, None, true).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .to_string()
            .contains("unknown built-in endpoint history")
    );

    let bc7 = bc7_history_lanes();
    let error = decode(7, &bc7, None, true).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .to_string()
            .contains("unknown built-in endpoint history")
    );
}

#[test]
fn bc7_inferred_signature_needs_provisional() {
    let mut lanes = [[0u16; 2]; 16];
    lanes[15] = [25, 1]; // rank 149, inferred-only
    let strict = decode(7, &lanes, None, false).unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Unsupported);
    assert!(strict.to_string().contains("inferred-only"));

    let result = decode(7, &lanes, None, true).unwrap();
    assert_eq!(result.bytes, bytes("5c010000000000000000000000000000"));
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label == "BC7 signature 149")
    );
}

#[test]
fn bc7_missing_history_bits_are_refused_not_zero_filled() {
    let mut history = bc7_history();
    history["bc7"]
        .as_object_mut()
        .unwrap()
        .remove("historyEndpointBits");
    let lanes = bc7_history_lanes();
    let error = decode(7, &lanes, Some(&history), true).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .to_string()
            .contains("unknown built-in endpoint history")
    );
}

#[test]
fn malformed_history_json_is_invalid_input() {
    for bad in [
        json!({"bc6h": {"endpointHistory": []}}),
        json!({"bc6h": {"endpointHistory": {"256": [[1,2,3,4],[1,2,3,4],[1,2,3,4]]}}}),
        json!({"bc6h": {"endpointHistory": {"0": [[1,2,3],[1,2,3,4],[1,2,3,4]]}}}),
        json!({"bc6h": {"indexHistory": {"0": [0,1]}}}),
        json!({"bc6h": {"endpointClass": {"0": 4}}}),
        json!({"bc6h": {"endpointConfirmedBits": {"0": [[17,0,0,0],[0,0,0,0],[0,0,0,0]]}}}),
        json!({"bc7": {"historyEndpoints": {"0": vec![0u8; 23]}}}),
        json!({"bc7": {"historyEndpointBits": {"0": vec![9u8; 24]}}}),
        json!({"bc7": {"historyIndices": {"0": vec![16u8; 32]}}}),
        json!({"bc7": {"historyIndexConfirmedBits": {"0": vec![5u8; 32]}}}),
        json!({"bc7": {"historyEndpointLastResort": [[1, 24]]}}),
        json!([1, 2, 3]),
    ] {
        let error = match Tables::new(Some(&bad)) {
            Ok(_) => panic!("malformed table was accepted: {bad}"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{bad}");
    }
}

#[test]
fn decode_validates_lane_shape_and_codec() {
    let lanes = [[0u16; 2]; 16];
    let tables = Tables::new(None).unwrap();
    let lane_refs = refs(&lanes);
    let error = tables.decode(6, &lane_refs[..15], 1, false).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);

    let short = [[0u16; 1]; 16];
    let short_refs: Vec<&[u16]> = short.iter().map(|lane| lane.as_slice()).collect();
    let error = tables.decode(7, &short_refs, 1, false).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);

    let error = tables.decode(1, &lane_refs, 1, false).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[test]
fn confirmed_history_parses_without_provisional_labels() {
    // Class 3 without explicit confirmed-bit tables defaults to full precision
    // for the required endpoint/index widths.
    let history = json!({"bc6h":{
        "endpointHistory":{"255":[[0,0,0,0],[0,0,0,0],[0,0,0,0]]},
        "indexHistory":{"255":[8,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]},
        "endpointClass":{"255":3},
        "indexClass":{"255":3}
    }});
    let lanes = bc6h_history_lanes();
    let result = decode(6, &lanes, Some(&history), false).unwrap();
    assert!(result.provisional.is_empty());
}

#[test]
fn bc6h_class_one_history_is_used_only_provisionally() {
    let mut history = bc6h_history();
    history["bc6h"]["endpointClass"]["255"] = json!(1);
    history["bc6h"]["indexClass"]["255"] = json!(1);
    let lanes = bc6h_history_lanes();

    let strict = decode(6, &lanes, Some(&history), false).unwrap_err();
    assert_eq!(strict.kind(), ErrorKind::Unsupported);

    let result = decode(6, &lanes, Some(&history), true).unwrap();
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label.contains("endpoint history 255"))
    );
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label.contains("index history 255"))
    );
}

#[test]
fn bc6h_class_zero_history_with_values_is_still_refused() {
    let mut history = bc6h_history();
    history["bc6h"]["endpointClass"]["255"] = json!(0);
    history["bc6h"]["indexClass"]["255"] = json!(0);
    let lanes = bc6h_history_lanes();
    let error = decode(6, &lanes, Some(&history), true).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .to_string()
            .contains("unknown built-in endpoint history")
    );
}

#[test]
fn bc7_missing_required_endpoint_slot_is_refused() {
    // Mode 6 reads seven bits per channel (eight for R with its p-bit). The
    // alpha endpoint slot needs seven; six must be refused even provisionally.
    let mut history = bc7_history();
    history["bc7"]["historyEndpointBits"]["255"][3] = json!(6);
    let lanes = bc7_history_lanes();
    let error = decode(7, &lanes, Some(&history), true).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(
        error
            .to_string()
            .contains("unknown built-in endpoint history")
    );
}

#[test]
fn bc7_endpoint_bits_below_the_read_width_do_not_change_output() {
    let mut history = bc7_history();
    for (key, depth) in [
        ("historyEndpointConfirmedBits", 8u64),
        ("historyIndexConfirmedBits", 4u64),
    ] {
        let row = history["bc7"][key]["255"].as_array_mut().unwrap();
        for value in row {
            *value = json!(depth);
        }
    }
    let lanes = bc7_history_lanes();
    let baseline = decode(7, &lanes, Some(&history), false).unwrap();

    let original = history["bc7"]["historyEndpoints"]["255"][1]
        .as_u64()
        .unwrap();
    let mut low_bit = history.clone();
    low_bit["bc7"]["historyEndpoints"]["255"][1] = json!(original ^ 1);
    let low = decode(7, &lanes, Some(&low_bit), false).unwrap();
    assert_eq!(baseline.bytes, low.bytes);

    let mut top_bit = history;
    top_bit["bc7"]["historyEndpoints"]["255"][1] = json!(original ^ 0x40);
    let top = decode(7, &lanes, Some(&top_bit), false).unwrap();
    assert_ne!(baseline.bytes, top.bytes);
}

#[test]
fn external_provenance_metadata_is_ignored_by_the_parser() {
    // Converted provisional tables carry a `_provenance` object next to the
    // codec sections. It is metadata, not a table and not a confirmation.
    let mut history = bc6h_history();
    history["_provenance"] = json!({
        "confidence": "provisional-fitted",
        "confirmed_bits": 0,
        "source_revision": "702435cd56fd09de4f456ddc94d60886de7365ee"
    });
    let lanes = bc6h_history_lanes();
    let result = decode(6, &lanes, Some(&history), true).unwrap();
    assert!(
        result
            .provisional
            .iter()
            .any(|label| label.contains("unconfirmed bits"))
    );
}
