//! Full-corpus payload audit for the supplied RDS5 stores.
//!
//! This is a coverage/structure audit, not a pixel-accuracy certificate:
//!
//! * `chunks` phase reads every chunk exactly once with a zero decoded-byte
//!   cache budget, hashes the decoded bytes and records decode provenance.
//! * `dds` phase calls `Store::texture_dds` for every catalog descriptor, hashes
//!   the complete DDS, and validates the container through the independent
//!   `examples/support/dds_consumer.rs` parser (`ddsfile`).
//!
//! A DDS pass means the container header, dimensions, mip/array/cube/volume
//! shape and payload length agree with the catalog shape. It does not certify
//! pixels. The consumer is deliberately a separate parser from the SDK writer.
//!
//! Output is streamed JSONL: one run header, per-store start/completion lines,
//! one line per chunk/texture (every descriptor gets an explicit status), one
//! ledger line per codec/format/layout, and optional representative DDS
//! samples with a manifest. Nothing is retained across descriptors except
//! counters and one output buffer.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example audit_corpus -- \
//!   --stores <owned-root> \
//!   --out D:/audit/corpus.jsonl \
//!   --phase both \
//!   [--store-filter NAME] [--samples D:/audit/samples] \
//!   [--policy fitted|lossless|confirmed|provisional]
//! ```
//!
//! The output path and sample directory must be outside the read-only stores
//! root. `--out` and the sample manifest use exclusive create; choose fresh
//! paths. The default policy is `fitted` (`DecodePolicy::AllowFittedBc`), which
//! allows every fitted BC path. Every store is opened with cache budget 0.
use rage_texture::{Chunk, DecodePolicy, Quality, Store, catalog::Texture, layout::TextureShape};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    fs::OpenOptions,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[path = "support/dds_consumer.rs"]
mod dds_consumer;

const SAMPLE_LIMIT: usize = 256;
const SAMPLE_BYTES_LIMIT: u64 = 256 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);
const SCHEMA: &str = "rage-texture-corpus-audit-v1";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Chunks,
    Dds,
    Both,
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Chunks => "chunks",
            Self::Dds => "dds",
            Self::Both => "both",
        }
    }
    fn chunks(self) -> bool {
        matches!(self, Self::Chunks | Self::Both)
    }
    fn dds(self) -> bool {
        matches!(self, Self::Dds | Self::Both)
    }
}

struct Config {
    stores: PathBuf,
    out: PathBuf,
    phase: Phase,
    store_filter: Option<String>,
    samples: Option<PathBuf>,
    policy: DecodePolicy,
    policy_name: &'static str,
}

impl Config {
    fn parse() -> Result<Self, String> {
        let mut stores = None;
        let mut out = None;
        let mut phase = Phase::Both;
        let mut store_filter = None;
        let mut samples = None;
        let mut policy = DecodePolicy::AllowFittedBc;
        let mut policy_name = "fitted";
        let mut args = env::args().skip(1);
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--stores" => {
                    stores = Some(PathBuf::from(
                        args.next().ok_or("--stores requires a path")?,
                    ));
                }
                "--out" => {
                    out = Some(PathBuf::from(args.next().ok_or("--out requires a path")?));
                }
                "--phase" => {
                    phase = match args.next().ok_or("--phase requires a value")?.as_str() {
                        "chunks" => Phase::Chunks,
                        "dds" => Phase::Dds,
                        "both" => Phase::Both,
                        other => {
                            return Err(format!(
                                "unknown --phase {other:?}; use chunks, dds or both"
                            ));
                        }
                    };
                }
                "--store-filter" => {
                    store_filter = Some(args.next().ok_or("--store-filter requires a value")?);
                }
                "--samples" => {
                    samples = Some(PathBuf::from(
                        args.next().ok_or("--samples requires a path")?,
                    ));
                }
                "--policy" => {
                    let value = args.next().ok_or("--policy requires a value")?;
                    (policy, policy_name) = parse_policy(&value)?;
                }
                "--allow-fitted" => {
                    policy = DecodePolicy::AllowFittedBc;
                    policy_name = "fitted";
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    return Err(format!("unknown argument {other:?}; try --help"));
                }
            }
        }
        let stores = stores.ok_or("--stores is required")?;
        let out = out.ok_or("--out is required")?;
        let stores_abs = std::path::absolute(&stores).map_err(|e| e.to_string())?;
        let out_abs = std::path::absolute(&out).map_err(|e| e.to_string())?;
        if out_abs.starts_with(&stores_abs) {
            return Err("--out must be outside the read-only stores root".to_owned());
        }
        if let Some(samples) = &samples {
            let samples_abs = std::path::absolute(samples).map_err(|e| e.to_string())?;
            if samples_abs.starts_with(&stores_abs) {
                return Err("--samples must be outside the read-only stores root".to_owned());
            }
        }
        Ok(Self {
            stores,
            out,
            phase,
            store_filter,
            samples,
            policy,
            policy_name,
        })
    }
}

