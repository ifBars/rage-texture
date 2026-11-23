//! Public-API coverage for the classic BC history snapshot.
//!
//! Block reconstruction is exercised directly in `src/history_bc.rs` with lane
//! fixtures and a differential oracle. These tests cover the public
//! `HistoryTables` surface: parsing, validation, digests, overlay/replace
//! documents, and file/directory loading. The bundled constants are an
//! attributed GPL-3.0 port of Leonida's clean-room tables at
//! `aa2988cc555bad7ecd508b48a702adbb32fe7c95`; no game data is used.
use rage_texture::HistoryTables;
use serde_json::json;
use std::io::ErrorKind;

fn builtin() -> HistoryTables {
    HistoryTables::from_json(b"{}").expect("an empty document selects the bundled tables")
}

fn error_kind(document: &[u8]) -> ErrorKind {
    HistoryTables::from_json(document)
        .err()
        .expect("expected an invalid history document")
        .kind()
}

#[test]
fn builtin_and_empty_documents_have_a_stable_digest() {
    let bundled = HistoryTables::builtin();
    let empty = builtin();
    assert_eq!(bundled.digest(), empty.digest());
    assert_eq!(empty.digest(), builtin().digest());
    assert_eq!(empty.digest().len(), 32);
    assert_eq!(empty.digest_hex().len(), 64);
    assert!(empty.digest_hex().bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn overlay_and_replacement_documents_change_the_digest() {
    let base = builtin();
    let overlay =
        HistoryTables::from_json(br#"{"bc1":{"confirmed":{"1":[1,2,3,4,5,6]}}}"#).unwrap();
    let replaced =
        HistoryTables::from_json(br#"{"bc1":{"replace":true,"confirmed":{"1":[1,2,3,4,5,6]}}}"#)
            .unwrap();
    assert_ne!(base.digest(), overlay.digest());
    assert_ne!(base.digest(), replaced.digest());
    assert_ne!(overlay.digest(), replaced.digest());

    let again =
        HistoryTables::from_json(br#"{"bc1":{"replace":true,"confirmed":{"1":[1,2,3,4,5,6]}}}"#)
            .unwrap();
    assert_eq!(replaced.digest(), again.digest());
}

#[test]
fn dense_arrays_and_sparse_objects_both_validate() {
    let mut rows = vec![json!([0, 0, 0, 0, 0, 0]); 256];
    rows[1] = json!([1, 2, 3, 4, 5, 6]);
    let dense_document =
        serde_json::to_vec(&json!({"bc1": {"replace": true, "confirmed": rows}})).unwrap();
    let dense = HistoryTables::from_json(&dense_document).unwrap();
    let dense_again = HistoryTables::from_json(&dense_document).unwrap();
    let sparse =
        HistoryTables::from_json(br#"{"bc1":{"replace":true,"confirmed":{"1":[1,2,3,4,5,6]}}}"#)
            .unwrap();
    assert_eq!(dense.digest(), dense_again.digest());
    // The digest covers the normalized document as supplied, so the same
    // entries in a different encoding are a distinct (conservatively
    // invalidating) snapshot rather than a silent cache hit.
    assert_ne!(dense.digest(), sparse.digest());
}

#[test]
fn a_single_codec_document_is_accepted() {
    let tables =
        HistoryTables::from_json(br#"{"codec":"bc1","confirmed":{"1":[1,2,3,4,5,6]}}"#).unwrap();
    assert_ne!(tables.digest(), builtin().digest());
}

#[test]
fn valid_boundary_values_are_accepted_for_every_classic_codec() {
    for document in [
        br#"{"bc1":{"confirmed":{"1":[31,63,31,31,63,31]}}}"#.as_slice(),
        br#"{"bc3":{"confirmed":{"1":[255,255,31,63,31,31,63,31]}}}"#.as_slice(),
        br#"{"bc4":{"provisional":{"1":[255,255]}}}"#.as_slice(),
        br#"{"bc5":{"provisional":{"1":[255,255,255,255]}}}"#.as_slice(),
    ] {
        HistoryTables::from_json(document).unwrap();
    }
}

#[test]
fn malformed_documents_are_invalid_input() {
    for document in [
        b"not json".as_slice(),
        b"[]".as_slice(),
        br#"{"bc1":[]}"#.as_slice(),
        br#"{"bc1":{"confirmed":[[0,0,0,0,0,0]]}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"256":[0,0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"-1":[0,0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"x":[0,0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"1":[0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"1":[32,0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"1":[0,0,0,0,64,0]}}}"#.as_slice(),
        br#"{"bc1":{"confirmed":{"1":[0.5,0,0,0,0,0]}}}"#.as_slice(),
        br#"{"bc1":{"replace":1}}"#.as_slice(),
        br#"{"bc1":{"codec":"bc3"}}"#.as_slice(),
        br#"{"codec":"bc9"}"#.as_slice(),
        br#"{"unknown":{}}"#.as_slice(),
        br#"{"bc3":{"confirmed":{"1":[0,0,0,0,0,0,64,0]}}}"#.as_slice(),
        br#"{"bc4":{"confirmed":{"1":[256,0]}}}"#.as_slice(),
        br#"{"bc5":{"provisional":{"1":[0,0,0]}}}"#.as_slice(),
    ] {
        assert_eq!(
            error_kind(document),
            ErrorKind::InvalidInput,
            "document {}",
            String::from_utf8_lossy(document)
        );
    }
}

#[test]
fn files_and_directories_are_loaded_as_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bc1.json");
    std::fs::write(&file, br#"{"codec":"bc1","confirmed":{"1":[1,2,3,4,5,6]}}"#).unwrap();
    let from_file = HistoryTables::from_path(&file).unwrap();
    assert_ne!(from_file.digest(), builtin().digest());

    let from_dir = HistoryTables::from_path(dir.path()).unwrap();
    assert_eq!(from_dir.digest(), from_file.digest());

    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        HistoryTables::from_path(empty.path())
            .err()
            .expect("empty directory")
            .kind(),
        ErrorKind::InvalidInput
    );

    let unrelated = tempfile::tempdir().unwrap();
    std::fs::write(unrelated.path().join("notes.txt"), b"not a codec").unwrap();
    assert_eq!(
        HistoryTables::from_path(unrelated.path())
            .err()
            .expect("unrelated directory")
            .kind(),
        ErrorKind::InvalidInput
    );
}
