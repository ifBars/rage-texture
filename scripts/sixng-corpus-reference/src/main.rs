//! Independent all-chunk parity reference over pinned SixNG.
//!
//! This binary parses each supplied RDS5 store with `sixng-core`'s public
//! `Ttpf` reader, then decodes every real chunk in chunk-table order through
//! SixNG's public BCPack modules (and the same `bcpack-rs` BC1 dependency).
//! It does not link or call the SDK.
//!
//! Digest contract (explicitly requested):
//!   SHA-256 over the concatenation of every successful chunk's decoded bytes,
//!   in numeric chunk-table order, with no headers. Failed chunks are recorded
//!   by index and omitted from the aggregate.
//!
//! Per-chunk SHA-256 values use the same definition as the SDK
//! `examples/audit_corpus.rs` chunk line: SHA-256 over `decoded.bytes` only.
//!
//! There is no decoded-byte cache. Each successful chunk is hashed and dropped.

use flate2::read::ZlibDecoder;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sixng_core::ttpf::Ttpf;
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = "sixng-corpus-reference-v1";
const SIXNG_REV: &str = "7881042c294f3eeea1e8153e7bcafd84f9d787b9";
const BCPACK_REV: &str = "02a8c4e802800d55b55797a32dbfe8b16d7fc23f";
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
const DIGEST_CONTRACT: &str = "sha256(concat(successful chunk decoded bytes in chunk-table order 0..sentinel_index-1)); failures omitted; no headers";
const SOURCE_IDENTITY_NOTE: &str = "diagnostic only: file size + nanosecond mtime + SHA-256 of the RDS5 header/chunk table; not a full source content hash; no auto-skip";

#[derive(Debug)]
struct Config {
    manifest: PathBuf,
    out_dir: PathBuf,
    only: Option<PathBuf>,
    filter: Option<String>,
    limit_stores: Option<usize>,
    list: bool,
}

fn print_help() {
    println!(
        "Independent all-chunk SixNG RDS5/BCPack parity reference.\n\
         \n\
         Usage: sixng-corpus-reference --manifest STORES.json --out-dir DIR [options]\n\
         \n\
         --manifest PATH     JSON array of RDS5 store paths (read-only)\n\
         --out-dir DIR       fresh output directory (must be outside the store root)\n\
         --only PATH         audit one exact store path from the manifest\n\
         --filter TEXT       case-insensitive relative-path substring\n\
         --limit-stores N    audit the first N selected stores\n\
         --list              print selected manifest indices and paths, then exit\n\
         --help, -h          print this help\n\
         \n\
         Output: one durable <index>_<store>.jsonl per store plus run-summary.json.\n\
         The out-dir must be empty; there is no auto-skip. Chunk failures are\n\
         recorded by index and omitted from the aggregate digest.\n\
         Source metadata (size, nanosecond mtime, header/table SHA-256) is\n\
         diagnostic only, not a full source content identity."
    );
}

fn parse_config() -> Result<Config, String> {
    let mut manifest = None;
    let mut out_dir = None;
    let mut only = None;
    let mut filter = None;
    let mut limit_stores = None;
    let mut list = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--manifest" => {
                manifest = Some(PathBuf::from(
                    args.next().ok_or("--manifest requires a path")?,
                ));
            }
            "--out-dir" => {
                out_dir = Some(PathBuf::from(
                    args.next().ok_or("--out-dir requires a path")?,
                ));
            }
            "--only" => {
                only = Some(PathBuf::from(args.next().ok_or("--only requires a path")?));
            }
            "--filter" => {
                filter = Some(args.next().ok_or("--filter requires text")?);
            }
            "--limit-stores" => {
                let value = args.next().ok_or("--limit-stores requires a number")?;
                limit_stores = Some(
                    value
                        .parse::<usize>()
                        .map_err(|error| format!("invalid --limit-stores {value:?}: {error}"))?,
                );
            }
            "--resume" => {
                return Err(
                    "--resume was removed: source size/table/mtime is not a content identity. Choose a fresh out-dir."
                        .to_owned(),
                );
            }
            "--list" => list = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?}; try --help")),
        }
    }
    let manifest = manifest.ok_or("--manifest is required")?;
    let out_dir = if list {
        out_dir.unwrap_or_else(|| PathBuf::from("."))
    } else {
        out_dir.ok_or("--out-dir is required")?
    };
    Ok(Config {
        manifest,
        out_dir,
        only,
        filter,
        limit_stores,
        list,
    })
}

