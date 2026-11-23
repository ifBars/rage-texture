use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static ID: AtomicUsize = AtomicUsize::new(0);
#[test]
fn texture_exports_resume_only_identical_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.ttpf");
    let mut bytes = vec![0u8; 8192];
    bytes[..4].copy_from_slice(b"RDS5");
    bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&(8192u64 | (22u64 << 56)).to_le_bytes());
    bytes[24..32].copy_from_slice(&(16u64 | (1u64 << 30)).to_le_bytes());
    bytes[32..40].copy_from_slice(&(32u64 | (2u64 << 30)).to_le_bytes());
    bytes[256..260].copy_from_slice(b"ETPF");
    bytes[260..264].copy_from_slice(&634u32.to_le_bytes());
    bytes[264..268].copy_from_slice(&1u32.to_le_bytes());
    bytes[268..272].copy_from_slice(&24u32.to_le_bytes());
    let d0 = 63u32 | (63 << 14);
    let d1 = 1u32 | (71 << 3) | (1 << 21) | (2 << 23);
    bytes[272..276].copy_from_slice(&d0.to_le_bytes());
    bytes[276..280].copy_from_slice(&d1.to_le_bytes());
    bytes[280..284].copy_from_slice(&1u32.to_le_bytes());
    bytes[288..296].copy_from_slice(&123u64.to_le_bytes());
    fs::write(&source, &bytes).unwrap();
    let exe = env!("CARGO_BIN_EXE_rage-texture");
    let listed = Command::new(exe).arg("list").arg(&source).output().unwrap();
    assert!(listed.status.success());
    let catalog: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(catalog[0]["mips"], 1);
    let png = dir.path().join("preview.png");
    let r = Command::new(exe)
        .arg("export")
        .arg(&source)
        .arg("0")
        .arg(&png)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(&fs::read(&png).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    let output = dir.path().join("output");
    let batch = || {
        Command::new(exe)
            .arg("export-all")
            .arg(&source)
            .arg(&output)
            .arg("--no-cache")
            .output()
            .unwrap()
    };
    let first_batch = batch();
    assert!(first_batch.status.success());
    let stats: serde_json::Value = serde_json::from_slice(&first_batch.stdout).unwrap();
    for key in ["cache_hits", "cache_bytes", "peak_cache_bytes", "evictions"] {
        assert_eq!(stats["stats"][key], 0);
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    let file = output.join(manifest["textures"][0]["file"].as_str().unwrap());
    let dds = fs::read(&file).unwrap();
    assert_eq!(&dds[..4], b"DDS ");
    let history = dir.path().join("history.json");
    fs::write(&history, b"{}").unwrap();
    let partial = dir.path().join("partial.dds");
    let selected = Command::new(exe)
        .arg("export")
        .arg(&source)
        .arg("0")
        .arg(&partial)
        .args(["--mips", "1", "--no-cache", "--confirmed", "--history"])
        .arg(&history)
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(fs::read(&partial).unwrap(), dds);
    for options in [
        vec!["--mips", "0"],
        vec!["--confirmed", "--allow-fitted"],
        vec!["--confirmed", "--allow-provisional"],
    ] {
        let rejected = dir.path().join("rejected.dds");
        assert!(
            !Command::new(exe)
                .arg("export")
                .arg(&source)
                .arg("0")
                .arg(&rejected)
                .args(options)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(!rejected.exists());
    }
    assert!(batch().status.success());
    assert_eq!(fs::read(&file).unwrap(), dds);
    fs::write(&file, b"old or corrupt output").unwrap();
    assert!(!batch().status.success());
    assert_eq!(fs::read(&file).unwrap(), b"old or corrupt output");
    let bad = Command::new(exe)
        .arg("export")
        .arg(&source)
        .arg("0")
        .arg(dir.path().join("bad.dds"))
        .args(["--mip", "0"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(!dir.path().join("bad.dds").exists());
}
#[test]
fn read_range_extracts_bytes_and_never_overwrites() {
    let dir = std::env::temp_dir().join(format!(
        "rage-cli-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("source.ttpf");
    let output = dir.join("out.bin");
    let mut b = vec![0u8; 8192];
    b[..4].copy_from_slice(b"RDS5");
    b[4..8].copy_from_slice(&3u32.to_le_bytes());
    b[8..16].copy_from_slice(&(8192u64 | (22u64 << 56)).to_le_bytes());
    b[24..32].copy_from_slice(&(16u64 | (1u64 << 30)).to_le_bytes());
    b[32..40].copy_from_slice(&(32u64 | (2u64 << 30)).to_le_bytes());
    b[4096..].fill(73);
    fs::write(&source, &b).unwrap();
    let invoke = |out: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_rage-texture"))
            .arg("read-range")
            .arg(&source)
            .args(["4096", "4096"])
            .arg(out)
            .output()
            .unwrap()
    };
    let r = invoke(&output);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(fs::read(&output).unwrap(), vec![73; 4096]);
    assert!(!invoke(&source).status.success());
    assert_eq!(fs::read(&source).unwrap(), b);
    assert!(!invoke(&output).status.success());
    assert_eq!(fs::read(&output).unwrap(), vec![73; 4096]);
    fs::remove_file(output).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_dir(dir).unwrap();
}