fn parse_policy(value: &str) -> Result<(DecodePolicy, &'static str), String> {
    match value {
        "fitted" | "allow-fitted" => Ok((DecodePolicy::AllowFittedBc, "fitted")),
        "lossless" => Ok((DecodePolicy::LosslessOnly, "lossless")),
        "confirmed" => Ok((DecodePolicy::ConfirmedOnly, "confirmed")),
        "provisional" => Ok((DecodePolicy::AllowProvisional, "provisional")),
        other => Err(format!(
            "unknown --policy {other:?}; use fitted, lossless, confirmed or provisional"
        )),
    }
}

fn print_help() {
    println!(
        "Full-corpus payload audit (streaming JSONL).\n\
         \n\
         Usage: audit_corpus --stores ROOT --out JSONL [options]\n\
         \n\
         --stores PATH       store root (or one store file), read-only\n\
         --out PATH          JSONL output; must not already exist\n\
         --phase PHASE       chunks | dds | both (default both)\n\
         --store-filter NAME case-insensitive relative-path substring\n\
         --samples DIR       write representative DDS samples + manifest\n\
         --policy POLICY     fitted (default) | lossless | confirmed | provisional\n\
         --allow-fitted      alias for --policy fitted\n\
         \n\
         Every store opens with cache budget 0. Chunks are read once in the\n\
         chunks phase; every texture is exported and consumer-validated in the\n\
         dds phase. Output and samples must be outside --stores."
    );
}

fn main() {
    match Config::parse() {
        Ok(config) => {
            if let Err(error) = run(config) {
                eprintln!("audit failed: {error}");
                std::process::exit(1);
            }
        }
        Err(message) => {
            eprintln!("audit: {message}");
            std::process::exit(2);
        }
    }
}

fn run(config: Config) -> io::Result<()> {
    let stores = collect(&config.stores, config.store_filter.as_deref())?;
    if stores.is_empty() {
        return Err(input("no stores matched --stores/--store-filter"));
    }
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config.out)
        .map_err(|error| {
            input(format!(
                "cannot create {}: {error}; choose a fresh external path",
                config.out.display()
            ))
        })?;
    let mut writer = BufWriter::new(output);
    let mut samples = match &config.samples {
        Some(dir) => Some(Samples::new(dir)?),
        None => None,
    };
    let mut run = RunState::new(&config, stores.len());
    write_line(&mut writer, &run.header(&config))?;
    writer.flush()?;

    for (at, path) in stores.iter().enumerate() {
        let relative = store_relative(&config.stores, path);
        eprintln!("audit [{}/{}] {relative}: opening", at + 1, stores.len());
        process_store(
            &config,
            path,
            &relative,
            &mut writer,
            &mut samples,
            &mut run,
        )?;
        run.stores_done += 1;
        run.progress(true);
        writer.flush()?;
    }

    run.finish(&config, &mut writer, &mut samples)?;
    writer.flush()?;
    run.progress(true);
    Ok(())
}

