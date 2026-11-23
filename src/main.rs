use rage_texture::{DecodePolicy, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{env, error::Error, fs, hint::black_box, io::Write, time::Instant};

#[derive(Deserialize)]
struct Plan {
    ranges: Vec<(u64, usize)>,
    cache_bytes: usize,
    repeats: usize,
    trials: usize,
    allow_fitted_bc1: bool,
    #[serde(default)]
    batch: bool,
}
#[derive(Serialize)]
struct Run {
    sha256: Vec<String>,
    cold_seconds: Vec<f64>,
    hot_seconds: Vec<f64>,
    stats: rage_texture::Stats,
}
const USAGE: &str = "usage: rage-texture COMMAND ...
  inspect STORE
  list STORE
  export STORE INDEX OUTPUT.dds|OUTPUT.png [--allow-fitted] [--no-cache] [--mip N --slice N | --mips COUNT]
  export-all STORE DIRECTORY [--allow-fitted] [--no-cache]
  read-range STORE OFFSET LENGTH OUTPUT
  range-hash STORE OFFSET LENGTH [--allow-fitted]
  mip-hash STORE INDEX MIP SLICE [--allow-fitted]
  bench-export STORE INDEX TRIALS [--no-cache]
  run STORE PLAN.json
DDS exports include all mips/slices; --mip/--slice select PNG previews only.
History-aware options: --confirmed | --allow-provisional, with optional --history FILE_OR_DIRECTORY.
These options are separate from the legacy --allow-fitted decoder path.";

#[derive(Default)]
struct DecoderOptions {
    policy: Option<DecodePolicy>,
    history: Option<std::sync::Arc<rage_texture::HistoryTables>>,
}
impl DecoderOptions {
    fn parse(
        raw: Vec<std::ffi::OsString>,
    ) -> Result<(Vec<std::ffi::OsString>, Self), Box<dyn Error>> {
        let mut options = Self::default();
        let mut args = Vec::new();
        let mut history_path = None;
        let mut i = 0;
        while i < raw.len() {
            match raw[i].to_str().filter(|_| i >= 3) {
                Some("--confirmed") | Some("--allow-provisional") => {
                    let policy = if raw[i] == "--confirmed" {
                        DecodePolicy::ConfirmedOnly
                    } else {
                        DecodePolicy::AllowProvisional
                    };
                    if options.policy.replace(policy).is_some() {
                        return Err("choose exactly one history decode policy".into());
                    }
                }
                Some("--history") => {
                    i += 1;
                    if history_path.is_some() {
                        return Err("--history supplied more than once".into());
                    }
                    history_path = Some(
                        raw.get(i)
                            .ok_or("--history needs a file or directory")?
                            .clone(),
                    );
                }
                _ => args.push(raw[i].clone()),
            }
            i += 1;
        }
        if (options.policy.is_some() || history_path.is_some())
            && args.iter().any(|a| a == "--allow-fitted")
        {
            return Err("history-aware options cannot be combined with --allow-fitted".into());
        }
        if let Some(path) = history_path {
            options.history = Some(std::sync::Arc::new(rage_texture::HistoryTables::from_path(
                path,
            )?));
            options.policy.get_or_insert(DecodePolicy::ConfirmedOnly);
        }
        Ok((args, options))
    }
    fn open(
        &self,
        path: impl AsRef<std::path::Path>,
        budget: usize,
        fallback: DecodePolicy,
    ) -> std::io::Result<Store> {
        let policy = self.policy.unwrap_or(fallback);
        match &self.history {
            Some(history) => Store::open_with_history(path, budget, policy, history.clone()),
            None => Store::open(path, budget, policy),
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let (args, decoder) = DecoderOptions::parse(env::args_os().collect())?;
    if args.len() == 2 && (args[1] == "--help" || args[1] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.len() < 3 {
        return Err(USAGE.into());
    }
    match args[1].to_str() {
        Some("range-hash")
            if args.len() == 5 || (args.len() == 6 && args[5] == "--allow-fitted") =>
        {
            let start: u64 = args[3].to_str().ok_or("invalid offset")?.parse()?;
            let len: usize = args[4].to_str().ok_or("invalid length")?.parse()?;
            let mut store = decoder.open(
                &args[2],
                0,
                if args.len() == 6 {
                    DecodePolicy::AllowFittedBc
                } else {
                    DecodePolicy::LosslessOnly
                },
            )?;
            let time = Instant::now();
            let (bytes, q) = store.read_range(start, len)?;
            let seconds = time.elapsed().as_secs_f64();
            println!(
                "{}",
                serde_json::json!({"bytes":bytes.len(),"quality":q,"seconds":seconds,"sha256":format!("{:x}",Sha256::digest(&bytes)),"stats":store.stats()})
            );
        }
        Some("mip-hash") if args.len() == 6 || (args.len() == 7 && args[6] == "--allow-fitted") => {
            let index: usize = args[3].to_str().ok_or("invalid index")?.parse()?;
            let mip: u32 = args[4].to_str().ok_or("invalid mip")?.parse()?;
            let slice: u32 = args[5].to_str().ok_or("invalid slice")?.parse()?;
            let mut store = decoder.open(
                &args[2],
                64 * 1024 * 1024,
                if args.len() == 7 {
                    DecodePolicy::AllowFittedBc
                } else {
                    DecodePolicy::LosslessOnly
                },
            )?;
            let ts = store.textures()?;
            let t = ts.get(index).ok_or("index out of range")?;
            let time = Instant::now();
            let (linear, q) = store.texture_mip(t, mip, slice)?;
            let rgba = rage_texture::preview::rgba8(
                t.dxgi,
                (t.width >> mip).max(1),
                (t.height >> mip).max(1),
                &linear,
            )?;
            let seconds = time.elapsed().as_secs_f64();
            println!(
                "{}",
                serde_json::json!({"bytes":rgba.len(),"quality":q,"seconds":seconds,"sha256":format!("{:x}",Sha256::digest(&rgba)),"linear_sha256":format!("{:x}",Sha256::digest(&linear))})
            );
        }
        Some("bench-export") if args.len() == 5 || (args.len() == 6 && args[5] == "--no-cache") => {
            let index: usize = args[3].to_str().ok_or("invalid texture index")?.parse()?;
            let trials: usize = args[4].to_str().ok_or("invalid trials")?.parse()?;
            if !(1..=31).contains(&trials) {
                return Err("trials must be 1..31".into());
            }
            let mut store = decoder.open(
                &args[2],
                if args.len() == 6 { 0 } else { 64 * 1024 * 1024 },
                DecodePolicy::LosslessOnly,
            )?;
            let catalog = store.textures()?;
            let texture = catalog.get(index).ok_or("texture index out of bounds")?;
            let mut elapsed = Vec::new();
            let mut sha256 = Vec::new();
            let mut bytes = 0usize;
            let mut stats = Vec::new();
            let mut stages = Vec::new();
            let mut input_bytes = Vec::new();
            for _ in 0..trials {
                store.reset();
                let started = Instant::now();
                let result = store.texture_dds(texture)?;
                elapsed.push(started.elapsed().as_secs_f64());
                bytes = result.dds.len();
                stats.push(store.stats());
                stages.push(result.timings);
                input_bytes.push(result.input_bytes);
                sha256.push(format!("{:x}", Sha256::digest(&result.dds)));
            }
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"seconds":elapsed,"sha256":sha256,"bytes":bytes,"stats":stats,"stages":stages,"input_bytes":input_bytes})
                )?
            );
        }
        Some("list") if args.len() == 3 => {
            let mut store = decoder.open(&args[2], 0, DecodePolicy::LosslessOnly)?;
            println!("{}", serde_json::to_string(&store.textures()?)?);
        }
        Some("export") if args.len() >= 5 => {
            let index: usize = args[3].to_str().ok_or("invalid index")?.parse()?;
            let mut mip = 0u32;
            let mut slice = 0u32;
            let mut fitted = false;
            let mut no_cache = false;
            let mut selected = false;
            let mut dds_mips = None;
            let mut at = 5;
            while at < args.len() {
                match args[at].to_str() {
                    Some("--allow-fitted") => fitted = true,
                    Some("--no-cache") => no_cache = true,
                    Some("--mips") => {
                        at += 1;
                        dds_mips = Some(
                            args.get(at)
                                .and_then(|v| v.to_str())
                                .ok_or("--mips needs a count")?
                                .parse::<u32>()?,
                        );
                    }
                    Some("--mip") | Some("--slice") => {
                        let flag = args[at].clone();
                        at += 1;
                        let value: u32 = args
                            .get(at)
                            .and_then(|x| x.to_str())
                            .ok_or("missing option value")?
                            .parse()?;
                        if flag == "--mip" {
                            mip = value
                        } else {
                            slice = value
                        }
                        selected = true;
                    }
                    _ => return Err("unknown export option".into()),
                }
                at += 1;
            }
            let mut store = decoder.open(
                &args[2],
                if no_cache { 0 } else { 64 * 1024 * 1024 },
                if fitted {
                    DecodePolicy::AllowFittedBc
                } else {
                    DecodePolicy::LosslessOnly
                },
            )?;
            let textures = store.textures()?;
            let texture = textures.get(index).ok_or("texture index out of range")?;
            let output = std::path::Path::new(&args[4]);
            let (bytes, quality) = match output
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("dds") => {
                    if selected {
                        return Err("--mip/--slice apply only to PNG previews".into());
                    }
                    let r = store.texture_dds_mips(texture, dds_mips.unwrap_or(texture.mips))?;
                    (r.dds, r.quality)
                }
                Some("png") => {
                    if dds_mips.is_some() {
                        return Err("--mips applies only to DDS exports".into());
                    }
                    store.texture_png(texture, mip, slice)?
                }
                _ => return Err("output extension must be .dds or .png".into()),
            };
            rage_texture::output::publish_new(output, &bytes)?;
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"bytes":bytes.len(),"quality":quality,"stats":store.stats()})
                )?
            );
        }
        Some("export-all") if args.len() >= 4 => {
            let mut fitted = false;
            let mut no_cache = false;
            for option in &args[4..] {
                match option.to_str() {
                    Some("--allow-fitted") => fitted = true,
                    Some("--no-cache") => no_cache = true,
                    _ => return Err("unknown export-all option".into()),
                }
            }
            let mut store = decoder.open(
                &args[2],
                if no_cache { 0 } else { 64 * 1024 * 1024 },
                if fitted {
                    DecodePolicy::AllowFittedBc
                } else {
                    DecodePolicy::LosslessOnly
                },
            )?;
            let textures = store.textures()?;
            let directory = std::path::Path::new(&args[3]);
            fs::create_dir_all(directory)?;
            let mut entries = Vec::new();
            let mut failed = 0usize;
            for texture in &textures {
                let name = match texture.asset_key {
                    Some(key) => format!("{:06}_{key:016x}.dds", texture.index),
                    None => format!("{:06}.dds", texture.index),
                };
                match store.texture_dds(texture).and_then(|r| {
                    rage_texture::output::publish_or_verify(&directory.join(&name), &r.dds)?;
                    Ok(r.quality)
                }) {
                    Ok(quality) => entries.push(
                        serde_json::json!({"index":texture.index,"file":name,"quality":quality}),
                    ),
                    Err(e) => {
                        failed += 1;
                        entries
                            .push(serde_json::json!({"index":texture.index,"error":e.to_string()}));
                    }
                }
            }
            let report = serde_json::to_vec_pretty(
                &serde_json::json!({"generator":"rage-texture","version":1,"decoder":env!("CARGO_PKG_VERSION"),"history_digest":store.history_digest().map(|d| d.iter().map(|b| format!("{b:02x}")).collect::<String>()),"failures":failed,"textures":entries}),
            )?;
            rage_texture::output::publish_manifest(&directory.join("manifest.json"), &report)?;
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"textures":textures.len(),"failures":failed,"stats":store.stats()})
                )?
            );
            if failed != 0 {
                return Err("batch incomplete; inspect manifest.json".into());
            }
        }
        Some("read-range") if args.len() == 6 => {
            let start: u64 = args[3].to_str().ok_or("invalid offset")?.parse()?;
            let len: usize = args[4].to_str().ok_or("invalid length")?.parse()?;
            let mut store = decoder.open(&args[2], 0, DecodePolicy::LosslessOnly)?;
            let (bytes, quality) = store.read_range(start, len)?;
            // Never overwrite an existing file, including the input or a hard link.
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[5])?;
            output.write_all(&bytes)?;
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"written_bytes":bytes.len(),"quality":quality})
                )?
            );
        }
        Some("inspect") if args.len() == 3 => {
            let store = decoder.open(&args[2], 0, DecodePolicy::LosslessOnly)?;
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({"virtual_size":store.virtual_size(),"chunks":store.chunks()})
                )?
            );
        }
        Some("run") if args.len() == 4 => {
            let plan: Plan = serde_json::from_slice(&fs::read(&args[3])?)?;
            if plan.ranges.is_empty()
                || plan.ranges.len() > 100_000
                || plan.repeats == 0
                || plan.repeats > 10_000
                || plan.trials == 0
                || plan.trials > 31
                || plan.cache_bytes > 1024 * 1024 * 1024
            {
                return Err("invalid benchmark bounds".into());
            }
            let mut store = decoder.open(
                &args[2],
                plan.cache_bytes,
                if plan.allow_fitted_bc1 {
                    DecodePolicy::AllowFittedBc1
                } else {
                    DecodePolicy::LosslessOnly
                },
            )?;
            let mut sha256 = Vec::new();
            for &(start, len) in &plan.ranges {
                sha256.push(format!(
                    "{:x}",
                    Sha256::digest(store.read_range(start, len)?.0)
                ));
            }
            let mut cold_seconds = Vec::new();
            let mut hot_seconds = Vec::new();
            for _ in 0..plan.trials {
                store.reset();
                let started = Instant::now();
                if plan.batch {
                    black_box(store.read_batch(&plan.ranges)?);
                } else {
                    for &(start, len) in &plan.ranges {
                        black_box(store.read_range(start, len)?);
                    }
                }
                cold_seconds.push(started.elapsed().as_secs_f64());
                // The same retained decoded working set is used by the baseline.
                let started = Instant::now();
                for _ in 0..plan.repeats {
                    if plan.batch {
                        black_box(store.read_batch(&plan.ranges)?);
                    } else {
                        for &(start, len) in &plan.ranges {
                            black_box(store.read_range(start, len)?);
                        }
                    }
                }
                hot_seconds.push(started.elapsed().as_secs_f64());
            }
            println!(
                "{}",
                serde_json::to_string(&Run {
                    sha256,
                    cold_seconds,
                    hot_seconds,
                    stats: store.stats()
                })?
            );
        }
        _ => return Err(format!("unknown command or argument count\n{USAGE}").into()),
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("rage-texture: {e}");
        std::process::exit(1);
    }
}
