//! Immutable BCPack initial-history snapshots and Leonida-compatible rebuilds.
//! Static rules/tables are attributed in the corresponding rebuild modules.
use crate::{history_bc, history_bptc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::{Arc, OnceLock},
};

const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
const DECODER_ID: &[u8] = b"rage-texture/confirmed-history/v1/leonida-aa2988cc/bcpack-5bf5cc56\0";
pub(crate) struct Rebuilt {
    pub bytes: Vec<u8>,
    pub provisional: Vec<String>,
}

/// Direct block-reconstruction result. Labels identify the known provisional
/// dependencies that were consumed; strict mode returns an empty label list.
pub struct DecodedHistory {
    pub bytes: Vec<u8>,
    pub provisional: Vec<String>,
    pub history_digest: [u8; 32],
}

/// A validated, owned snapshot. Loading a file does not create a path-based
/// lookup cache: later file changes require loading a new snapshot explicitly.
/// The digest includes canonical JSON and the decoder/static-table identity.
pub struct HistoryTables {
    classic: history_bc::Tables,
    bptc: history_bptc::Tables,
    digest: [u8; 32],
}
impl HistoryTables {
    pub fn builtin() -> Arc<Self> {
        static BUILTIN: OnceLock<Arc<HistoryTables>> = OnceLock::new();
        Arc::clone(BUILTIN.get_or_init(|| {
            Arc::new(Self::from_json(b"{}").expect("bundled history tables must validate"))
        }))
    }
    pub fn from_json(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > MAX_JSON_BYTES {
            return Err(input("history JSON exceeds 8 MiB limit"));
        }
        let document: Value = serde_json::from_slice(bytes).map_err(|e| input(e.to_string()))?;
        let document = normalize(document)?;
        validate_index_aliases(&document)?;
        let classic = history_bc::Tables::new(Some(&document))?;
        let bptc = history_bptc::Tables::new(Some(&document))?;
        let mut hasher = Sha256::new();
        hasher.update(DECODER_ID);
        hasher.update(include_bytes!("history.rs"));
        // Actual bundled source/data identities distinguish decoder/table
        // revisions even during development before a package version bump.
        hasher.update(include_bytes!("history_bc.rs"));
        hasher.update(include_bytes!("history_bc_tables.json"));
        hasher.update(include_bytes!("history_bptc.rs"));
        hasher.update(include_bytes!("bc6pack.rs"));
        hasher.update(include_bytes!("bc7pack.rs"));
        hasher.update(include_bytes!("bc7pack_tables.rs"));
        hasher.update(serde_json::to_vec(&document).map_err(|e| input(e.to_string()))?);
        Ok(Self {
            classic,
            bptc,
            digest: hasher.finalize().into(),
        })
    }
    /// Load one combined JSON document, a single-codec document, or a directory
    /// containing bc1.json, bc3.json, bc4.json, bc5.json, bc6h.json and bc7.json.
    /// Files are read afresh and bounded; no timestamp-only table cache exists.
    pub fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Self::from_json(&read_bounded(path)?);
        }
        let mut combined = Map::new();
        let mut found = false;
        let mut total = 0usize;
        for codec in ["bc1", "bc3", "bc4", "bc5", "bc6h", "bc7"] {
            let file = path.join(format!("{codec}.json"));
            let bytes = match read_bounded(&file) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| input("history directory size overflow"))?;
            if total as u64 > MAX_JSON_BYTES {
                return Err(input("combined history JSON exceeds 8 MiB limit"));
            }
            let document: Value =
                serde_json::from_slice(&bytes).map_err(|e| input(e.to_string()))?;
            let normalized = normalize(document)?;
            let section = normalized.get(codec).ok_or_else(|| {
                input(format!(
                    "{codec}.json does not contain a {codec} history section"
                ))
            })?;
            combined.insert(codec.into(), section.clone());
            found = true;
        }
        if !found {
            return Err(input(
                "history directory contains no recognized codec JSON files",
            ));
        }
        Self::from_json(
            &serde_json::to_vec(&Value::Object(combined)).map_err(|e| input(e.to_string()))?,
        )
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn digest_hex(&self) -> String {
        self.digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Reconstruct one compressed BCPack chunk without an asset container or
    /// decoded cache. The public convenience API caps packed and decoded data
    /// at 16 MiB; Store uses its separately configured Limits.
    pub fn decode_chunk(
        &self,
        data: &[u8],
        codec: u8,
        out_bytes: usize,
        allow_provisional: bool,
    ) -> io::Result<DecodedHistory> {
        if data.len() > 16 * 1024 * 1024 || out_bytes > 16 * 1024 * 1024 {
            return Err(input("direct BCPack chunk exceeds 16 MiB limit"));
        }
        let rebuilt = self.decode(data, codec, out_bytes, allow_provisional)?;
        Ok(DecodedHistory {
            bytes: rebuilt.bytes,
            provisional: rebuilt.provisional,
            history_digest: self.digest,
        })
    }

    pub(crate) fn decode(
        &self,
        data: &[u8],
        codec: u8,
        out_bytes: usize,
        allow_provisional: bool,
    ) -> io::Result<Rebuilt> {
        let bits: &[u8] = match codec {
            1 => &bcpack::bc1::BC1_ALPHABET_BITS,
            3 => &crate::bc3pack::BC3_ALPHABET_BITS,
            4 => &crate::bc45pack::BC4_ALPHABET_BITS,
            5 => &crate::bc45pack::BC5_ALPHABET_BITS,
            6 => &crate::bc6pack::BC6_ALPHABET_BITS,
            7 => &crate::bc7pack::BC7_ALPHABET_BITS,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "not a BCPack codec",
                ));
            }
        };
        let block_bytes = if matches!(codec, 1 | 4) { 8 } else { 16 };
        if out_bytes == 0 || !out_bytes.is_multiple_of(block_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "BCPack output size is not block aligned",
            ));
        }
        let decoded = bcpack::rans::decode_slice(data, bits, out_bytes / 8)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let lanes: Vec<_> = (0..decoded.lanes).map(|i| decoded.lane(i)).collect();
        let result = if codec <= 5 {
            self.classic
                .decode(codec, &lanes, out_bytes / block_bytes, allow_provisional)?
        } else {
            self.bptc
                .decode(codec, &lanes, out_bytes / block_bytes, allow_provisional)?
        };
        if result.bytes.len() != out_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "history rebuild output size mismatch",
            ));
        }
        if !allow_provisional && !result.provisional.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "strict reconstruction used provisional history",
            ));
        }
        Ok(result)
    }
}

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn validate_index_aliases(document: &Value) -> io::Result<()> {
    const FIELDS: &[&str] = &[
        "confirmed",
        "provisional",
        "endpointHistory",
        "indexHistory",
        "endpointClass",
        "indexClass",
        "endpointConfirmedBits",
        "indexConfirmedBits",
        "historyEndpoints",
        "historyEndpointBits",
        "historyEndpointConfirmedBits",
        "historyIndices",
        "historyIndexBits",
        "historyIndexConfirmedBits",
    ];
    for section in document.as_object().into_iter().flat_map(|m| m.values()) {
        for field in FIELDS {
            if let Some(entries) = section.get(field).and_then(Value::as_object) {
                let mut seen = std::collections::HashSet::new();
                for key in entries.keys() {
                    if let Ok(index) = key.parse::<usize>()
                        && !seen.insert(index)
                    {
                        return Err(input(format!(
                            "duplicate numeric history index in {field}: {key}"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}
fn normalize(document: Value) -> io::Result<Value> {
    let object = document
        .as_object()
        .ok_or_else(|| input("history document must be a JSON object"))?;
    let mut sections = Map::new();
    for codec in ["bc1", "bc3", "bc4", "bc5", "bc6h", "bc7"] {
        if let Some(section) = object.get(codec) {
            if !section.is_object() {
                return Err(input(format!("{codec} history section must be an object")));
            }
            sections.insert(codec.into(), section.clone());
        }
    }
    if sections.is_empty() {
        if object.is_empty() {
            return Ok(Value::Object(sections));
        }
        let codec = if let Some(codec) = object.get("codec").and_then(Value::as_str) {
            match codec {
                "bc1" | "bc3" | "bc4" | "bc5" => codec,
                _ => return Err(input("unknown single-codec history identifier")),
            }
        } else if object.contains_key("endpointHistory") {
            "bc6h"
        } else if object.contains_key("historyEndpoints") {
            "bc7"
        } else {
            return Err(input("history document has no recognized codec section"));
        };
        sections.insert(codec.into(), document);
    }
    Ok(Value::Object(sections))
}
fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_JSON_BYTES {
        return Err(input("history JSON exceeds 8 MiB limit"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(input("history JSON exceeds 8 MiB limit"));
    }
    Ok(bytes)
}