fn main() {
    match parse_config() {
        Ok(config) => {
            if let Err(error) = run(config) {
                eprintln!("sixng-corpus-reference: {error}");
                std::process::exit(1);
            }
        }
        Err(message) => {
            eprintln!("sixng-corpus-reference: {message}");
            std::process::exit(2);
        }
    }
}

fn run(config: Config) -> Result<(), Box<dyn Error>> {
    let manifest_bytes = fs::read(&config.manifest)?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let value: Value = serde_json::from_slice(&manifest_bytes)?;
    let all_paths = manifest_paths(&value)?;
    if all_paths.is_empty() {
        return Err(input_error("manifest contains no stores"));
    }
    let root = common_root(&all_paths);

    let mut selected: Vec<(usize, PathBuf)> = all_paths
        .iter()
        .enumerate()
        .filter(|(_, path)| config.only.as_ref().is_none_or(|only| only == *path))
        .filter(|(_, path)| {
            config.filter.as_ref().is_none_or(|needle| {
                normalized(path)
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            })
        })
        .map(|(index, path)| (index, path.clone()))
        .collect();
    if let Some(limit) = config.limit_stores {
        selected.truncate(limit);
    }
    if selected.is_empty() {
        return Err(input_error("no stores matched --only/--filter"));
    }

    if config.list {
        for (index, path) in &selected {
            println!("{index:03}\t{}", path.display());
        }
        return Ok(());
    }

    let out_dir = std::path::absolute(&config.out_dir)?;
    let root_abs = std::path::absolute(&root)?;
    if out_dir.starts_with(&root_abs) {
        return Err(input_error(format!(
            "out-dir {} is inside the read-only store root {}",
            out_dir.display(),
            root_abs.display()
        )));
    }
    fs::create_dir_all(&out_dir)?;
    let mut entries = fs::read_dir(&out_dir)?;
    if entries.next().is_some() {
        return Err(input_error(format!(
            "out-dir {} is not empty; choose a fresh directory (no auto-skip)",
            out_dir.display()
        )));
    }

    let source_bundle_sha256 = source_bundle_sha256();
    let exe_sha256 = exe_sha256();
    let cargo_lock_sha256 = sha256_hex(include_bytes!("../Cargo.lock"));
    let command_line = env::args().collect::<Vec<_>>().join(" ");
    let started_unix = now_unix();
    let run_started = Instant::now();
    let mut progress = Progress::new(selected.len());
    let mut summaries = Vec::with_capacity(selected.len());
    let mut stores_failed = 0u64;

    for (manifest_index, path) in &selected {
        let relative = store_relative(&root, path);
        let file_name = output_name(*manifest_index, &relative);
        let summary = process_store(
            *manifest_index,
            path,
            &relative,
            &file_name,
            &out_dir,
            &mut progress,
        )?;
        if summary.status == "failed" {
            stores_failed += 1;
        }
        progress.store_finished(&summary);
        summaries.push(summary);
    }

    let stores_done = summaries.len() as u64;
    let chunks_expected = summaries
        .iter()
        .map(|summary| summary.chunks_expected)
        .sum();
    let chunks_attempted = summaries
        .iter()
        .map(|summary| summary.chunks_attempted)
        .sum();
    let chunks_pass = summaries.iter().map(|summary| summary.chunks_pass).sum();
    let chunks_fail = summaries.iter().map(|summary| summary.chunks_fail).sum();
    let successful_bytes = summaries
        .iter()
        .map(|summary| summary.successful_bytes)
        .sum();
    let finished_unix = now_unix();
    let run = RunSummary {
        schema: SCHEMA,
        sixng_rev: SIXNG_REV,
        bcpack_rev: BCPACK_REV,
        harness_source_sha256: source_bundle_sha256,
        cargo_lock_sha256,
        exe_sha256,
        manifest: config.manifest.display().to_string(),
        manifest_sha256,
        command_line,
        out_dir: out_dir.display().to_string(),
        started_unix,
        finished_unix,
        elapsed_seconds: run_started.elapsed().as_secs_f64(),
        cache_budget_bytes: 0,
        digest_contract: DIGEST_CONTRACT,
        source_identity_note: SOURCE_IDENTITY_NOTE,
        stores_total: selected.len() as u64,
        stores_done,
        stores_failed,
        chunks_expected,
        chunks_attempted,
        chunks_pass,
        chunks_fail,
        successful_bytes,
        failure_count: chunks_fail,
        stores: summaries,
    };
    write_json_atomic(&out_dir.join("run-summary.json"), &run)?;
    progress.finish(&run);
    println!(
        "sixng-corpus-reference: {} stores ({} done, {} failed), chunks {}/{} pass {}/{} fail {}, {:.3} GiB successful, {:.1}s",
        run.stores_total,
        run.stores_done,
        run.stores_failed,
        run.chunks_pass,
        run.chunks_expected,
        run.chunks_pass,
        run.chunks_attempted,
        run.chunks_fail,
        run.successful_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        run.elapsed_seconds
    );
    println!("summary: {}", out_dir.join("run-summary.json").display());
    if stores_failed > 0 {
        return Err(input_error(format!(
            "{stores_failed} store(s) failed; see run-summary.json"
        )));
    }
    Ok(())
}

