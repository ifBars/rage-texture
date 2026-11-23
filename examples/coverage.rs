//! Read-only layout coverage inventory for a directory of RDS5 stores.
//!
//! Usage:
//!   cargo run --release --example coverage -- --stores DIR [--cases]
//!       [--limit STORES] [--all-files]
//!
//! The scan opens every discovered store with a zero decoded-byte budget,
//! reads the embedded catalog, and validates each descriptor with
//! `Store::texture_layout`. It never reads or decodes texture payloads, and
//! stdout contains only aggregate metadata plus optional per-case descriptor
//! metadata (store-relative path and texture index). Progress goes to stderr.
use rage_texture::{DecodePolicy, Store, preview};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Shape {
    dxgi: u32,
    width: u32,
    height: u32,
    depth: u32,
    mips: u32,
    dimension: u32,
    mode: u32,
    non_tail: u32,
}

impl Shape {
    fn of(t: &rage_texture::catalog::Texture) -> Self {
        Self {
            dxgi: t.dxgi,
            width: t.width,
            height: t.height,
            depth: t.depth,
            mips: t.mips,
            dimension: t.dimension,
            mode: t.mode,
            non_tail: t.non_tail,
        }
    }
    fn json(&self) -> Value {
        json!({
            "dxgi": self.dxgi,
            "width": self.width,
            "height": self.height,
            "depth": self.depth,
            "mips": self.mips,
            "dimension": self.dimension,
            "mode": self.mode,
            "non_tail": self.non_tail,
        })
    }
    fn family(&self) -> &'static str {
        match (self.dimension, self.depth) {
            (1, 1) => "texture2d",
            (1, _) => "array2d",
            (2, _) => "cubemap",
            (3, _) => "volume3d",
            _ => "unknown",
        }
    }
}

#[derive(Default)]
struct FormatCounts {
    textures: u64,
    layout_ok: u64,
    layout_failed: u64,
    preview_supported: u64,
    preview_unsupported: u64,
    reasons: BTreeMap<String, u64>,
}

#[derive(Default)]
struct GroupCounts {
    textures: u64,
    layout_ok: u64,
    layout_failed: u64,
}

#[derive(Default)]
struct Totals {
    stores_total: u64,
    stores_opened: u64,
    store_failures: Vec<Value>,
    textures: u64,
    layout_ok: u64,
    layout_failed: u64,
    preview_supported: u64,
    preview_unsupported: u64,
    reasons: BTreeMap<String, u64>,
    reason_kinds: BTreeMap<String, u64>,
    shapes: BTreeMap<Shape, u64>,
    families: BTreeMap<String, GroupCounts>,
    groups: BTreeMap<String, GroupCounts>,
    formats: BTreeMap<u32, FormatCounts>,
    cases: Vec<Value>,
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

fn candidate(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        e.eq_ignore_ascii_case("ttpf")
            || e.eq_ignore_ascii_case("rdds")
            || e.eq_ignore_ascii_case("rds5")
    })
}

