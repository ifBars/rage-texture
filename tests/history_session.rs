//! End-to-end history-policy and source-generation regressions. Synthetic only.
use rage_texture::{DecodePolicy, HistoryTables, Quality, Store};
use std::{fs, io, sync::Arc};

fn tables(red: bool, provisional: bool) -> Arc<HistoryTables> {
    let endpoints = if red {
        [31, 0, 0, 31, 0, 0]
    } else {
        [0, 63, 0, 0, 63, 0]
    };
    let mut section = serde_json::json!({"replace":true});
    section[if provisional {
        "provisional"
    } else {
        "confirmed"
    }] = serde_json::json!({"255":endpoints});
    Arc::new(
        HistoryTables::from_json(&serde_json::to_vec(&serde_json::json!({"bc1":section})).unwrap())
            .unwrap(),
    )
}
fn fixture(path: &std::path::Path) {
    use bcpack::encode::{Fragment, encode_lanes};
    let lanes: Vec<_> = (0..12)
        .map(|i| {
            vec![Fragment {
                symbols: vec![if i == 0 { 2 } else { 0 }; 512],
                q: 0,
                z: 0,
            }]
        })
        .collect();
    let packed = encode_lanes(&bcpack::bc1::BC1_ALPHABET_BITS, &lanes).unwrap();
    let physical = (4096 + packed.len()).div_ceil(256) * 256;
    let pad = physical - 4096 - packed.len();
    let mut data = vec![0u8; physical];
    data[..4].copy_from_slice(b"RDS5");
    data[4..8].copy_from_slice(&3u32.to_le_bytes());
    data[8..16].copy_from_slice(&(8192u64 | (22 << 56)).to_le_bytes());
    data[24..32]
        .copy_from_slice(&(16u64 | (1 << 26) | (1 << 30) | ((pad as u64) << 52)).to_le_bytes());
    data[32..40].copy_from_slice(&((physical as u64 / 256) | (2 << 30)).to_le_bytes());
    data[256..260].copy_from_slice(b"ETPF");
    data[260..264].copy_from_slice(&634u32.to_le_bytes());
    data[264..268].copy_from_slice(&1u32.to_le_bytes());
    data[268..272].copy_from_slice(&24u32.to_le_bytes());
    data[272..276].copy_from_slice(&(63u32 | (63 << 14)).to_le_bytes());
    data[276..280].copy_from_slice(&(1u32 | (71 << 3) | (1 << 21) | (2 << 23)).to_le_bytes());
    data[280..284].copy_from_slice(&1u32.to_le_bytes());
    data[288..296].copy_from_slice(&123u64.to_le_bytes());
    data[4096..4096 + packed.len()].copy_from_slice(&packed);
    fs::write(path, data).unwrap();
}

#[test]
fn changed_history_flushes_cache_and_invalidates_descriptors_refresh_retains_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ttpf");
    fixture(&path);
    let red = tables(true, false);
    let green = tables(false, false);
    let mut store =
        Store::open_with_history(&path, 4096, DecodePolicy::ConfirmedOnly, red.clone()).unwrap();
    let old = store.textures().unwrap().remove(0);
    assert_eq!(
        &store.read_chunk(0).unwrap().bytes[..2],
        &0xf800u16.to_le_bytes()
    );
    store.read_chunk(0).unwrap();
    assert_eq!(store.stats().cache_hits, 1);
    store.replace_history(red.clone()).unwrap();
    assert_eq!(store.stats().cache_hits, 1);
    store.texture_dds(&old).unwrap();
    store.replace_history(green.clone()).unwrap();
    assert_eq!((store.stats().cache_bytes, store.stats().decodes), (0, 0));
    assert!(store.texture_dds(&old).is_err());
    let new = store.textures().unwrap().remove(0);
    let output = store.texture_dds(&new).unwrap();
    assert_eq!(&output.dds[148..150], &0x07e0u16.to_le_bytes());
    assert!(
        matches!(output.quality[0],Quality::HistoryBc{provisional_dependencies:0,history_digest,..} if history_digest==green.digest())
    );
    store.refresh().unwrap();
    assert_eq!(store.history_digest(), Some(green.digest()));
    assert_eq!(
        &store.read_chunk(0).unwrap().bytes[..2],
        &0x07e0u16.to_le_bytes()
    );
}

#[test]
fn strict_and_provisional_policies_never_substitute_unknown_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ttpf");
    fixture(&path);
    let provisional = tables(true, true);
    let mut strict =
        Store::open_with_history(&path, 0, DecodePolicy::ConfirmedOnly, provisional.clone())
            .unwrap();
    assert_eq!(
        strict.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    let mut loose =
        Store::open_with_history(&path, 0, DecodePolicy::AllowProvisional, provisional).unwrap();
    let result = loose.read_chunk(0).unwrap();
    assert_eq!(&result.bytes[..2], &0xf800u16.to_le_bytes());
    assert!(
        matches!(result.quality,Quality::HistoryBc{provisional_dependencies,..} if provisional_dependencies>0)
    );
    assert_eq!(
        (
            loose.stats().cache_hits,
            loose.stats().cache_bytes,
            loose.stats().peak_cache_bytes
        ),
        (0, 0, 0)
    );
    let empty = Arc::new(HistoryTables::from_json(br#"{"bc1":{"replace":true}}"#).unwrap());
    for policy in [DecodePolicy::ConfirmedOnly, DecodePolicy::AllowProvisional] {
        let mut store = Store::open_with_history(&path, 4096, policy, empty.clone()).unwrap();
        assert_eq!(
            store.read_chunk(0).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(store.stats().decodes, 0);
        assert_eq!(store.stats().cache_bytes, 0);
    }
}

#[test]
fn loading_history_again_reads_content_even_when_length_and_mtime_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bc1.json");
    let red = br#"{"bc1":{"replace":true,"confirmed":{"255":[31,0,0,31,0,0]}}}"#;
    let green = br#"{"bc1":{"replace":true,"confirmed":{"255":[0,63,0,0,63,0]}}}"#;
    assert_eq!(red.len(), green.len());
    fs::write(&path, red).unwrap();
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    let a = HistoryTables::from_path(&path).unwrap();
    fs::write(&path, green).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    let b = HistoryTables::from_path(&path).unwrap();
    assert_ne!(a.digest(), b.digest());
    assert_eq!(a.digest(), HistoryTables::from_json(red).unwrap().digest());
    assert_eq!(
        b.digest(),
        HistoryTables::from_path(dir.path()).unwrap().digest()
    );
}

#[test]
fn malformed_history_is_rejected_and_canonical_document_order_has_one_identity() {
    for bad in [b"[]".as_slice(), b"{", br#"{"typo":{}}"#, br#"{"bc1":[]}"#] {
        assert!(HistoryTables::from_json(bad).is_err());
    }
    let a = HistoryTables::from_json(
        br#"{"bc1":{"replace":true,"confirmed":{"255":[31,0,0,31,0,0]}}}"#,
    )
    .unwrap();
    let b = HistoryTables::from_json(
        br#"{ "bc1": {"confirmed":{"255":[31,0,0,31,0,0]},"replace":true} }"#,
    )
    .unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.digest_hex().len(), 64);
    assert!(
        HistoryTables::from_json(
            br#"{"bc1":{"confirmed":{"1":[0,0,0,0,0,0],"01":[1,0,0,0,0,0]}}}"#
        )
        .is_err()
    );
}
