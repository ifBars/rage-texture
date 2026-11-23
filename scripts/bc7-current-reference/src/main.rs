//! Tiny batch hash reference for the current SixNG BC7 table revision.
//! Groups JSON: [{"path": "...", "chunks": [{"index": 0, "start": 0, "len": 16384}]}]
//! `start` is the neutral RDS5 offset; SixNG's extra header page is added here.
use sha2::{Digest, Sha256};
use std::env;
use std::error::Error;
use std::io;
use std::path::Path;
use std::time::Instant;

const SIXNG_REV: &str = "d724355df2398a646eb771d6d3da17c3b1bafe11";
const PR_HEAD: &str = "6b7d18924ce7a947f10ba7b609f11015f6f0eb71";

fn invalid(message: &str) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message))
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        return Err(invalid("usage: bc7-current-reference GROUPS.json"));
    }
    eprintln!("sixng_rev={SIXNG_REV} pr_head={PR_HEAD} cache=off");
    let groups: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    for group in groups.as_array().ok_or_else(|| invalid("groups must be an array"))? {
        let path = group["path"]
            .as_str()
            .ok_or_else(|| invalid("group.path must be a string"))?;
        let store = sixng_core::ttpf::Ttpf::open(Path::new(path))?;
        for chunk in group["chunks"]
            .as_array()
            .ok_or_else(|| invalid("group.chunks must be an array"))?
        {
            let index = chunk["index"]
                .as_u64()
                .ok_or_else(|| invalid("chunk.index must be an integer"))?;
            let start = chunk["start"]
                .as_u64()
                .ok_or_else(|| invalid("chunk.start must be an integer"))?;
            let len = chunk["len"]
                .as_u64()
                .ok_or_else(|| invalid("chunk.len must be an integer"))?;
            let virt = start
                .checked_add(4096)
                .ok_or_else(|| invalid("chunk.start overflow"))?;
            let started = Instant::now();
            let (data, skipped) = store.read_ex(virt, len)?;
            let seconds = started.elapsed().as_secs_f64();
            let sha256 = format!("{:x}", Sha256::digest(&data));
            println!(
                "{}",
                serde_json::json!({
                    "path": path,
                    "index": index,
                    "bytes": data.len(),
                    "sha256": sha256,
                    "skipped": skipped,
                    "seconds": seconds,
                })
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("bc7-current-reference: {error}");
        std::process::exit(1);
    }
}