fn manifest_paths(value: &Value) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let array = value
        .as_array()
        .or_else(|| value.get("stores").and_then(Value::as_array))
        .ok_or_else(|| input_error("manifest must be a JSON array of store paths"))?;
    let mut paths = Vec::with_capacity(array.len());
    for entry in array {
        let path = entry
            .as_str()
            .ok_or_else(|| input_error("manifest store entry is not a string"))?;
        paths.push(PathBuf::from(path));
    }
    Ok(paths)
}

fn common_root(paths: &[PathBuf]) -> PathBuf {
    let Some(first) = paths.first() else {
        return PathBuf::new();
    };
    let mut root = first.clone();
    for path in &paths[1..] {
        while !path.starts_with(&root) {
            if !root.pop() {
                return PathBuf::new();
            }
        }
    }
    if !root.pop() || root.as_os_str().is_empty() {
        return PathBuf::new();
    }
    root
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn store_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn output_name(index: usize, relative: &str) -> String {
    let mut safe: String = relative
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        safe.push_str("store");
    }
    if safe.len() > 160 {
        safe.truncate(160);
    }
    format!("{index:03}_{safe}.jsonl")
}

#[derive(Clone, Serialize)]
struct StoreSummary {
    manifest_index: usize,
    store: String,
    path: String,
    file_bytes: u64,
    mtime_unix_nanos: u64,
    table_sha256: Option<String>,
    table_entries: Option<u32>,
    sentinel_index: usize,
    sentinel_source: String,
    first_virtual_offset: u64,
    end_virtual_offset: u64,
    chunks_expected: u64,
    chunks_attempted: u64,
    chunks_pass: u64,
    chunks_fail: u64,
    successful_bytes: u64,
    aggregate_sha256: String,
    failure_indices: Vec<u64>,
    failure_kinds: BTreeMap<String, u64>,
    codec_counts: BTreeMap<String, u64>,
    unfitted_blocks: u64,
    invalid_blocks: u64,
    cache_budget_bytes: u64,
    source_changed: bool,
    status: String,
    error: Option<String>,
    elapsed_seconds: f64,
}