fn collect(path: &Path, all_files: bool) -> io::Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut entries: Vec<_> = fs::read_dir(&dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            let child = entry.path();
            if kind.is_dir() {
                pending.push(child);
            } else if kind.is_file() && (all_files || candidate(&child)) {
                files.push(child);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn group_name(root: &Path, store: &Path) -> String {
    let relative = store.strip_prefix(root).unwrap_or(store);
    let parts: Vec<_> = relative
        .components()
        .filter_map(|part| part.as_os_str().to_str())
        .collect();
    if let Some(at) = parts.iter().position(|part| *part == "ttdb")
        && let Some(group) = parts.get(at + 1)
    {
        return (*group).to_owned();
    }
    store
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("root")
        .to_owned()
}

fn record_error(totals: &mut Totals, kind: &str, reason: &str) {
    *totals.reasons.entry(reason.to_owned()).or_default() += 1;
    *totals
        .reason_kinds
        .entry(format!("{kind}: {reason}"))
        .or_default() += 1;
}

fn counts_json(counts: &GroupCounts) -> Value {
    json!({
        "textures": counts.textures,
        "layout_ok": counts.layout_ok,
        "layout_failed": counts.layout_failed,
    })
}

fn scan(root: &Path, stores: &[PathBuf], cases: bool, totals: &mut Totals) {
    let started = Instant::now();
    for (at, path) in stores.iter().enumerate() {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let group = group_name(root, path);
        let mut store = match Store::open(path, 0, DecodePolicy::LosslessOnly) {
            Ok(store) => store,
            Err(error) => {
                eprintln!(
                    "coverage [{}/{}] {relative}: open failed: {error}",
                    at + 1,
                    stores.len()
                );
                totals.store_failures.push(json!({
                    "store": relative,
                    "error": error.to_string(),
                    "error_kind": format!("{:?}", error.kind()),
                }));
                continue;
            }
        };
        totals.stores_opened += 1;
        let mut group_totals = GroupCounts::default();
        let textures = match store.textures() {
            Ok(textures) => textures,
            Err(error) => {
                eprintln!(
                    "coverage [{}/{}] {relative}: catalog failed: {error}",
                    at + 1,
                    stores.len()
                );
                totals.store_failures.push(json!({
                    "store": relative,
                    "error": error.to_string(),
                    "error_kind": format!("{:?}", error.kind()),
                }));
                continue;
            }
        };
        let mut ok = 0u64;
        let mut failed = 0u64;
        for texture in &textures {
            let shape = Shape::of(texture);
            totals.textures += 1;
            *totals.shapes.entry(shape.clone()).or_default() += 1;
            let preview = preview::supports(texture.dxgi);
            {
                let format = totals.formats.entry(texture.dxgi).or_default();
                format.textures += 1;
                if preview {
                    totals.preview_supported += 1;
                    format.preview_supported += 1;
                } else {
                    totals.preview_unsupported += 1;
                    format.preview_unsupported += 1;
                }
            }
            group_totals.textures += 1;
            match store.texture_layout(texture) {
                Ok(_) => {
                    totals.layout_ok += 1;
                    totals.formats.entry(texture.dxgi).or_default().layout_ok += 1;
                    group_totals.layout_ok += 1;
                    let family = totals
                        .families
                        .entry(shape.family().to_owned())
                        .or_default();
                    family.textures += 1;
                    family.layout_ok += 1;
                    ok += 1;
                }
                Err(error) => {
                    totals.layout_failed += 1;
                    group_totals.layout_failed += 1;
                    let family = totals
                        .families
                        .entry(shape.family().to_owned())
                        .or_default();
                    family.textures += 1;
                    family.layout_failed += 1;
                    let kind = format!("{:?}", error.kind());
                    let reason = error.to_string();
                    record_error(totals, &kind, &reason);
                    {
                        let format = totals.formats.entry(texture.dxgi).or_default();
                        format.layout_failed += 1;
                        *format.reasons.entry(reason.clone()).or_default() += 1;
                    }
                    failed += 1;
                    if cases {
                        totals.cases.push(json!({
                            "store": relative,
                            "index": texture.index,
                            "shape": shape.json(),
                            "error": reason,
                            "error_kind": kind,
                        }));
                    }
                }
            }
        }
        let totals_group = totals.groups.entry(group).or_default();
        totals_group.textures += group_totals.textures;
        totals_group.layout_ok += group_totals.layout_ok;
        totals_group.layout_failed += group_totals.layout_failed;
        eprintln!(
            "coverage [{}/{}] {relative}: textures={} ok={ok} failed={failed} elapsed={:.3}s",
            at + 1,
            stores.len(),
            textures.len(),
            started.elapsed().as_secs_f64()
        );
    }
    totals.stores_total = stores.len() as u64;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stores_dir = None;
    let mut cases = false;
    let mut all_files = false;
    let mut limit = usize::MAX;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stores" => {
                stores_dir = Some(PathBuf::from(
                    args.next().ok_or("--stores needs a directory")?,
                ));
            }
            "--cases" => cases = true,
            "--all-files" => all_files = true,
            "--limit" => {
                limit = args
                    .next()
                    .ok_or("--limit needs a count")?
                    .parse()
                    .map_err(|_| "--limit must be a positive integer")?;
                if limit == 0 {
                    return Err("--limit must be a positive integer".into());
                }
            }
            "--help" | "-h" => {
                println!("usage: coverage --stores DIR [--cases] [--limit STORES] [--all-files]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    let root = stores_dir
        .ok_or("usage: coverage --stores DIR [--cases] [--limit STORES] [--all-files]")?;
    let mut paths = collect(&root, all_files)?;
    paths.truncate(limit);
    if paths.is_empty() {
        return Err("no candidate stores found".into());
    }
    let mut totals = Totals::default();
    scan(&root, &paths, cases, &mut totals);
    let formats: Vec<_> = totals
        .formats
        .iter()
        .map(|(&dxgi, counts)| {
            let reasons: Vec<_> = counts
                .reasons
                .iter()
                .map(|(reason, count)| json!({"reason": reason, "count": count}))
                .collect();
            json!({
                "dxgi": dxgi,
                "name": format_name(dxgi),
                "textures": counts.textures,
                "layout_ok": counts.layout_ok,
                "layout_failed": counts.layout_failed,
                "preview_supported": counts.preview_supported,
                "preview_unsupported": counts.preview_unsupported,
                "unsupported_reasons": reasons,
            })
        })
        .collect();
    let reasons: Vec<_> = totals
        .reasons
        .iter()
        .map(|(reason, count)| json!({"reason": reason, "count": count}))
        .collect();
    let reason_kinds: Vec<_> = totals
        .reason_kinds
        .iter()
        .map(|(kind, count)| json!({"kind": kind, "count": count}))
        .collect();
    let shapes: Vec<_> = totals
        .shapes
        .iter()
        .map(|(shape, count)| json!({"shape": shape.json(), "count": count}))
        .collect();
    let groups: Vec<_> = totals
        .groups
        .iter()
        .map(|(group, counts)| {
            let mut value = counts_json(counts);
            value["group"] = json!(group);
            value
        })
        .collect();
    let families: Vec<_> = totals
        .families
        .iter()
        .map(|(family, counts)| {
            let mut value = counts_json(counts);
            value["family"] = json!(family);
            value
        })
        .collect();
    let mut report = Map::new();
    report.insert("generator".into(), json!("rage-texture-coverage"));
    report.insert("version".into(), json!(1));
    report.insert("stores_root".into(), json!(root.to_string_lossy()));
    report.insert("decoded_budget_bytes".into(), json!(0));
    report.insert("stores_total".into(), json!(totals.stores_total));
    report.insert("stores_opened".into(), json!(totals.stores_opened));
    report.insert("store_failures".into(), json!(totals.store_failures));
    report.insert("textures".into(), json!(totals.textures));
    report.insert("layout_ok".into(), json!(totals.layout_ok));
    report.insert("layout_failed".into(), json!(totals.layout_failed));
    report.insert("preview_supported".into(), json!(totals.preview_supported));
    report.insert(
        "preview_unsupported".into(),
        json!(totals.preview_unsupported),
    );
    report.insert("unsupported_reasons".into(), json!(reasons));
    report.insert("unsupported_reason_kinds".into(), json!(reason_kinds));
    report.insert("unsupported_shapes".into(), json!(shapes));
    report.insert("shape_families".into(), json!(families));
    report.insert("groups".into(), json!(groups));
    report.insert("formats".into(), json!(formats));
    if cases {
        report.insert("cases".into(), json!(totals.cases));
    }
    println!("{}", serde_json::to_string(&Value::Object(report))?);
    Ok(())
}
