//! Independent synthetic RDS5 checks (test/review task).
//!
//! Every fixture here is generated from bytes in this file and written to a
//! per-test temporary directory; no game data, decoded bytes or local paths
//! enter the repository.
//!
//! Scope, chosen to complement `tests/reader.rs` and `tests/cli.rs`:
//!
//! * decode provenance for the fitted codecs, in particular that a fitted BC1
//!   chunk is never reported as lossless and that `LosslessOnly` cannot decode
//!   it by accident;
//! * chunk-table validation (the table is the only source of layout truth, so
//!   every derived field is an attack surface);
//! * strict refusal of unknown codecs under *every* policy;
//! * zlib framing: truncation, checksum, trailing input, declared-extent
//!   mismatch, and a decompression bomb;
//! * non-zero chunk padding (the new format does not require zero filler);
//! * the cache regression requirements: no cross-file contamination, detection
//!   of same-size in-place updates, detection of pathname replacement, and a
//!   `refresh` that discards the old generation's bytes and metadata.
use rage_texture::{DecodePolicy, Limits, Quality, Store};
use std::{
    fs,
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// RDS5 addresses are 4096-byte units and offsets are 256-byte units, so every
/// representable chunk extent is a positive multiple of 4096.
const PAGE: usize = 4096;

// ---------------------------------------------------------------------------
// Fixture plumbing
// ---------------------------------------------------------------------------

static ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    dir: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rage-independent-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("synthetic.ttpf");
        fs::write(&path, bytes).unwrap();
        Self { dir, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Overwrite the existing inode with the same number of bytes. This is the
    /// case a path-only guard would miss, so the identity/stamp guard must
    /// catch it.
    fn write_in_place(&self, bytes: &[u8]) {
        let current = fs::metadata(&self.path).unwrap().len();
        assert_eq!(
            current,
            bytes.len() as u64,
            "in-place update must not change the source length"
        );
        // Give the filesystem timestamp a chance to move even on coarse
        // timestamp granularity. The size is intentionally unchanged above.
        std::thread::sleep(Duration::from_millis(20));
        let mut file = fs::OpenOptions::new().write(true).open(&self.path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }

    /// Replace the pathname with a different file (rename over the old name).
    /// The old handle keeps the old inode/inode-equivalent; the path guard must
    /// notice that the path now resolves to a different file.
    fn replace(&self, bytes: &[u8]) {
        let staged = self.dir.join("replacement.tmp");
        fs::write(&staged, bytes).unwrap();
        fs::rename(&staged, &self.path).unwrap();
    }

    fn open(&self, budget: usize, policy: DecodePolicy) -> Store {
        Store::open(&self.path, budget, policy).expect("fixture store opens")
    }

    fn open_with_limits(
        &self,
        budget: usize,
        policy: DecodePolicy,
        limits: Limits,
    ) -> io::Result<Store> {
        Store::open_with_limits(&self.path, budget, policy, limits)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// Synthetic store builder
// ---------------------------------------------------------------------------

struct ChunkPlan {
    codec: u8,
    packed: Vec<u8>,
    /// Virtual extent in bytes. Must be a positive multiple of 4096 because the
    /// table can only express extents between two 4096-byte address units.
    decoded: usize,
    /// Byte used for the alignment filler between `packed` and the next chunk.
    pad_fill: u8,
}

fn page(codec: u8, packed: Vec<u8>, pad_fill: u8) -> ChunkPlan {
    ChunkPlan {
        codec,
        packed,
        decoded: PAGE,
        pad_fill,
    }
}

fn zlib(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

fn set_entry(bytes: &mut [u8], index: usize, value: u64) {
    bytes[24 + index * 8..32 + index * 8].copy_from_slice(&value.to_le_bytes());
}

fn entry(bytes: &[u8], index: usize) -> u64 {
    u64::from_le_bytes(bytes[24 + index * 8..32 + index * 8].try_into().unwrap())
}

const OFFSET_MASK: u64 = 0x0000_0000_003f_ffff;
const ADDRESS_MASK: u64 = 0x000f_ffff_c000_0000;
const CODEC_MASK: u64 = 0x0000_0000_3c00_0000;
const PAD_MASK: u64 = 0xfff0_0000_0000_0000;

fn patch_offset(value: u64, units: u64) -> u64 {
    (value & !OFFSET_MASK) | (units & OFFSET_MASK)
}

fn patch_address(value: u64, units: u64) -> u64 {
    (value & !ADDRESS_MASK) | ((units & 0x3f_ffff) << 30)
}

fn patch_codec(value: u64, codec: u8) -> u64 {
    (value & !CODEC_MASK) | ((codec as u64) << 26)
}

fn patch_pad(value: u64, pad: u64) -> u64 {
    (value & !PAD_MASK) | ((pad & 0xfff) << 52)
}

/// Build a well-formed store whose chunk table is internally consistent:
/// `count` = chunks + 2, extents are contiguous in both address spaces, the
/// sentinel reproduces `file_len` and `virtual_size`, and every span is
/// 256-byte aligned so `pad` describes exactly the filler.
fn build(plans: &[ChunkPlan]) -> Vec<u8> {
    assert!(!plans.is_empty(), "a store needs at least one data chunk");
    let entries = plans.len() + 1;
    let metadata_end = (24 + entries * 8).div_ceil(256) * 256;
    // `open` requires the first chunk to start at least 16 bytes past the
    // rounded metadata end; 256 keeps every fixture comfortably aligned.
    let data_start = (metadata_end + 256).max(PAGE);

    let mut addresses = Vec::with_capacity(plans.len());
    let mut virtual_size = 0u64;
    for plan in plans {
        assert_eq!(
            plan.decoded % PAGE,
            0,
            "chunk extents are 4096-byte virtual units"
        );
        assert!(plan.decoded > 0);
        addresses.push(virtual_size / PAGE as u64);
        virtual_size += plan.decoded as u64;
    }

    let mut bytes = vec![0u8; data_start];
    bytes[..4].copy_from_slice(b"RDS5");
    bytes[4..8].copy_from_slice(&((plans.len() + 2) as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&(virtual_size | (22u64 << 56)).to_le_bytes());

    let mut at = data_start;
    for (i, plan) in plans.iter().enumerate() {
        let span = plan.packed.len().div_ceil(256) * 256;
        let pad = span - plan.packed.len();
        let entry = (at as u64 / 256)
            | ((plan.codec as u64) << 26)
            | (addresses[i] << 30)
            | ((pad as u64) << 52);
        set_entry(&mut bytes, i, entry);
        bytes.extend_from_slice(&plan.packed);
        bytes.resize(at + span, plan.pad_fill);
        at += span;
    }
    let sentinel = (at as u64 / 256) | ((virtual_size / PAGE as u64) << 30);
    set_entry(&mut bytes, plans.len(), sentinel);
    bytes
}

fn expect_invalid(bytes: &[u8]) {
    let fixture = Fixture::new(bytes);
    match Store::open(fixture.path(), 0, DecodePolicy::LosslessOnly) {
        Ok(_) => panic!("malformed store opened without an error"),
        Err(error) => assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidData,
            "malformed store produced {error:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// Fitted BC1: explicit opt-in and honest provenance
// ---------------------------------------------------------------------------

#[test]
fn fitted_bc1_requires_an_explicit_opt_in() {
    const BLOCKS: usize = 512;
    let (stream, _) = bcpack::encode::synthetic_bc1(0x5eed_1234, BLOCKS, 32);
    let bytes = build(&[page(1, stream.clone(), 0)]);
    let fixture = Fixture::new(&bytes);

    // The codec is known to the build, but the default policy must not decode
    // reconstructed pixels.
    let mut lossless = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    assert!(lossless.chunks()[0].supported());
    let error = lossless.read_chunk(0).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(lossless.stats().decodes, 0);
    assert_eq!(lossless.stats().cache_bytes, 0);

    let mut fitted = fixture.open(PAGE, DecodePolicy::AllowFittedBc1);
    let decoded = fitted.read_chunk(0).unwrap();
    assert_eq!(decoded.bytes.len(), BLOCKS * 8);

    let window = bcpack::bc1::Window::default_table();
    let quads = bcpack::bc1::QuadTable::default_table();
    let direct = bcpack::bc1::decode(&stream, BLOCKS, &window, &quads).unwrap();
    // The store must return exactly the upstream reconstruction ...
    assert_eq!(decoded.bytes.as_ref(), direct.blocks.as_slice());
    // ... and must not claim it is lossless: provenance is per chunk.
    assert_eq!(
        decoded.quality,
        Quality::FittedBc1 {
            unfitted_blocks: direct.unfitted_blocks,
        }
    );
    assert_ne!(decoded.quality, Quality::LosslessContainer);
    assert!(direct.unfitted_blocks <= BLOCKS);

    // A cached repeat keeps the same provenance and byte identity.
    let cached = fitted.read_chunk(0).unwrap();
    assert_eq!(cached.quality, decoded.quality);
    assert_eq!(cached.bytes.as_ref(), decoded.bytes.as_ref());
    assert_eq!(fitted.stats().cache_hits, 1);
    assert_eq!(fitted.stats().decodes, 1);
}

#[test]
fn fitted_bc1_provenance_is_per_chunk_not_per_store() {
    const BLOCKS: usize = 512;
    let (stream, _) = bcpack::encode::synthetic_bc1(0x1234_5678, BLOCKS, 64);
    let lossless_page = vec![0x5a; PAGE];
    let bytes = build(&[page(0, lossless_page.clone(), 0), page(1, stream, 0)]);
    let fixture = Fixture::new(&bytes);
    let mut store = fixture.open(2 * PAGE, DecodePolicy::AllowFittedBc1);

    let (data, quality) = store.read_range(0, 2 * PAGE).unwrap();
    assert_eq!(&data[..PAGE], lossless_page.as_slice());
    assert_eq!(quality.len(), 2);
    assert_eq!(quality[0], Quality::LosslessContainer);
    assert!(matches!(quality[1], Quality::FittedBc1 { .. }));
    assert_ne!(quality[1], Quality::LosslessContainer);

    // Reading the fitted page in isolation reports the same class.
    let mut isolated = fixture.open(0, DecodePolicy::AllowFittedBc1);
    let chunk = isolated.read_chunk(1).unwrap();
    assert_eq!(chunk.quality, quality[1]);
}

#[test]
fn corrupt_fitted_bc1_is_refused_not_zero_filled() {
    const BLOCKS: usize = 512;
    let (stream, _) = bcpack::encode::synthetic_bc1(0x0bad_f00d, BLOCKS, 16);

    // A truncated entropy stream must not decode to a partial page.
    let truncated = stream[..stream.len() / 2].to_vec();
    let fixture = Fixture::new(&build(&[page(1, truncated, 0)]));
    let mut store = fixture.open(PAGE, DecodePolicy::AllowFittedBc1);
    let error = store.read_chunk(0).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(store.stats().decodes, 0);
    assert_eq!(store.stats().cache_bytes, 0);
    assert_eq!(store.stats().peak_cache_bytes, 0);

    // Garbage that is not a BCPack stream must fail explicitly as well.
    let garbage = vec![0xa5; 1024];
    let fixture = Fixture::new(&build(&[page(1, garbage, 0x7f)]));
    let mut store = fixture.open(PAGE, DecodePolicy::AllowFittedBc1);
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(store.stats().cache_bytes, 0);
}

// ---------------------------------------------------------------------------
// Chunk table validation
// ---------------------------------------------------------------------------

#[test]
fn malformed_chunk_tables_are_rejected() {
    let base = build(&[page(0, vec![1; PAGE], 0), page(0, vec![2; PAGE], 0)]);

    // Header-level disagreement.
    let mut bad = base.clone();
    bad[4..8].copy_from_slice(&2u32.to_le_bytes());
    expect_invalid(&bad);

    let mut bad = base.clone();
    bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    expect_invalid(&bad);

    // count-1 entries do not fit in the file.
    let mut bad = base.clone();
    bad[4..8].copy_from_slice(&2000u32.to_le_bytes());
    expect_invalid(&bad);

    // Magic / size tag / virtual size floor.
    let mut bad = base.clone();
    bad[..4].copy_from_slice(b"RDS4");
    expect_invalid(&bad);
    let mut bad = base.clone();
    bad[15] = 23;
    expect_invalid(&bad);
    let mut bad = base.clone();
    bad[8..16].copy_from_slice(&(22u64 << 56).to_le_bytes());
    expect_invalid(&bad);

    // Sentinel must reproduce both source lengths.
    let mut bad = base.clone();
    let sentinel = entry(&bad, 2);
    set_entry(&mut bad, 2, patch_offset(sentinel, 49));
    expect_invalid(&bad);
    let mut bad = base.clone();
    let sentinel = entry(&bad, 2);
    set_entry(&mut bad, 2, patch_address(sentinel, 3));
    expect_invalid(&bad);

    // Reversed file extent.
    let mut bad = base.clone();
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_offset(first, 40));
    expect_invalid(&bad);

    // Chunk end beyond the declared file length.
    let mut bad = base.clone();
    let second = entry(&bad, 1);
    set_entry(&mut bad, 1, patch_offset(second, 50));
    expect_invalid(&bad);

    // Zero decoded extent (two entries share an address).
    let mut bad = base.clone();
    let second = entry(&bad, 1);
    set_entry(&mut bad, 1, patch_address(second, 0));
    expect_invalid(&bad);

    // Reversed virtual extent.
    let mut bad = base.clone();
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_address(first, 1));
    let second = entry(&bad, 1);
    set_entry(&mut bad, 1, patch_address(second, 0));
    expect_invalid(&bad);

    // Chunk whose data starts inside the rounded metadata region.
    let compressed = zlib(&vec![3u8; PAGE]);
    let small = build(&[page(8, compressed.clone(), 0)]);
    let mut bad = small.clone();
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_offset(first, 1));
    expect_invalid(&bad);

    // Padding longer than the chunk span and a zero packed extent.
    let mut bad = small.clone();
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_pad(first, 300));
    expect_invalid(&bad);
    let mut bad = small;
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_pad(first, 256));
    expect_invalid(&bad);

    // A known codec with a stored extent that disagrees with the decode size.
    let compressed = zlib(&vec![4u8; PAGE]);
    let mut bad = build(&[page(8, compressed, 0)]);
    let first = entry(&bad, 0);
    set_entry(&mut bad, 0, patch_codec(first, 0));
    expect_invalid(&bad);
}

#[test]
fn chunk_count_limits_are_enforced_at_the_boundary() {
    let bytes = build(&[page(0, vec![7; PAGE], 0), page(0, vec![8; PAGE], 0)]);
    let fixture = Fixture::new(&bytes);

    assert!(
        fixture
            .open_with_limits(
                0,
                DecodePolicy::LosslessOnly,
                Limits {
                    max_chunks: 1,
                    ..Limits::default()
                }
            )
            .is_err()
    );
    let mut store = fixture
        .open_with_limits(
            0,
            DecodePolicy::LosslessOnly,
            Limits {
                max_chunks: 2,
                ..Limits::default()
            },
        )
        .expect("count-2 == max_chunks is in bounds");
    assert_eq!(store.chunks().len(), 2);
    assert_eq!(store.read_chunk(1).unwrap().bytes.len(), PAGE);
}

// ---------------------------------------------------------------------------
// Strict codec refusal
// ---------------------------------------------------------------------------

#[test]
fn reserved_codecs_fail_explicitly_under_every_policy() {
    for codec in [2u8, 9, 10, 11, 12, 13, 14, 15] {
        let bytes = build(&[page(codec, vec![0x6b; PAGE], 0)]);
        let fixture = Fixture::new(&bytes);
        for policy in [
            DecodePolicy::LosslessOnly,
            DecodePolicy::AllowFittedBc1,
            DecodePolicy::AllowFittedBc,
        ] {
            let mut store = fixture.open(PAGE, policy);
            assert!(!store.chunks()[0].supported(), "codec {codec} claimed");
            let error = store.read_chunk(0).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::Unsupported,
                "codec {codec} under {policy:?}"
            );
            assert_eq!(store.stats().decodes, 0);
            assert_eq!(store.stats().cache_bytes, 0);
            // A range read must propagate the same refusal, never filler bytes.
            assert_eq!(
                store.read_range(0, 16).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
    }
}

#[test]
fn fitted_bc_family_still_needs_its_own_opt_in() {
    // Codec 3..=7 stay refused until the caller accepts fitted reconstruction.
    let bytes = build(&[page(3, vec![0x2c; 256], 0)]);
    let fixture = Fixture::new(&bytes);
    let mut store = fixture.open(PAGE, DecodePolicy::AllowFittedBc1);
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(store.stats().cache_bytes, 0);
}

// ---------------------------------------------------------------------------
// zlib framing
// ---------------------------------------------------------------------------

#[test]
fn zlib_framing_and_extent_disagreements_are_rejected() {
    let raw: Vec<u8> = (0..PAGE).map(|i| (i % 251) as u8).collect();
    let stream = zlib(&raw);

    // Baseline: an exact stream decodes losslessly.
    let fixture = Fixture::new(&build(&[page(8, stream.clone(), 0)]));
    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    let chunk = store.read_chunk(0).unwrap();
    assert_eq!(chunk.bytes.as_ref(), raw.as_slice());
    assert_eq!(chunk.quality, Quality::LosslessContainer);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("truncated tail", stream[..stream.len() - 1].to_vec()),
        ("truncated half", stream[..stream.len() / 2].to_vec()),
        ("checksum", {
            let mut broken = stream.clone();
            let last = broken.len() - 1;
            broken[last] ^= 0xff;
            broken
        }),
        ("trailing input", {
            let mut extra = stream.clone();
            extra.extend_from_slice(&[1, 2, 3, 4]);
            extra
        }),
    ];
    for (label, packed) in cases {
        let fixture = Fixture::new(&build(&[page(8, packed, 0)]));
        let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
        let error = store.read_chunk(0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
        assert_eq!(store.stats().decodes, 0, "{label}");
        assert_eq!(store.stats().cache_bytes, 0, "{label}");
    }

    // Declared extent larger than the stream's output.
    let fixture = Fixture::new(&build(&[ChunkPlan {
        codec: 8,
        packed: stream.clone(),
        decoded: 2 * PAGE,
        pad_fill: 0,
    }]));
    let mut store = fixture.open(0, DecodePolicy::LosslessOnly);
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    // Declared extent smaller than what the stream would produce: output must
    // stop at the declared bound and be refused, not truncated silently.
    let larger = zlib(&vec![9u8; 2 * PAGE]);
    let fixture = Fixture::new(&build(&[page(8, larger, 0)]));
    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(store.stats().decodes, 0);
    assert_eq!(store.stats().cache_bytes, 0);

    // Decompression bomb: a small packed extent that expands far past the
    // declared page must be bounded and refused.
    let bomb = zlib(&vec![0u8; 8 * 1024 * 1024]);
    assert!(
        bomb.len() * 100 < 8 * 1024 * 1024,
        "fixture must compress by more than 100x, got {} bytes",
        bomb.len()
    );
    let fixture = Fixture::new(&build(&[page(8, bomb, 0)]));
    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(store.stats().decodes, 0);
    assert_eq!(store.stats().peak_cache_bytes, 0);
}

// ---------------------------------------------------------------------------
// Filler bytes are not data
// ---------------------------------------------------------------------------

#[test]
fn nonzero_chunk_padding_is_ignored() {
    let first: Vec<u8> = (0..PAGE).map(|i| (i % 249) as u8).collect();
    let second: Vec<u8> = (0..PAGE).map(|i| ((i * 7) % 253) as u8).collect();
    // Deliberately non-zero fillers: nothing in the format requires the bytes
    // between the packed extent and the next chunk to be zero.
    let plans = [page(8, zlib(&first), 0xd5), page(8, zlib(&second), 0xa7)];
    let mut bytes = build(&plans);

    // The metadata gap between the table and the first chunk is also not
    // required to be zero (for example, ETPF metadata may begin there).
    for (i, slot) in bytes.iter_mut().enumerate().take(1024).skip(64) {
        *slot = (i as u8) | 1;
    }
    // Prove the fixture really carries non-zero filler between extents; the
    // equality checks below then show it never reaches the decoded bytes.
    let first_packed = zlib(&first);
    let second_packed = zlib(&second);
    let first_span = first_packed.len().div_ceil(256) * 256;
    let second_span = second_packed.len().div_ceil(256) * 256;
    let first_entry = entry(&bytes, 0);
    let first_start = ((first_entry & OFFSET_MASK) * 256) as usize;
    assert_eq!(
        (first_entry >> 52) as usize,
        first_span - first_packed.len()
    );
    assert!(
        bytes[first_start + first_packed.len()..first_start + first_span]
            .iter()
            .all(|&b| b == 0xd5)
    );
    let second_entry = entry(&bytes, 1);
    let second_start = ((second_entry & OFFSET_MASK) * 256) as usize;
    assert_eq!(
        (second_entry >> 52) as usize,
        second_span - second_packed.len()
    );
    assert!(
        bytes[second_start + second_packed.len()..second_start + second_span]
            .iter()
            .all(|&b| b == 0xa7)
    );

    let fixture = Fixture::new(&bytes);
    let mut store = fixture.open(2 * PAGE, DecodePolicy::LosslessOnly);

    let (data, quality) = store.read_range(0, 2 * PAGE).unwrap();
    assert_eq!(&data[..PAGE], first.as_slice());
    assert_eq!(&data[PAGE..], second.as_slice());
    assert_eq!(quality, vec![Quality::LosslessContainer; 2]);
    assert_eq!(
        store.stats().source_bytes,
        (first_packed.len() + second_packed.len()) as u64
    );
}

#[test]
fn a_leading_virtual_hole_is_an_error_not_zeroes() {
    // The table can describe a store whose first chunk starts above virtual
    // zero. Reads there must fail rather than fabricate zeroes.
    let mut bytes = build(&[page(0, vec![0x31; PAGE], 0)]);
    let first = entry(&bytes, 0);
    set_entry(&mut bytes, 0, patch_address(first, 1));
    let sentinel = entry(&bytes, 1);
    set_entry(&mut bytes, 1, patch_address(sentinel, 2));
    bytes[8..16].copy_from_slice(&((2 * PAGE as u64) | (22u64 << 56)).to_le_bytes());
    let fixture = Fixture::new(&bytes);
    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);

    assert_eq!(store.virtual_size(), 2 * PAGE as u64);
    assert_eq!(
        store.read_range(0, 16).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        store.read_chunk(0).unwrap().bytes.as_ref(),
        vec![0x31; PAGE].as_slice()
    );
    assert!(store.read_range(PAGE as u64, 16).is_ok());
}

// ---------------------------------------------------------------------------
// Cache and source-identity regressions
// ---------------------------------------------------------------------------

#[test]
fn caches_are_isolated_between_files_of_equal_size() {
    let a = build(&[page(0, vec![0x11; PAGE], 0)]);
    let b = build(&[page(0, vec![0x22; PAGE], 0)]);
    assert_eq!(a.len(), b.len(), "fixtures share a size on purpose");
    let first = Fixture::new(&a);
    let second = Fixture::new(&b);

    let mut left = first.open(PAGE, DecodePolicy::LosslessOnly);
    let mut right = second.open(PAGE, DecodePolicy::LosslessOnly);

    assert_eq!(left.read_chunk(0).unwrap().bytes.as_ref(), &[0x11; PAGE]);
    assert_eq!(right.read_chunk(0).unwrap().bytes.as_ref(), &[0x22; PAGE]);
    // Each session pays for its own decode: nothing leaks across files.
    assert_eq!(left.stats().decodes, 1);
    assert_eq!(right.stats().decodes, 1);
    assert_eq!(left.stats().cache_bytes, PAGE);
    assert_eq!(right.stats().cache_bytes, PAGE);

    assert_eq!(left.read_chunk(0).unwrap().bytes.as_ref(), &[0x11; PAGE]);
    assert_eq!(right.read_chunk(0).unwrap().bytes.as_ref(), &[0x22; PAGE]);
    assert_eq!(left.stats().cache_hits, 1);
    assert_eq!(right.stats().cache_hits, 1);
}

#[test]
fn same_size_in_place_update_is_detected_and_refresh_reloads() {
    let original = build(&[page(0, vec![0x11; PAGE], 0)]);
    let updated = build(&[page(0, vec![0x22; PAGE], 0)]);
    assert_eq!(original.len(), updated.len());
    let fixture = Fixture::new(&original);

    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    assert_eq!(store.read_chunk(0).unwrap().bytes.as_ref(), &[0x11; PAGE]);

    fixture.write_in_place(&updated);

    // The warm cache must not satisfy the read, and no mixed old/new bytes are
    // returned: the guard fails the whole operation and flushes the cache.
    let error = store.read_chunk(0).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(store.stats().cache_bytes, 0);
    assert_eq!(store.stats().decodes, 0);
    // Still stale until an explicit refresh.
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    store
        .refresh()
        .expect("refresh reparses the updated source");
    let chunk = store.read_chunk(0).unwrap();
    assert_eq!(chunk.bytes.as_ref(), &[0x22; PAGE]);
    assert_eq!(store.stats().decodes, 1);
    assert_eq!(store.stats().cache_hits, 0);
}

#[test]
fn pathname_replacement_is_detected_and_refresh_reloads() {
    let original = build(&[page(0, vec![0x11; PAGE], 0)]);
    let replacement = build(&[page(0, vec![0x22; PAGE], 0)]);
    assert_eq!(original.len(), replacement.len());
    let fixture = Fixture::new(&original);

    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    assert_eq!(store.read_chunk(0).unwrap().bytes.as_ref(), &[0x11; PAGE]);

    fixture.replace(&replacement);
    let error = store.read_chunk(0).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(store.stats().cache_bytes, 0);

    store.refresh().expect("refresh follows the replacement");
    assert_eq!(store.read_chunk(0).unwrap().bytes.as_ref(), &[0x22; PAGE]);

    // A replacement with a different layout is re-parsed rather than trusted.
    let smaller = build(&[page(0, vec![0x33; PAGE], 0)]);
    let two_pages = build(&[page(0, vec![0x44; PAGE], 0), page(0, vec![0x55; PAGE], 0)]);
    fixture.replace(&two_pages);
    assert!(store.read_chunk(0).is_err());
    store.refresh().expect("refresh reparses a larger layout");
    assert_eq!(store.chunks().len(), 2);
    assert_eq!(store.virtual_size(), 2 * PAGE as u64);
    assert_eq!(store.read_chunk(1).unwrap().bytes.as_ref(), &[0x55; PAGE]);

    fixture.replace(&smaller);
    assert!(store.read_chunk(0).is_err());
    store.refresh().expect("refresh reparses a smaller layout");
    assert_eq!(store.chunks().len(), 1);
    assert_eq!(store.virtual_size(), PAGE as u64);
    assert_eq!(
        store.read_chunk(1).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn refresh_discards_the_previous_generations_cache_and_counters() {
    let first_generation = build(&[page(0, vec![0x11; PAGE], 0), page(0, vec![0x12; PAGE], 0)]);
    let second_generation = build(&[page(0, vec![0x21; PAGE], 0), page(0, vec![0x22; PAGE], 0)]);
    assert_eq!(first_generation.len(), second_generation.len());
    let fixture = Fixture::new(&first_generation);

    let mut store = fixture.open(2 * PAGE, DecodePolicy::LosslessOnly);
    store.read_chunk(0).unwrap();
    store.read_chunk(1).unwrap();
    assert_eq!(store.stats().decodes, 2);
    assert_eq!(store.stats().cache_bytes, 2 * PAGE);

    fixture.replace(&second_generation);
    store.refresh().unwrap();

    // Counters belong to the new generation, and the decoded bytes of the old
    // generation are gone: a retained index-keyed entry would show up as a hit
    // serving 0x11 below.
    assert_eq!(store.stats().decodes, 0);
    assert_eq!(store.stats().cache_hits, 0);
    assert_eq!(store.stats().cache_bytes, 0);
    assert_eq!(store.stats().peak_cache_bytes, 0);

    assert_eq!(store.read_chunk(0).unwrap().bytes.as_ref(), &[0x21; PAGE]);
    assert_eq!(store.stats().cache_hits, 0);
    assert_eq!(store.stats().decodes, 1);
}

#[test]
fn failed_refresh_leaves_the_session_stale_never_stale_bytes() {
    let original = build(&[page(0, vec![0x11; PAGE], 0)]);
    let fixture = Fixture::new(&original);
    let mut store = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    store.read_chunk(0).unwrap();

    // Corrupt the sentinel so the reparse must fail.
    let mut corrupt = original.clone();
    let sentinel = entry(&corrupt, 1);
    set_entry(&mut corrupt, 1, patch_offset(sentinel, 49));
    fixture.replace(&corrupt);

    assert!(store.refresh().is_err());
    // The old generation's bytes must not be served after a failed refresh.
    assert_eq!(
        store.read_chunk(0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        store.read_range(0, 16).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    // Recovering the source makes refresh usable again.
    let recovered = build(&[page(0, vec![0x33; PAGE], 0)]);
    fixture.replace(&recovered);
    store.refresh().expect("refresh retries from disk");
    assert_eq!(store.read_chunk(0).unwrap().bytes.as_ref(), &[0x33; PAGE]);
}

#[test]
fn refresh_retains_budget_policy_and_limits() {
    const BLOCKS: usize = 512;
    let (stream, _) = bcpack::encode::synthetic_bc1(0xfeed_beef, BLOCKS, 32);
    let (other, _) = bcpack::encode::synthetic_bc1(0xfeed_beef, BLOCKS, 32);
    let original = build(&[page(1, stream, 0), page(1, other, 0)]);
    let fixture = Fixture::new(&original);

    let limits = Limits {
        max_range_bytes: 64,
        ..Limits::default()
    };
    let mut store = fixture
        .open_with_limits(PAGE, DecodePolicy::AllowFittedBc1, limits)
        .expect("BC1 store opens under a narrow range limit");

    // Policy survives the refresh: the fitted reconstruction is still usable.
    fixture.replace(&original);
    store.refresh().unwrap();
    let chunk = store.read_chunk(0).unwrap();
    assert!(matches!(chunk.quality, Quality::FittedBc1 { .. }));

    // The narrow limit survives the refresh.
    assert_eq!(
        store.read_range(0, 128).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    // The byte budget survives the refresh: two pages fit only one at a time.
    store.read_chunk(1).unwrap();
    assert_eq!(store.stats().cache_bytes, PAGE);
    assert!(store.stats().evictions >= 1);
    assert!(store.stats().peak_cache_bytes <= PAGE);
}

#[test]
fn read_batch_is_one_source_generation() {
    let original = build(&[page(0, vec![0x61; PAGE], 0), page(0, vec![0x62; PAGE], 0)]);
    let updated = build(&[page(0, vec![0x71; PAGE], 0), page(0, vec![0x72; PAGE], 0)]);
    assert_eq!(original.len(), updated.len());
    let fixture = Fixture::new(&original);
    let mut store = fixture.open(2 * PAGE, DecodePolicy::LosslessOnly);

    let ranges = [(0u64, 16usize), (PAGE as u64, 16)];
    let batch = store.read_batch(&ranges).unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].0.as_slice(), &[0x61; 16]);
    assert_eq!(batch[1].0.as_slice(), &[0x62; 16]);
    assert!(store.read_batch(&[]).unwrap().is_empty());

    // Out-of-bounds members fail the batch as a whole.
    assert!(store.read_batch(&[(0, 16), (2 * PAGE as u64, 1)]).is_err());
    assert!(store.read_batch(&[(1, 0)]).is_ok());

    fixture.write_in_place(&updated);
    let error = store.read_batch(&ranges).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // No partial generation is returned and the cache is flushed.
    assert_eq!(store.stats().cache_bytes, 0);

    store.refresh().unwrap();
    let batch = store.read_batch(&ranges).unwrap();
    assert_eq!(batch[0].0.as_slice(), &[0x71; 16]);
    assert_eq!(batch[1].0.as_slice(), &[0x72; 16]);
}

#[test]
fn read_batch_caps_distinct_decoded_work() {
    let bytes = build(&[page(0, vec![0x91; PAGE], 0), page(0, vec![0x92; PAGE], 0)]);
    let fixture = Fixture::new(&bytes);
    let mut store = fixture
        .open_with_limits(
            0,
            DecodePolicy::LosslessOnly,
            Limits {
                max_range_bytes: 5000,
                ..Limits::default()
            },
        )
        .unwrap();

    // One page of decoded work fits under the cap.
    assert_eq!(store.read_batch(&[(0, 16)]).unwrap().len(), 1);

    // 32 requested bytes pass the requested-byte cap, but the two distinct
    // chunks cost 8192 decoded bytes, so the whole batch is refused.
    assert_eq!(
        store
            .read_batch(&[(0, 16), (PAGE as u64, 16)])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );

    // Repeating a range must not double-count its chunk.
    assert_eq!(store.read_batch(&[(0, 16), (0, 16)]).unwrap().len(), 2);

    // Zero-length ranges touch no decoded work.
    assert!(
        store
            .read_batch(&[(0, 0), (PAGE as u64, 0)])
            .unwrap()
            .iter()
            .all(|r| r.0.is_empty())
    );

    // The boundary is inclusive: two pages of work pass a two-page cap.
    let mut wide = fixture
        .open_with_limits(
            0,
            DecodePolicy::LosslessOnly,
            Limits {
                max_range_bytes: 2 * PAGE,
                ..Limits::default()
            },
        )
        .unwrap();
    assert_eq!(
        wide.read_batch(&[(0, 16), (PAGE as u64, 16)])
            .unwrap()
            .len(),
        2
    );
}

// ---------------------------------------------------------------------------
// Catalog descriptors and extraction: metadata is generation-scoped too
// ---------------------------------------------------------------------------

/// Write the minimal ETPF catalog the reader expects at the rounded metadata
/// end: header (`ETPF`, 634, count, area), 16-byte records, then 8-byte keys.
fn write_catalog(bytes: &mut [u8], record: [u32; 4], key: u64) {
    let at: usize = 256; // rounded end of the two-entry table
    bytes[at..at + 4].copy_from_slice(b"ETPF");
    bytes[at + 4..at + 8].copy_from_slice(&634u32.to_le_bytes());
    bytes[at + 8..at + 12].copy_from_slice(&1u32.to_le_bytes());
    bytes[at + 12..at + 16].copy_from_slice(&24u32.to_le_bytes());
    for (i, word) in record.iter().enumerate() {
        bytes[at + 16 + i * 4..at + 20 + i * 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes[at + 32..at + 40].copy_from_slice(&key.to_le_bytes());
}

#[test]
fn catalog_descriptors_are_scoped_to_their_source_generation() {
    const BLOCKS: usize = 512;
    let (stream, _) = bcpack::encode::synthetic_bc1(0x00c0_ffee, BLOCKS, 32);
    let mut bytes = build(&[page(1, stream, 0)]);

    // 64x64 BC1_UNORM, one mip, dimension 1, descriptor mode 2 (4 KiB tile),
    // base 0. `mips = (a >> 28) + (b & 7)`, so the high nibble carries 1 and
    // `non_tail` stays 0.
    let a = 63u32 | (63u32 << 14) | (1u32 << 28);
    let b = (71u32 << 3) | (1u32 << 21) | (2u32 << 23);
    write_catalog(&mut bytes, [a, b, 0, 0], 0x1234_5678_9abc_def0);
    let fixture = Fixture::new(&bytes);

    let mut store = fixture.open(PAGE, DecodePolicy::AllowFittedBc1);
    let catalog = store.textures().expect("synthetic catalog parses");
    assert_eq!(catalog.len(), 1);
    let texture = &catalog[0];
    assert_eq!(
        (texture.width, texture.height, texture.mips, texture.dxgi),
        (64, 64, 1, 71)
    );
    assert_eq!(texture.asset_key, Some(0x1234_5678_9abc_def0));

    // Extraction keeps the fitted provenance and writes a DX10 DDS whose
    // payload is exactly the visible BC1 block grid.
    let extracted = store.texture_dds(texture).expect("extract BC1 texture");
    assert_eq!(&extracted.dds[..4], b"DDS ");
    assert_eq!(&extracted.dds[84..88], b"DX10");
    assert_eq!(extracted.dds.len(), 148 + (64 / 4) * (64 / 4) * 8);
    assert_eq!(extracted.quality.len(), 1);
    assert!(matches!(extracted.quality[0], Quality::FittedBc1 { .. }));

    // The default policy cannot extract reconstructed pixels.
    let mut strict = fixture.open(PAGE, DecodePolicy::LosslessOnly);
    let strict_texture = strict.textures().unwrap().pop().unwrap();
    assert_eq!(
        strict.texture_dds(&strict_texture).err().map(|e| e.kind()),
        Some(io::ErrorKind::Unsupported)
    );

    // A descriptor from the previous source generation must be refused even
    // though the bytes at the path would still parse.
    // Touch only the metadata gap: the replacement must stay a *decodable*
    // store so the refusal below can only be the generation guard.
    let mut replacement = bytes.clone();
    replacement[100] ^= 0x80;
    fixture.replace(&replacement);
    store.refresh().expect("refresh adopts the replacement");
    assert_eq!(
        store.texture_dds(texture).err().map(|e| e.kind()),
        Some(io::ErrorKind::InvalidData)
    );
    let fresh = store.textures().unwrap();
    assert!(store.texture_dds(&fresh[0]).is_ok());
}