fn process_store(
    manifest_index: usize,
    path: &Path,
    relative: &str,
    file_name: &str,
    out_dir: &Path,
    progress: &mut Progress,
) -> Result<StoreSummary, Box<dyn Error>> {
    let started = Instant::now();
    let final_path = out_dir.join(file_name);
    let (file_bytes, mtime_unix_nanos) = match source_stamp(path) {
        Ok(stamp) => stamp,
        Err(error) => {
            return Err(input_error(format!(
                "cannot stat {}: {error}",
                path.display()
            )));
        }
    };
    let table = table_digest(path).ok();
    let table_sha256 = table.as_ref().map(|table| table.0.clone());
    let table_entries = table.as_ref().map(|table| table.1);

    if final_path.exists() {
        return Err(input_error(format!(
            "{} already exists; the out-dir must be empty (no auto-skip)",
            final_path.display()
        )));
    }

    let mut writer = StoreWriter::new(final_path)?;
    writer.line(&json!({
        "kind": "store_start",
        "schema": SCHEMA,
        "store": relative,
        "path": path.display().to_string(),
        "file_bytes": file_bytes,
        "mtime_unix_nanos": mtime_unix_nanos,
        "table_sha256": table_sha256,
        "table_entries": table_entries,
        "cache_budget_bytes": 0,
        "digest_contract": DIGEST_CONTRACT,
        "source_identity_note": SOURCE_IDENTITY_NOTE,
    }))?;
    writer.maybe_flush(true)?;

    let store = match Ttpf::open(path) {
        Ok(store) => store,
        Err(error) => {
            let summary = failed_summary(
                manifest_index,
                relative,
                path,
                file_bytes,
                mtime_unix_nanos,
                table_sha256,
                table_entries,
                started,
                format!("Ttpf::open: {error}"),
            );
            writer.finish_with(&summary)?;
            return Ok(summary);
        }
    };

    let chunks = &store.chunks;
    let (sentinel_index, sentinel_source) = match sentinel(chunks, file_bytes) {
        Some(value) => value,
        None => {
            let summary = failed_summary(
                manifest_index,
                relative,
                path,
                file_bytes,
                mtime_unix_nanos,
                table_sha256,
                table_entries,
                started,
                "cannot identify the chunk-table sentinel".to_owned(),
            );
            writer.finish_with(&summary)?;
            return Ok(summary);
        }
    };
    let real_count = sentinel_index;
    if real_count == 0 {
        let summary = failed_summary(
            manifest_index,
            relative,
            path,
            file_bytes,
            mtime_unix_nanos,
            table_sha256,
            table_entries,
            started,
            "store has no real chunks before the sentinel".to_owned(),
        );
        writer.finish_with(&summary)?;
        return Ok(summary);
    }
    let first_virtual_offset = chunks[0].1;
    let end_virtual_offset = chunks[sentinel_index].1;
    if end_virtual_offset <= first_virtual_offset {
        let summary = failed_summary(
            manifest_index,
            relative,
            path,
            file_bytes,
            mtime_unix_nanos,
            table_sha256,
            table_entries,
            started,
            format!(
                "sentinel virtual offset {end_virtual_offset} is not after first chunk {first_virtual_offset}"
            ),
        );
        writer.finish_with(&summary)?;
        return Ok(summary);
    }

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            let summary = failed_summary(
                manifest_index,
                relative,
                path,
                file_bytes,
                mtime_unix_nanos,
                table_sha256,
                table_entries,
                started,
                format!("open source: {error}"),
            );
            writer.finish_with(&summary)?;
            return Ok(summary);
        }
    };

    let mut summary = StoreSummary {
        manifest_index,
        store: relative.to_owned(),
        path: path.display().to_string(),
        file_bytes,
        mtime_unix_nanos,
        table_sha256,
        table_entries,
        sentinel_index,
        sentinel_source: sentinel_source.to_owned(),
        first_virtual_offset,
        end_virtual_offset,
        chunks_expected: real_count as u64,
        chunks_attempted: 0,
        chunks_pass: 0,
        chunks_fail: 0,
        successful_bytes: 0,
        aggregate_sha256: String::new(),
        failure_indices: Vec::new(),
        failure_kinds: BTreeMap::new(),
        codec_counts: BTreeMap::new(),
        unfitted_blocks: 0,
        invalid_blocks: 0,
        cache_budget_bytes: 0,
        source_changed: false,
        status: "pass".to_owned(),
        error: None,
        elapsed_seconds: 0.0,
    };

    let mut aggregate = Sha256::new();
    let mut compressed = Vec::new();

    for index in 0..real_count {
        let (file_offset, virtual_offset, flags, pad) = chunks[index];
        let (next_file_offset, next_virtual_offset, _, _) = chunks[index + 1];
        let span = next_virtual_offset.saturating_sub(virtual_offset);
        let packed = next_file_offset
            .checked_sub(file_offset)
            .and_then(|length| length.checked_sub(pad));
        let codec = flags & 0x3F;
        let name = codec_name(codec, flags);
        let base = json!({
            "kind": "chunk",
            "store": relative,
            "chunk": index,
            "codec": codec,
            "codec_name": name,
            "file_offset": file_offset,
            "virtual_offset": virtual_offset,
            "packed_bytes": packed,
            "decoded_bytes": span,
        });

        let outcome = match packed {
            None => Err(ChunkError::new(
                "range",
                format!(
                    "file offsets reversed or padding exceeds span: offset={file_offset}, next={next_file_offset}, pad={pad}"
                ),
            )),
            Some(0) => Err(ChunkError::new("range", "zero packed bytes".to_owned())),
            Some(0..=MAX_CHUNK_BYTES) if span > MAX_CHUNK_BYTES => Err(ChunkError::new(
                "range",
                format!("decoded span {span} exceeds the {MAX_CHUNK_BYTES}-byte safety limit"),
            )),
            Some(packed) => decode_chunk(
                &mut file,
                &mut compressed,
                file_offset,
                packed,
                span,
                flags,
            ),
        };

        match outcome {
            Ok(decoded) => {
                let chunk_sha256 = sha256_hex(&decoded.bytes);
                aggregate.update(&decoded.bytes);
                summary.chunks_pass += 1;
                summary.successful_bytes += span;
                summary.unfitted_blocks += decoded.unfitted as u64;
                summary.invalid_blocks += decoded.invalid as u64;
                *summary.codec_counts.entry(codec.to_string()).or_default() += 1;
                let mut line = base;
                let object = line.as_object_mut().expect("chunk line is an object");
                object.insert("status".to_owned(), json!("pass"));
                object.insert("sha256".to_owned(), json!(chunk_sha256));
                object.insert("unfitted_blocks".to_owned(), json!(decoded.unfitted));
                object.insert("invalid_blocks".to_owned(), json!(decoded.invalid));
                writer.line(&line)?;
            }
            Err(error) => {
                summary.chunks_fail += 1;
                summary.failure_indices.push(index as u64);
                *summary
                    .failure_kinds
                    .entry(error.kind.to_owned())
                    .or_default() += 1;
                let mut line = base;
                let object = line.as_object_mut().expect("chunk line is an object");
                object.insert("status".to_owned(), json!("fail"));
                object.insert("error_kind".to_owned(), json!(error.kind));
                object.insert("error".to_owned(), json!(error.message));
                writer.line(&line)?;
            }
        }
        summary.chunks_attempted += 1;
        writer.maybe_flush(false)?;
        progress.tick(
            false,
            relative,
            summary.chunks_attempted,
            real_count as u64,
            summary.chunks_pass,
            summary.chunks_fail,
            summary.successful_bytes,
        );
    }

    summary.aggregate_sha256 = format!("{:x}", aggregate.finalize());
    summary.elapsed_seconds = started.elapsed().as_secs_f64();
    summary.source_changed = source_stamp(path)
        .map(|stamp| stamp != (file_bytes, mtime_unix_nanos))
        .unwrap_or(true);
    summary.status = if summary.source_changed {
        "source_changed".to_owned()
    } else if summary.chunks_fail > 0 {
        "completed_with_failures".to_owned()
    } else {
        "pass".to_owned()
    };
    writer.finish_with(&summary)?;
    eprintln!(
        "store {}: chunks {}/{} pass {}/{}, failures {}, {:.3} GiB, aggregate {}, {:.2}s",
        relative,
        summary.chunks_pass,
        summary.chunks_expected,
        summary.chunks_pass,
        summary.chunks_attempted,
        summary.chunks_fail,
        summary.successful_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        summary.aggregate_sha256,
        summary.elapsed_seconds
    );
    Ok(summary)
}

