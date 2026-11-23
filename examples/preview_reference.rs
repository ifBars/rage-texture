//! Cache-disabled extraction/preview timing helper; game inputs remain external.
use rage_texture::{DecodePolicy, Store, preview};
use sha2::{Digest, Sha256};
use std::{error::Error, time::Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 {
        return Err("usage: preview_reference STORE INDEX MIP SLICE".into());
    }
    let number = |i: usize| -> Result<u32, Box<dyn Error>> {
        Ok(args[i].to_str().ok_or("non-UTF8 number")?.parse()?)
    };
    let mut store = Store::open(&args[1], 0, DecodePolicy::AllowFittedBc)?;
    let catalog = store.textures()?;
    let texture = catalog
        .get(number(2)? as usize)
        .ok_or("index outside catalog")?;
    let mip = number(3)?;
    let slice = number(4)?;
    let started = Instant::now();
    let (linear, quality) = store.texture_mip(texture, mip, slice)?;
    let rgba = preview::rgba8(
        texture.dxgi,
        (texture.width >> mip).max(1),
        (texture.height >> mip).max(1),
        &linear,
    )?;
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::json!({
            "bytes": rgba.len(), "sha256": format!("{:x}",Sha256::digest(&rgba)),
            "seconds":seconds, "stats":store.stats(), "quality":quality,
            "asset_key":texture.asset_key, "mips":texture.mips
        })
    );
    Ok(())
}