fn process_store(
    config: &Config,
    path: &Path,
    relative: &str,
    writer: &mut BufWriter<fs::File>,
    samples: &mut Option<Samples>,
    run: &mut RunState,
) -> io::Result<()> {
    let started = Instant::now();
    let (bytes, mtime, stamp) = match source_stamp(path) {
        Ok(stamp) => stamp,
        Err(error) => {
            write_line(
                writer,
                &json!({
                    "kind": "store_failed",
                    "store": relative,
                    "path": path.display().to_string(),
                    "stage": "metadata",
                    "error_kind": format!("{:?}", error.kind()),
                    "error": error.to_string(),
                }),
            )?;
            run.stores_failed += 1;
            return Ok(());
        }
    };
    let mut store = match Store::open(path, 0, config.policy) {
        Ok(store) => store,
        Err(error) => {
            write_line(
                writer,
                &json!({
                    "kind": "store_failed",
                    "store": relative,
                    "path": path.display().to_string(),
                    "stage": "open",
                    "bytes": bytes,
                    "mtime_unix": mtime,
                    "source_stamp": stamp,
                    "error_kind": format!("{:?}", error.kind()),
                    "error": error.to_string(),
                }),
            )?;
            run.stores_failed += 1;
            return Ok(());
        }
    };
    run.stores_opened += 1;
    let textures = match store.textures() {
        Ok(textures) => textures,
        Err(error) => {
            write_line(
                writer,
                &json!({
                    "kind": "store_failed",
                    "store": relative,
                    "path": path.display().to_string(),
                    "stage": "catalog",
                    "bytes": bytes,
                    "mtime_unix": mtime,
                    "source_stamp": stamp,
                    "error_kind": format!("{:?}", error.kind()),
                    "error": error.to_string(),
                }),
            )?;
            run.stores_failed += 1;
            return Ok(());
        }
    };
    let chunks = store.chunks().to_vec();
    let history_digest = store.history_digest().map(|digest| hex(&digest));
    let before = run.counters;
    let mut chunk_aggregate = ChunkAggregate::default();
    let mut dds_aggregate = DdsAggregate::default();

    for chunk in &chunks {
        run.expect_chunk(chunk.codec);
    }
    for texture in &textures {
        let shape = texture.shape();
        run.expect_texture(
            format_codec(texture.dxgi),
            format_name(texture.dxgi),
            family(&shape),
        );
    }

    write_line(
        writer,
        &json!({
            "kind": "store_start",
            "store": relative,
            "path": path.display().to_string(),
            "bytes": bytes,
            "mtime_unix": mtime,
            "source_stamp": stamp,
            "virtual_size": store.virtual_size(),
            "chunks": chunks.len(),
            "textures": textures.len(),
            "history_digest": history_digest,
            "phase": config.phase.name(),
        }),
    )?;
    writer.flush()?;

    if config.phase.chunks() {
        for (index, chunk) in chunks.iter().enumerate() {
            process_chunk(
                &mut store,
                chunk,
                index,
                relative,
                writer,
                run,
                &mut chunk_aggregate,
            )?;
        }
    }
    if config.phase.dds() {
        for texture in &textures {
            process_texture(
                &mut store,
                texture,
                relative,
                writer,
                samples,
                run,
                &mut dds_aggregate,
            )?;
        }
    }

    let (_, _, end_stamp) = source_stamp(path)?;
    let source_changed = end_stamp != stamp;
    if source_changed {
        run.source_changed += 1;
    }
    let stats = store.stats();
    let cache_policy_ok = stats.cache_hits == 0 && stats.cache_bytes == 0;
    if !cache_policy_ok {
        run.cache_policy_violations += 1;
    }
    let after = run.counters;
    let delta = counters_delta(after, before);
    let status = if source_changed {
        "source_changed"
    } else if !cache_policy_ok {
        "cache_policy_violation"
    } else if delta.chunks_fail + delta.textures_fail + delta.consumer_failures > 0 {
        "completed_with_failures"
    } else {
        "pass"
    };
    let chunk_aggregate_json = if config.phase.chunks() {
        let ChunkAggregate { sha, bytes, failed } = chunk_aggregate;
        json!({
            "sha256": hex(&sha.finalize()),
            "chunks": delta.chunks_pass,
            "bytes": bytes,
            "failed_indices": failed,
        })
    } else {
        Value::Null
    };
    let dds_aggregate_json = if config.phase.dds() {
        let DdsAggregate {
            sha,
            bytes,
            textures,
            failed,
            consumer_failed,
        } = dds_aggregate;
        json!({
            "sha256": hex(&sha.finalize()),
            "textures": textures,
            "bytes": bytes,
            "failed_textures": failed,
            "consumer_failed_textures": consumer_failed,
        })
    } else {
        Value::Null
    };
    write_line(
        writer,
        &json!({
            "kind": "store_done",
            "store": relative,
            "status": status,
            "chunks": {
                "expected": delta.chunks_expected,
                "attempted": delta.chunks_attempted,
                "pass": delta.chunks_pass,
                "fail": delta.chunks_fail,
            },
            "textures": {
                "expected": delta.textures_expected,
                "attempted": delta.textures_attempted,
                "pass": delta.textures_pass,
                "fail": delta.textures_fail,
            },
            "chunk_aggregate": chunk_aggregate_json,
            "dds_aggregate": dds_aggregate_json,
            "fitted": delta.fitted,
            "lossless": delta.lossless,
            "history_confirmed": delta.history_confirmed,
            "consumer_failures": delta.consumer_failures,
            "source_changed": source_changed,
            "source_stamp_end": end_stamp,
            "cache_policy_ok": cache_policy_ok,
            "stats": stats,
            "elapsed_seconds": started.elapsed().as_secs_f64(),
        }),
    )?;
    writer.flush()?;
    eprintln!(
        "audit {relative}: chunks {}/{} pass {}/{}, textures {}/{} pass {}/{}, {:.2}s",
        delta.chunks_attempted,
        delta.chunks_expected,
        delta.chunks_pass,
        delta.chunks_attempted,
        delta.textures_attempted,
        delta.textures_expected,
        delta.textures_pass,
        delta.textures_attempted,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

struct ChunkAggregate {
    sha: Sha256,
    bytes: u64,
    failed: Vec<usize>,
}
impl Default for ChunkAggregate {
    fn default() -> Self {
        Self {
            sha: Sha256::new(),
            bytes: 0,
            failed: Vec::new(),
        }
    }
}

struct DdsAggregate {
    sha: Sha256,
    bytes: u64,
    textures: u64,
    failed: Vec<usize>,
    consumer_failed: Vec<usize>,
}
impl Default for DdsAggregate {
    fn default() -> Self {
        Self {
            sha: Sha256::new(),
            bytes: 0,
            textures: 0,
            failed: Vec::new(),
            consumer_failed: Vec::new(),
        }
    }
}

fn process_chunk(
    store: &mut Store,
    chunk: &Chunk,
    index: usize,
    relative: &str,
    writer: &mut BufWriter<fs::File>,
    run: &mut RunState,
    aggregate: &mut ChunkAggregate,
) -> io::Result<()> {
    run.counters.chunks_attempted += 1;
    let line = match store.read_chunk(index) {
        Ok(decoded) => {
            let (quality, lossless, fitted, history_confirmed) =
                quality_summary(std::slice::from_ref(&decoded.quality));
            aggregate.sha.update(decoded.bytes.as_ref());
            aggregate.bytes += decoded.bytes.len() as u64;
            if fitted {
                run.counters.fitted += 1;
            }
            if lossless {
                run.counters.lossless += 1;
            }
            if history_confirmed > 0 {
                run.counters.history_confirmed += 1;
            }
            run.counters.chunks_pass += 1;
            run.record_chunk(chunk.codec, true);
            json!({
                "kind": "chunk",
                "store": relative,
                "chunk": index,
                "codec": chunk.codec,
                "codec_name": codec_name(chunk.codec),
                "file_offset": chunk.file_offset,
                "virtual_offset": chunk.virtual_offset,
                "packed_bytes": chunk.packed_bytes,
                "decoded_bytes": chunk.decoded_bytes,
                "status": "pass",
                "sha256": sha256_hex(decoded.bytes.as_ref()),
                "quality": quality,
                "fitted": fitted,
                "lossless": lossless,
            })
        }
        Err(error) => {
            aggregate.failed.push(index);
            run.counters.chunks_fail += 1;
            run.record_chunk(chunk.codec, false);
            json!({
                "kind": "chunk",
                "store": relative,
                "chunk": index,
                "codec": chunk.codec,
                "codec_name": codec_name(chunk.codec),
                "file_offset": chunk.file_offset,
                "virtual_offset": chunk.virtual_offset,
                "packed_bytes": chunk.packed_bytes,
                "decoded_bytes": chunk.decoded_bytes,
                "status": "fail",
                "error_kind": format!("{:?}", error.kind()),
                "error": error.to_string(),
            })
        }
    };
    write_line(writer, &line)?;
    run.progress(false);
    Ok(())
}

fn process_texture(
    store: &mut Store,
    texture: &Texture,
    relative: &str,
    writer: &mut BufWriter<fs::File>,
    samples: &mut Option<Samples>,
    run: &mut RunState,
    dds_aggregate: &mut DdsAggregate,
) -> io::Result<()> {
    let shape = texture.shape();
    let layout = family(&shape);
    let format = format_name(texture.dxgi);
    let codec = format_codec(texture.dxgi);
    run.counters.textures_attempted += 1;
    let line = match store.texture_dds(texture) {
        Ok(extracted) => {
            let (quality, lossless, fitted, history_confirmed) =
                quality_summary(&extracted.quality);
            if fitted {
                run.counters.fitted += 1;
            }
            if lossless {
                run.counters.lossless += 1;
            }
            if history_confirmed > 0 {
                run.counters.history_confirmed += 1;
            }
            let hash = sha256_hex(&extracted.dds);
            dds_aggregate.sha.update(&extracted.dds);
            dds_aggregate.bytes += extracted.dds.len() as u64;
            dds_aggregate.textures += 1;
            let consumer = dds_consumer::validate(&extracted.dds, &shape);
            let consumer_ok = consumer.is_ok();
            if !consumer_ok {
                dds_aggregate.consumer_failed.push(texture.index);
            }
            let sample = if consumer_ok {
                samples.as_mut().map(|samples| {
                    samples.maybe_write(relative, texture, &extracted.dds, &shape, &quality)
                })
            } else {
                None
            };
            if consumer_ok {
                run.counters.textures_pass += 1;
                run.record_texture(codec, format, layout, true);
            } else {
                run.counters.textures_fail += 1;
                run.counters.consumer_failures += 1;
                run.record_texture(codec, format, layout, false);
            }
            json!({
                "kind": "texture",
                "store": relative,
                "texture": texture.index,
                "asset_key": texture.asset_key,
                "dxgi": texture.dxgi,
                "format": format,
                "width": texture.width,
                "height": texture.height,
                "depth": texture.depth,
                "mips": texture.mips,
                "dimension": texture.dimension,
                "mode": texture.mode,
                "layout": layout,
                "status": if consumer_ok { "pass" } else { "fail" },
                "bytes": extracted.dds.len(),
                "sha256": hash,
                "quality": quality,
                "fitted": fitted,
                "lossless": lossless,
                "consumer": if consumer_ok { "pass" } else { "fail" },
                "consumer_error": consumer.err().map(|error| error.to_string()),
                "input_bytes": extracted.input_bytes,
                "timings": extracted.timings,
                "sample": sample.map(|outcome| outcome.json()),
            })
        }
        Err(error) => {
            dds_aggregate.failed.push(texture.index);
            run.counters.textures_fail += 1;
            run.record_texture(codec, format, layout, false);
            json!({
                "kind": "texture",
                "store": relative,
                "texture": texture.index,
                "asset_key": texture.asset_key,
                "dxgi": texture.dxgi,
                "format": format,
                "width": texture.width,
                "height": texture.height,
                "depth": texture.depth,
                "mips": texture.mips,
                "dimension": texture.dimension,
                "mode": texture.mode,
                "layout": layout,
                "status": "fail",
                "error_kind": format!("{:?}", error.kind()),
                "error": error.to_string(),
            })
        }
    };
    write_line(writer, &line)?;
    run.progress(false);
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct Counters {
    chunks_expected: u64,
    chunks_attempted: u64,
    chunks_pass: u64,
    chunks_fail: u64,
    textures_expected: u64,
    textures_attempted: u64,
    textures_pass: u64,
    textures_fail: u64,
    fitted: u64,
    lossless: u64,
    history_confirmed: u64,
    consumer_failures: u64,
}

fn counters_delta(after: Counters, before: Counters) -> Counters {
    Counters {
        chunks_expected: after.chunks_expected - before.chunks_expected,
        chunks_attempted: after.chunks_attempted - before.chunks_attempted,
        chunks_pass: after.chunks_pass - before.chunks_pass,
        chunks_fail: after.chunks_fail - before.chunks_fail,
        textures_expected: after.textures_expected - before.textures_expected,
        textures_attempted: after.textures_attempted - before.textures_attempted,
        textures_pass: after.textures_pass - before.textures_pass,
        textures_fail: after.textures_fail - before.textures_fail,
        fitted: after.fitted - before.fitted,
        lossless: after.lossless - before.lossless,
        history_confirmed: after.history_confirmed - before.history_confirmed,
        consumer_failures: after.consumer_failures - before.consumer_failures,
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LedgerKey {
    phase: &'static str,
    codec: u8,
    format: String,
    layout: String,
}

#[derive(Default, Clone, Copy)]
struct LedgerCounts {
    expected: u64,
    attempted: u64,
    pass: u64,
    fail: u64,
}

struct RunState {
    started: Instant,
    last_progress: Instant,
    stores_total: usize,
    stores_done: usize,
    stores_opened: u64,
    stores_failed: u64,
    source_changed: u64,
    cache_policy_violations: u64,
    counters: Counters,
    ledger: BTreeMap<LedgerKey, LedgerCounts>,
}

impl RunState {
    fn new(_config: &Config, stores_total: usize) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_progress: now,
            stores_total,
            stores_done: 0,
            stores_opened: 0,
            stores_failed: 0,
            source_changed: 0,
            cache_policy_violations: 0,
            counters: Counters::default(),
            ledger: BTreeMap::new(),
        }
    }

    fn header(&self, config: &Config) -> Value {
        json!({
            "kind": "run",
            "schema": SCHEMA,
            "stores_root": config.stores.display().to_string(),
            "out": config.out.display().to_string(),
            "phase": config.phase.name(),
            "policy": config.policy_name,
            "cache_budget": 0,
            "store_filter": config.store_filter,
            "samples": config.samples.as_ref().map(|path| path.display().to_string()),
            "sample_limit": SAMPLE_LIMIT,
            "sample_bytes_limit": SAMPLE_BYTES_LIMIT,
            "stores_total": self.stores_total,
            "package": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
            "binary": format!(
                "{}/{} {}/{} debug={}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                cfg!(debug_assertions)
            ),
            "audit_source_sha256": sha256_hex(include_bytes!("audit_corpus.rs")),
            "dds_consumer_source_sha256": sha256_hex(include_bytes!("support/dds_consumer.rs")),
            "aggregate_contract": {
                "chunk_aggregate.sha256": "SHA-256 of successful decoded chunk bytes concatenated in numeric chunk table order, including chunk 0; failed chunks are omitted and their indices are listed in chunk_aggregate.failed_indices",
                "dds_aggregate.sha256": "SHA-256 of successful extracted DDS bytes concatenated in numeric texture index order; extraction failures are omitted and listed in dds_aggregate.failed_textures; consumer validation failures are listed separately in dds_aggregate.consumer_failed_textures and their DDS bytes remain in the aggregate",
                "empty_sha256": sha256_hex(b""),
            },
            "validation": "container-only; consumer parser is independent of the SDK writer",
            "started_unix": now_unix(),
        })
    }

    fn expect_chunk(&mut self, codec: u8) {
        self.counters.chunks_expected += 1;
        self.ledger
            .entry(LedgerKey {
                phase: "chunks",
                codec,
                format: "*".to_owned(),
                layout: "*".to_owned(),
            })
            .or_default()
            .expected += 1;
    }

    fn record_chunk(&mut self, codec: u8, pass: bool) {
        let entry = self
            .ledger
            .entry(LedgerKey {
                phase: "chunks",
                codec,
                format: "*".to_owned(),
                layout: "*".to_owned(),
            })
            .or_default();
        entry.attempted += 1;
        if pass {
            entry.pass += 1;
        } else {
            entry.fail += 1;
        }
    }

    fn expect_texture(&mut self, codec: u8, format: &str, layout: &str) {
        self.counters.textures_expected += 1;
        self.ledger
            .entry(LedgerKey {
                phase: "dds",
                codec,
                format: format.to_owned(),
                layout: layout.to_owned(),
            })
            .or_default()
            .expected += 1;
    }

    fn record_texture(&mut self, codec: u8, format: &str, layout: &str, pass: bool) {
        let entry = self
            .ledger
            .entry(LedgerKey {
                phase: "dds",
                codec,
                format: format.to_owned(),
                layout: layout.to_owned(),
            })
            .or_default();
        entry.attempted += 1;
        if pass {
            entry.pass += 1;
        } else {
            entry.fail += 1;
        }
    }

    fn progress(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now.duration_since(self.last_progress) < PROGRESS_INTERVAL {
            return;
        }
        self.last_progress = now;
        eprintln!(
            "audit: stores {}/{} (opened {}, failed {}), chunks {}/{}, textures {}/{}, pass {}, fail {}, fitted {}, lossless {}, {:.1}s",
            self.stores_done,
            self.stores_total,
            self.stores_opened,
            self.stores_failed,
            self.counters.chunks_attempted,
            self.counters.chunks_expected,
            self.counters.textures_attempted,
            self.counters.textures_expected,
            self.counters.chunks_pass + self.counters.textures_pass,
            self.counters.chunks_fail + self.counters.textures_fail,
            self.counters.fitted,
            self.counters.lossless,
            self.started.elapsed().as_secs_f64()
        );
    }

    fn finish(
        &mut self,
        config: &Config,
        writer: &mut BufWriter<fs::File>,
        samples: &mut Option<Samples>,
    ) -> io::Result<()> {
        for (key, counts) in &self.ledger {
            write_line(
                writer,
                &json!({
                    "kind": "ledger",
                    "phase": key.phase,
                    "codec": key.codec,
                    "codec_name": codec_name(key.codec),
                    "format": key.format,
                    "layout": key.layout,
                    "expected": counts.expected,
                    "attempted": counts.attempted,
                    "pass": counts.pass,
                    "fail": counts.fail,
                    "missing": counts.expected.saturating_sub(counts.attempted),
                }),
            )?;
        }
        for phase in ["chunks", "dds"] {
            let mut expected = 0;
            let mut attempted = 0;
            let mut pass = 0;
            let mut fail = 0;
            for (key, counts) in &self.ledger {
                if key.phase == phase {
                    expected += counts.expected;
                    attempted += counts.attempted;
                    pass += counts.pass;
                    fail += counts.fail;
                }
            }
            if expected > 0 || attempted > 0 {
                write_line(
                    writer,
                    &json!({
                        "kind": "ledger_totals",
                        "phase": phase,
                        "expected": expected,
                        "attempted": attempted,
                        "pass": pass,
                        "fail": fail,
                        "missing": expected.saturating_sub(attempted),
                    }),
                )?;
            }
        }
        if let Some(samples) = samples {
            samples.finish(writer)?;
        }
        write_line(
            writer,
            &json!({
                "kind": "run_done",
                "schema": SCHEMA,
                "phase": config.phase.name(),
                "policy": config.policy_name,
                "stores_total": self.stores_total,
                "stores_opened": self.stores_opened,
                "stores_failed": self.stores_failed,
                "source_changed": self.source_changed,
                "cache_policy_violations": self.cache_policy_violations,
                "chunks": {
                    "expected": self.counters.chunks_expected,
                    "attempted": self.counters.chunks_attempted,
                    "pass": self.counters.chunks_pass,
                    "fail": self.counters.chunks_fail,
                },
                "textures": {
                    "expected": self.counters.textures_expected,
                    "attempted": self.counters.textures_attempted,
                    "pass": self.counters.textures_pass,
                    "fail": self.counters.textures_fail,
                },
                "fitted": self.counters.fitted,
                "lossless": self.counters.lossless,
                "history_confirmed": self.counters.history_confirmed,
                "consumer_failures": self.counters.consumer_failures,
                "elapsed_seconds": self.started.elapsed().as_secs_f64(),
                "audit_source_sha256": sha256_hex(include_bytes!("audit_corpus.rs")),
                "dds_consumer_source_sha256": sha256_hex(include_bytes!("support/dds_consumer.rs")),
            }),
        )?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SampleKey {
    dxgi: u32,
    dimension: u32,
    array: bool,
    mips_gt1: bool,
}

struct Samples {
    dir: PathBuf,
    manifest: BufWriter<fs::File>,
    seen: BTreeSet<SampleKey>,
    count: usize,
    bytes: u64,
}

enum SampleOutcome {
    Written(String),
    Duplicate,
    SkippedCap,
    Error(String),
}
impl SampleOutcome {
    fn json(&self) -> Value {
        match self {
            Self::Written(file) => json!({"status": "written", "file": file}),
            Self::Duplicate => json!({"status": "duplicate"}),
            Self::SkippedCap => json!({"status": "skipped_cap"}),
            Self::Error(error) => json!({"status": "error", "error": error}),
        }
    }
}

impl Samples {
    fn new(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let manifest = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("manifest.jsonl"))
            .map_err(|error| {
                input(format!(
                    "cannot create sample manifest in {}: {error}; choose a fresh sample directory",
                    dir.display()
                ))
            })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest: BufWriter::new(manifest),
            seen: BTreeSet::new(),
            count: 0,
            bytes: 0,
        })
    }

    fn maybe_write(
        &mut self,
        store: &str,
        texture: &Texture,
        dds: &[u8],
        shape: &TextureShape,
        quality: &Value,
    ) -> SampleOutcome {
        let key = SampleKey {
            dxgi: texture.dxgi,
            dimension: texture.dimension,
            array: texture.depth > 1,
            mips_gt1: texture.mips > 1,
        };
        if self.seen.contains(&key) {
            return SampleOutcome::Duplicate;
        }
        if self.count >= SAMPLE_LIMIT
            || self.bytes.saturating_add(dds.len() as u64) > SAMPLE_BYTES_LIMIT
        {
            return SampleOutcome::SkippedCap;
        }
        let file = sample_name(store, texture.index, texture.dxgi);
        let path = self.dir.join(&file);
        let mut output = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(output) => output,
            Err(error) => return SampleOutcome::Error(error.to_string()),
        };
        if let Err(error) = output.write_all(dds) {
            return SampleOutcome::Error(error.to_string());
        }
        let line = json!({
            "kind": "sample",
            "key": {
                "dxgi": texture.dxgi,
                "format": format_name(texture.dxgi),
                "dimension": texture.dimension,
                "array": key.array,
                "mips_gt1": key.mips_gt1,
            },
            "store": store,
            "texture": texture.index,
            "file": file,
            "bytes": dds.len(),
            "sha256": sha256_hex(dds),
            "shape": shape_json(shape),
            "quality": quality,
        });
        if let Err(error) =
            write_line(&mut self.manifest, &line).and_then(|()| self.manifest.flush())
        {
            return SampleOutcome::Error(error.to_string());
        }
        self.seen.insert(key);
        self.count += 1;
        self.bytes += dds.len() as u64;
        SampleOutcome::Written(file)
    }

    fn finish(&mut self, writer: &mut BufWriter<fs::File>) -> io::Result<()> {
        write_line(
            writer,
            &json!({
                "kind": "sample_done",
                "count": self.count,
                "bytes": self.bytes,
                "limit": SAMPLE_LIMIT,
                "bytes_limit": SAMPLE_BYTES_LIMIT,
            }),
        )?;
        self.manifest.flush()
    }
}

fn sample_name(store: &str, texture: usize, dxgi: u32) -> String {
    let mut safe: String = store
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    safe = safe.trim_matches('_').to_owned();
    if safe.is_empty() {
        safe.push_str("store");
    }
    if safe.len() > 96 {
        safe.truncate(96);
    }
    format!("{safe}_{texture:06}_{dxgi}.dds")
}

fn shape_json(shape: &TextureShape) -> Value {
    json!({
        "width": shape.width,
        "height": shape.height,
        "depth": shape.depth,
        "mips": shape.mips,
        "dxgi": shape.dxgi,
        "dimension": shape.dimension,
        "mode": shape.mode,
        "non_tail": shape.non_tail,
    })
}

fn quality_summary(qualities: &[Quality]) -> (Value, bool, bool, u64) {
    let mut lossless = 0u64;
    let mut fitted_bc1 = 0u64;
    let mut fitted_bc: BTreeMap<u8, (u64, u64, u64)> = BTreeMap::new();
    let mut history_bc: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
    let mut history_confirmed = 0u64;
    let mut history_provisional = 0u64;
    let mut unfitted = 0u64;
    let mut invalid = 0u64;
    for quality in qualities {
        match *quality {
            Quality::LosslessContainer => lossless += 1,
            Quality::FittedBc1 { unfitted_blocks } => {
                fitted_bc1 += 1;
                unfitted += unfitted_blocks as u64;
            }
            Quality::FittedBc {
                codec,
                unfitted_blocks,
                invalid_blocks,
            } => {
                let entry = fitted_bc.entry(codec).or_default();
                entry.0 += 1;
                entry.1 += unfitted_blocks as u64;
                entry.2 += invalid_blocks as u64;
                unfitted += unfitted_blocks as u64;
                invalid += invalid_blocks as u64;
            }
            Quality::HistoryBc {
                codec,
                provisional_dependencies,
                ..
            } => {
                let entry = history_bc.entry(codec).or_default();
                entry.0 += 1;
                entry.1 += provisional_dependencies as u64;
                // Fully confirmed history is not fitted: it is a distinct
                // reconstruction category, neither fitted nor lossless.
                if provisional_dependencies == 0 {
                    history_confirmed += 1;
                } else {
                    history_provisional += 1;
                }
            }
        }
    }
    let lossless_only = !qualities.is_empty() && lossless == qualities.len() as u64;
    let fitted = fitted_bc1 > 0 || !fitted_bc.is_empty() || history_provisional > 0;
    let mut fitted_json = Map::new();
    for (codec, (chunks, unfitted, invalid)) in fitted_bc {
        fitted_json.insert(
            codec_name(codec).to_owned(),
            json!({"chunks": chunks, "unfitted_blocks": unfitted, "invalid_blocks": invalid}),
        );
    }
    let mut history_json = Map::new();
    for (codec, (chunks, dependencies)) in history_bc {
        history_json.insert(
            codec_name(codec).to_owned(),
            json!({"chunks": chunks, "provisional_dependencies": dependencies}),
        );
    }
    (
        json!({
            "lossless_container": lossless,
            "fitted_bc1": fitted_bc1,
            "fitted_bc": fitted_json,
            "history_bc": history_json,
            "history_confirmed": history_confirmed,
            "history_provisional": history_provisional,
            "unfitted_blocks": unfitted,
            "invalid_blocks": invalid,
        }),
        lossless_only,
        fitted,
        history_confirmed,
    )
}

fn format_name(dxgi: u32) -> &'static str {
    match dxgi {
        2 => "R32G32B32A32_FLOAT",
        10 => "R16G16B16A16_FLOAT",
        11 => "R16G16B16A16_UNORM",
        13 => "R16G16B16A16_SNORM",
        16 => "R32G32_FLOAT",
        24 => "R10G10B10A2_UNORM",
        26 => "R11G11B10_FLOAT",
        28 => "R8G8B8A8_UNORM",
        29 => "R8G8B8A8_UNORM_SRGB",
        31 => "R8G8B8A8_SNORM",
        34 => "R16G16_FLOAT",
        35 => "R16G16_UNORM",
        37 => "R16G16_SNORM",
        41 => "R32_FLOAT",
        49 => "R8G8_UNORM",
        51 => "R8G8_SNORM",
        54 => "R16_FLOAT",
        56 => "R16_UNORM",
        58 => "R16_SNORM",
        61 => "R8_UNORM",
        63 => "R8_SNORM",
        65 => "A8_UNORM",
        71 => "BC1_UNORM",
        72 => "BC1_UNORM_SRGB",
        74 => "BC2_UNORM",
        75 => "BC2_UNORM_SRGB",
        77 => "BC3_UNORM",
        78 => "BC3_UNORM_SRGB",
        80 => "BC4_UNORM",
        81 => "BC4_SNORM",
        83 => "BC5_UNORM",
        84 => "BC5_SNORM",
        87 => "B8G8R8A8_UNORM",
        88 => "B8G8R8X8_UNORM",
        91 => "B8G8R8A8_UNORM_SRGB",
        93 => "B8G8R8X8_UNORM_SRGB",
        95 => "BC6H_UF16",
        96 => "BC6H_SF16",
        98 => "BC7_UNORM",
        99 => "BC7_UNORM_SRGB",
        _ => "unknown",
    }
}