fn sentinel(chunks: &[(u64, u64, u8, u64)], file_bytes: u64) -> Option<(usize, &'static str)> {
    if let Some(index) = chunks
        .iter()
        .position(|&(file_offset, _, _, _)| file_offset == file_bytes)
    {
        return Some((index, "file_len"));
    }
    if chunks.len() >= 2 && chunks[chunks.len() - 1].1 <= chunks[chunks.len() - 2].1 {
        return Some((chunks.len() - 2, "etpf"));
    }
    if chunks.is_empty() {
        None
    } else {
        Some((chunks.len() - 1, "last"))
    }
}

fn decode_chunk(
    file: &mut File,
    compressed: &mut Vec<u8>,
    file_offset: u64,
    packed_bytes: u64,
    span: u64,
    flags: u8,
) -> Result<DecodedChunk, ChunkError> {
    if packed_bytes == 0 || packed_bytes > MAX_CHUNK_BYTES {
        return Err(ChunkError::new(
            "range",
            format!("packed size {packed_bytes} is outside the safety limit"),
        ));
    }
    if span == 0 || span > MAX_CHUNK_BYTES {
        return Err(ChunkError::new(
            "range",
            format!("decoded span {span} is outside the safety limit"),
        ));
    }
    file.seek(SeekFrom::Start(file_offset))
        .map_err(|error| ChunkError::new("io", format!("seek: {error}")))?;
    compressed.resize(packed_bytes as usize, 0);
    file.read_exact(compressed)
        .map_err(|error| ChunkError::new("io", format!("read: {error}")))?;

    let codec = flags & 0x3F;
    let is_bcpack = flags & 0x1C != 0;
    if is_bcpack && !matches!(codec, 0x04 | 0x0C | 0x10 | 0x14 | 0x18 | 0x1C) {
        return Err(ChunkError::new(
            "unsupported_codec",
            format!("BCPack codec {codec:#04x} is not one of SixNG's six BC formats"),
        ));
    }
    if !is_bcpack && codec != 0 && codec != 0x20 {
        return Err(ChunkError::new(
            "unsupported_codec",
            format!("non-BCPack flag byte {flags:#04x} is not stored or zlib"),
        ));
    }

    let span = span as usize;
    let (bytes, unfitted, invalid) = if !is_bcpack && codec == 0 {
        if compressed.len() != span {
            return Err(length_error("stored", compressed.len(), span));
        }
        (std::mem::take(compressed), 0, 0)
    } else if !is_bcpack {
        let mut raw = Vec::with_capacity(span);
        ZlibDecoder::new(&compressed[..])
            .read_to_end(&mut raw)
            .map_err(|error| ChunkError::new("zlib", error.to_string()))?;
        if raw.len() != span {
            return Err(length_error("zlib", raw.len(), span));
        }
        (raw, 0, 0)
    } else {
        match codec {
            0x04 => {
                if !span.is_multiple_of(8) {
                    return Err(block_error("BC1", span, 8));
                }
                let tables = bc1_tables();
                let output = bcpack::bc1::decode(compressed, span / 8, &tables.0, &tables.1)
                    .map_err(|error| ChunkError::new("bc1", error.to_string()))?;
                (output.blocks, output.unfitted_blocks, 0)
            }
            0x0C => {
                if !span.is_multiple_of(16) {
                    return Err(block_error("BC3", span, 16));
                }
                let output = sixng_core::bc3pack::decode(compressed, span / 16)
                    .ok_or_else(|| ChunkError::new("bc3", "BCPack BC3 decode failed".to_owned()))?;
                (output.blocks, output.unfitted_blocks, 0)
            }
            0x10 => {
                if !span.is_multiple_of(8) {
                    return Err(block_error("BC4", span, 8));
                }
                let output = sixng_core::bc45pack::decode_bc4(compressed, span / 8)
                    .ok_or_else(|| ChunkError::new("bc4", "BCPack BC4 decode failed".to_owned()))?;
                (output.blocks, output.unfitted_blocks, 0)
            }
            0x14 => {
                if !span.is_multiple_of(16) {
                    return Err(block_error("BC5", span, 16));
                }
                let output = sixng_core::bc45pack::decode_bc5(compressed, span / 16)
                    .ok_or_else(|| ChunkError::new("bc5", "BCPack BC5 decode failed".to_owned()))?;
                (output.blocks, output.unfitted_blocks, 0)
            }
            0x18 => {
                if !span.is_multiple_of(16) {
                    return Err(block_error("BC6H", span, 16));
                }
                let output =
                    sixng_core::bc6pack::decode_bc6h(compressed, span / 16).ok_or_else(|| {
                        ChunkError::new("bc6h", "BCPack BC6H decode failed".to_owned())
                    })?;
                (output.blocks, output.unfitted_blocks, 0)
            }
            0x1C => {
                if !span.is_multiple_of(16) {
                    return Err(block_error("BC7", span, 16));
                }
                let output = sixng_core::bc7pack::decode(compressed, span / 16)
                    .ok_or_else(|| ChunkError::new("bc7", "BCPack BC7 decode failed".to_owned()))?;
                (output.blocks, output.unfitted_blocks, output.invalid_blocks)
            }
            _ => unreachable!("unsupported BCPack codec was rejected above"),
        }
    };
    if bytes.len() != span {
        return Err(length_error("decoded", bytes.len(), span));
    }
    Ok(DecodedChunk {
        bytes,
        unfitted,
        invalid,
    })
}

fn bc1_tables() -> &'static (bcpack::bc1::Window, bcpack::bc1::QuadTable) {
    static TABLES: OnceLock<(bcpack::bc1::Window, bcpack::bc1::QuadTable)> = OnceLock::new();
    TABLES.get_or_init(|| {
        (
            bcpack::bc1::Window::default_table(),
            bcpack::bc1::QuadTable::default_table(),
        )
    })
}

fn block_error(name: &str, span: usize, block_bytes: usize) -> ChunkError {
    ChunkError::new(
        "block_alignment",
        format!("{name} span {span} is not a multiple of {block_bytes}"),
    )
}

fn length_error(source: &str, actual: usize, expected: usize) -> ChunkError {
    ChunkError::new(
        "length_mismatch",
        format!("{source} produced {actual} bytes; expected {expected}"),
    )
}

fn codec_name(codec: u8, flags: u8) -> &'static str {
    if flags & 0x1C == 0 {
        if codec == 0 {
            "stored"
        } else {
            "zlib"
        }
    } else {
        match codec {
            0x04 => "BC1",
            0x0C => "BC3",
            0x10 => "BC4",
            0x14 => "BC5",
            0x18 => "BC6H",
            0x1C => "BC7",
            _ => "unsupported",
        }
    }
}

#[derive(Debug)]
struct DecodedChunk {
    bytes: Vec<u8>,
    unfitted: usize,
    invalid: usize,
}