fn format_codec(dxgi: u32) -> u8 {
    match dxgi {
        71 | 72 => 1,
        74 | 75 => 2,
        77 | 78 => 3,
        80 | 81 => 4,
        83 | 84 => 5,
        95 | 96 => 6,
        98 | 99 => 7,
        _ => 0,
    }
}

fn codec_name(codec: u8) -> &'static str {
    match codec {
        0 => "raw",
        1 => "BC1",
        2 => "BC2",
        3 => "BC3",
        4 => "BC4",
        5 => "BC5",
        6 => "BC6H",
        7 => "BC7",
        8 => "zlib",
        _ => "unknown",
    }
}

fn family(shape: &TextureShape) -> &'static str {
    match (shape.dimension, shape.depth) {
        (1, 1) => "texture2d",
        (1, _) => "array2d",
        (2, _) => "cubemap",
        (3, _) => "volume3d",
        _ => "unknown",
    }
}

fn candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ttpf")
                || extension.eq_ignore_ascii_case("rdds")
                || extension.eq_ignore_ascii_case("rds5")
        })
}

fn collect(root: &Path, filter: Option<&str>) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root.to_path_buf());
    } else {
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let mut entries: Vec<_> = fs::read_dir(&directory)?.collect::<Result<_, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    continue;
                }
                let child = entry.path();
                if kind.is_dir() {
                    pending.push(child);
                } else if kind.is_file() && candidate(&child) {
                    files.push(child);
                }
            }
        }
    }
    files.sort();
    if let Some(filter) = filter {
        let needle = filter.to_ascii_lowercase();
        files.retain(|path| {
            store_relative(root, path)
                .to_ascii_lowercase()
                .contains(&needle)
        });
    }
    Ok(files)
}

fn store_relative(root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

fn source_stamp(path: &Path) -> io::Result<(u64, i64, String)> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let len = metadata.len();
    Ok((len, modified, format!("{len}:{modified}")))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_line(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(b"\n")
}

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    #[test]
    fn confirmed_history_is_distinct_from_fitted_and_container_data() {
        let (summary, lossless, fitted, confirmed) = quality_summary(&[Quality::HistoryBc {
            codec: 6,
            provisional_dependencies: 0,
            history_digest: [1; 32],
        }]);
        assert!(!lossless && !fitted);
        assert_eq!(confirmed, 1);
        assert_eq!(summary["history_confirmed"], 1);
        assert_eq!(summary["history_provisional"], 0);
    }

    #[test]
    fn one_provisional_dependency_keeps_the_output_provisional() {
        let (summary, lossless, fitted, confirmed) = quality_summary(&[
            Quality::LosslessContainer,
            Quality::HistoryBc {
                codec: 7,
                provisional_dependencies: 1,
                history_digest: [2; 32],
            },
        ]);
        assert!(!lossless && fitted);
        assert_eq!(confirmed, 0);
        assert_eq!(summary["history_confirmed"], 0);
        assert_eq!(summary["history_provisional"], 1);
    }
}