#[derive(Debug)]
struct ChunkError {
    kind: &'static str,
    message: String,
}

impl ChunkError {
    fn new(kind: &'static str, message: String) -> Self {
        Self { kind, message }
    }
}

#[allow(clippy::too_many_arguments)]
fn failed_summary(
    manifest_index: usize,
    relative: &str,
    path: &Path,
    file_bytes: u64,
    mtime_unix_nanos: u64,
    table_sha256: Option<String>,
    table_entries: Option<u32>,
    started: Instant,
    error: String,
) -> StoreSummary {
    let mut failure_kinds = BTreeMap::new();
    failure_kinds.insert("store".to_owned(), 1);
    StoreSummary {
        manifest_index,
        store: relative.to_owned(),
        path: path.display().to_string(),
        file_bytes,
        mtime_unix_nanos,
        table_sha256,
        table_entries,
        sentinel_index: 0,
        sentinel_source: "none".to_owned(),
        first_virtual_offset: 0,
        end_virtual_offset: 0,
        chunks_expected: 0,
        chunks_attempted: 0,
        chunks_pass: 0,
        chunks_fail: 0,
        successful_bytes: 0,
        aggregate_sha256: sha256_hex(&[]),
        failure_indices: Vec::new(),
        failure_kinds,
        codec_counts: BTreeMap::new(),
        unfitted_blocks: 0,
        invalid_blocks: 0,
        cache_budget_bytes: 0,
        source_changed: false,
        status: "failed".to_owned(),
        error: Some(error),
        elapsed_seconds: started.elapsed().as_secs_f64(),
    }
}

struct StoreWriter {
    writer: BufWriter<File>,
    part: PathBuf,
    final_path: PathBuf,
    last_flush: Instant,
}

impl StoreWriter {
    fn new(final_path: PathBuf) -> io::Result<Self> {
        let part = final_path.with_extension("jsonl.part");
        if part.exists() {
            fs::remove_file(&part)?;
        }
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        Ok(Self {
            writer: BufWriter::with_capacity(1024 * 1024, output),
            part,
            final_path,
            last_flush: Instant::now(),
        })
    }

    fn line(&mut self, value: &Value) -> io::Result<()> {
        write_line(&mut self.writer, value)
    }

    fn maybe_flush(&mut self, force: bool) -> io::Result<()> {
        if force || self.last_flush.elapsed() >= PROGRESS_INTERVAL {
            self.writer.flush()?;
            self.last_flush = Instant::now();
        }
        Ok(())
    }

    fn finish_with(mut self, summary: &StoreSummary) -> io::Result<()> {
        let mut done = serde_json::to_value(summary)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let object = done
            .as_object_mut()
            .expect("store summary serializes as an object");
        object.insert("kind".to_owned(), json!("store_done"));
        self.line(&done)?;
        self.maybe_flush(true)?;
        self.writer.get_ref().sync_all()?;
        drop(self.writer);
        fs::rename(&self.part, &self.final_path)?;
        Ok(())
    }
}

fn table_digest(path: &Path) -> io::Result<(String, u32, u64)> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header)?;
    if &header[..4] != b"RDS5" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an RDS5 store",
        ));
    }
    let entries = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if entries == 0 || entries > 4_000_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk-table entry count out of range",
        ));
    }
    let mut table = vec![0u8; entries as usize * 8];
    file.read_exact(&mut table)?;
    let mut hasher = Sha256::new();
    hasher.update(header);
    hasher.update(&table);
    Ok((
        format!("{:x}", hasher.finalize()),
        entries,
        (24 + entries as usize * 8) as u64,
    ))
}

fn source_stamp(path: &Path) -> io::Result<(u64, u64)> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0);
    Ok((metadata.len(), modified))
}

fn source_bundle_sha256() -> String {
    let mut hasher = Sha256::new();
    hasher.update(include_bytes!("main.rs"));
    hasher.update(include_bytes!("../Cargo.toml"));
    hasher.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", hasher.finalize())
}

fn exe_sha256() -> String {
    match env::current_exe().and_then(fs::read) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(_) => "unavailable".to_owned(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn write_line(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(b"\n")
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&temporary, path)
}

fn input_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

#[derive(Serialize)]
struct RunSummary {
    schema: &'static str,
    sixng_rev: &'static str,
    bcpack_rev: &'static str,
    harness_source_sha256: String,
    cargo_lock_sha256: String,
    exe_sha256: String,
    manifest: String,
    manifest_sha256: String,
    command_line: String,
    out_dir: String,
    started_unix: u64,
    finished_unix: u64,
    elapsed_seconds: f64,
    cache_budget_bytes: u64,
    digest_contract: &'static str,
    source_identity_note: &'static str,
    stores_total: u64,
    stores_done: u64,
    stores_failed: u64,
    chunks_expected: u64,
    chunks_attempted: u64,
    chunks_pass: u64,
    chunks_fail: u64,
    successful_bytes: u64,
    failure_count: u64,
    stores: Vec<StoreSummary>,
}

struct Progress {
    started: Instant,
    last: Instant,
    stores_total: usize,
    stores_done: usize,
    stores_failed: usize,
    chunks_pass: u64,
    chunks_fail: u64,
    successful_bytes: u64,
}

impl Progress {
    fn new(stores_total: usize) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            stores_total,
            stores_done: 0,
            stores_failed: 0,
            chunks_pass: 0,
            chunks_fail: 0,
            successful_bytes: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tick(
        &mut self,
        force: bool,
        store: &str,
        chunk: u64,
        chunk_total: u64,
        pass: u64,
        fail: u64,
        bytes: u64,
    ) {
        let now = Instant::now();
        if !force && now.duration_since(self.last) < PROGRESS_INTERVAL {
            return;
        }
        self.last = now;
        let elapsed = self.started.elapsed().as_secs_f64();
        let total_pass = self.chunks_pass + pass;
        let total_fail = self.chunks_fail + fail;
        let total_bytes = self.successful_bytes + bytes;
        let mib_per_second = if elapsed > 0.0 {
            total_bytes as f64 / 1024.0 / 1024.0 / elapsed
        } else {
            0.0
        };
        eprintln!(
            "progress stores {}/{} (done {}, failed {}), chunks pass {} fail {}, {:.3} GiB, {:.1} MiB/s, current {} {}/{}, {:.1}s",
            self.stores_done,
            self.stores_total,
            self.stores_done,
            self.stores_failed,
            total_pass,
            total_fail,
            total_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            mib_per_second,
            store,
            chunk,
            chunk_total,
            elapsed
        );
    }

    fn store_finished(&mut self, summary: &StoreSummary) {
        self.stores_done += 1;
        if summary.status == "failed" {
            self.stores_failed += 1;
        }
        self.chunks_pass += summary.chunks_pass;
        self.chunks_fail += summary.chunks_fail;
        self.successful_bytes += summary.successful_bytes;
    }

    fn finish(&self, run: &RunSummary) {
        eprintln!(
            "done: stores {}/{}, chunks pass {}/{} fail {}, successful {:.3} GiB, {:.1}s",
            run.stores_done,
            run.stores_total,
            run.chunks_pass,
            run.chunks_expected,
            run.chunks_fail,
            run.successful_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            run.elapsed_seconds
        );
    }
}
