//! Uncached extraction regressions (owned by the uncached/performance task).
//!
//! The performance gate now requires extraction with the decoded cache
//! **disabled in both implementations**. These tests prove the property from the
//! outside, on synthetic stores, with the `Store` budget fixed at zero:
//!
//! * a 3D volume's storage region is decoded **once per mip**, not once per Z
//!   slice (the pre-optimization `texture_dds` re-read the same region for every
//!   Z);
//! * a multi-mip 2D chain whose small mips share one tail tile is decoded
//!   **once** through the bounded one-call input span, and again correctly
//!   through the per-region fallback when the span exceeds `max_range_bytes`;
//! * no path records a cache hit or any resident bytes, so no part of a timing
//!   can be cache-assisted;
//! * both paths are byte-identical to detiling every mip/slice through
//!   `texture_mip` and assembling the subresources with `dds::write_dds`, which
//!   is the independent reference path. The fallback therefore loses no
//!   coverage relative to the bounded-input call.
//!
//! Fixtures are synthetic RDS5 stores with an embedded ETPF catalog, built from
//! patterned bytes and written to a per-test temporary directory. No game data
//! is read; decoded bytes never leave the test process.
use rage_texture::{
    DecodePolicy, Limits, Quality, Store, dds, layout,
    layout::{SurfaceKind, TextureShape},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const PAGE: usize = 4096;
/// 16x8x32 RGBA8: one 4 KiB-tile volume of four storage pages.
const WIDTH: u32 = 16;
const HEIGHT: u32 = 8;
const DEPTH: u32 = 32;
const DXGI_R8G8B8A8_UNORM: u32 = 28;
const BYTES_PER_TEXEL: usize = 4;

static ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    dir: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rage-uncached-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("volume.ttpf");
        fs::write(&path, bytes).unwrap();
        Self { dir, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

fn set_entry(bytes: &mut [u8], index: usize, value: u64) {
    bytes[24 + index * 8..32 + index * 8].copy_from_slice(&value.to_le_bytes());
}

/// Deterministic, non-repeating page contents so a Z slice cannot accidentally
/// equal its neighbour and a per-Z re-read would be visible in the statistics.
fn page_bytes(index: usize) -> Vec<u8> {
    (0..PAGE)
        .map(|i| ((i * 31 + index * 97 + 7) % 251) as u8)
        .collect()
}

/// Minimal RDS5 store: `chunks` stored (codec 0) pages, contiguous in both
/// address spaces, with the sentinel reproducing file length and virtual size.
fn build_store(chunks: usize, catalog: [u32; 4], key: u64) -> Vec<u8> {
    assert!(chunks > 0);
    let entries = chunks + 1;
    let metadata_end = (24 + entries * 8).div_ceil(256) * 256;
    let data_start = (metadata_end + 256).max(PAGE);
    let virtual_size = (chunks * PAGE) as u64;

    let mut bytes = vec![0u8; data_start];
    bytes[..4].copy_from_slice(b"RDS5");
    bytes[4..8].copy_from_slice(&((chunks + 2) as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&(virtual_size | (22u64 << 56)).to_le_bytes());
    for i in 0..chunks {
        set_entry(
            &mut bytes,
            i,
            ((data_start + i * PAGE) as u64 / 256) | ((i as u64) << 30),
        );
        bytes.extend_from_slice(&page_bytes(i));
    }
    set_entry(
        &mut bytes,
        chunks,
        ((data_start + chunks * PAGE) as u64 / 256) | ((virtual_size / PAGE as u64) << 30),
    );

    // ETPF header at the 256-aligned metadata end, then one 16-byte record and
    // its 8-byte key (area = 24). The header and area stay below data_start.
    let at: usize = 256;
    assert!(at + 16 + 24 <= data_start);
    bytes[at..at + 4].copy_from_slice(b"ETPF");
    bytes[at + 4..at + 8].copy_from_slice(&634u32.to_le_bytes());
    bytes[at + 8..at + 12].copy_from_slice(&1u32.to_le_bytes());
    bytes[at + 12..at + 16].copy_from_slice(&24u32.to_le_bytes());
    for (i, word) in catalog.iter().enumerate() {
        bytes[at + 16 + i * 4..at + 20 + i * 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes[at + 32..at + 40].copy_from_slice(&key.to_le_bytes());
    bytes
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn uncached_volume_dds_decodes_storage_region_once_not_per_z() {
    let shape = TextureShape {
        width: WIDTH,
        height: HEIGHT,
        depth: DEPTH,
        mips: 1,
        dxgi: DXGI_R8G8B8A8_UNORM,
        dimension: 3,
        mode: 2,
        non_tail: 0,
    };
    let expected = layout::layout(&shape).expect("single-mip 4 KiB-tile volume layout");
    assert_eq!(expected.kind, SurfaceKind::Volume3D);
    let region = expected.mips[0].region;
    assert_eq!(region, expected.total_bytes);
    assert!(
        region.is_multiple_of(PAGE as u64),
        "the fixture assumes whole storage pages"
    );
    let chunks = usize::try_from(region / PAGE as u64).unwrap();
    assert_eq!(chunks, 4, "16x8x32 RGBA8 is four 4 KiB volume tiles");

    // Descriptor fields per the ETPF catalog: width/height in `a`, the high
    // mips nibble carries `mips - non_tail`, and `b` holds dxgi, depth,
    // dimension and mode. base 0, allocation_end == virtual_size.
    let a = (WIDTH - 1) | ((HEIGHT - 1) << 14) | (1u32 << 28);
    let b = (DXGI_R8G8B8A8_UNORM << 3) | ((DEPTH - 1) << 10) | (3u32 << 21) | (2u32 << 23);
    let bytes = build_store(chunks, [a, b, 0, 0], 0xfeed_face_dead_beef);
    let fixture = Fixture::new(&bytes);

    // Budget zero: nothing may be retained or reused. The store layout must
    // match the computed allocation span, or texture_layout refuses it.
    let mut store = Store::open(fixture.path(), 0, DecodePolicy::LosslessOnly)
        .expect("synthetic volume store opens");
    assert_eq!(store.virtual_size(), expected.total_bytes);
    assert_eq!(store.chunks().len(), chunks);

    let catalog = store.textures().expect("synthetic catalog parses");
    assert_eq!(catalog.len(), 1);
    let texture = &catalog[0];
    assert_eq!(
        (
            texture.width,
            texture.height,
            texture.depth,
            texture.mips,
            texture.dxgi,
            texture.dimension,
            texture.mode,
        ),
        (WIDTH, HEIGHT, DEPTH, 1, DXGI_R8G8B8A8_UNORM, 3, 2)
    );

    let extracted = store
        .texture_dds(texture)
        .expect("uncached volume DDS extraction");

    // The whole storage region was decoded exactly once: one decode per
    // contributing chunk, one read of `region` source bytes. The old per-Z
    // path decoded DEPTH times as much.
    let stats = store.stats();
    assert_eq!(stats.decodes, chunks as u64);
    assert_eq!(stats.source_bytes, region);
    assert_eq!(stats.cache_hits, 0);
    assert_eq!(stats.cache_bytes, 0);
    assert_eq!(stats.peak_cache_bytes, 0);
    assert_eq!(stats.evictions, 0);
    // One provenance entry per contributing chunk, not one per Z slice.
    assert_eq!(extracted.quality.len(), chunks);
    assert!(
        extracted
            .quality
            .iter()
            .all(|q| *q == Quality::LosslessContainer)
    );

    // DDS shape: volume depth and the DX10 3D dimension, payload is the whole
    // visible volume.
    assert_eq!(&extracted.dds[..4], b"DDS ");
    assert_eq!(&extracted.dds[84..88], b"DX10");
    assert_eq!(u32_at(&extracted.dds, 24), DEPTH);
    assert_eq!(u32_at(&extracted.dds, 132), 4);
    assert_eq!(
        extracted.dds.len(),
        148 + (WIDTH * HEIGHT * DEPTH * BYTES_PER_TEXEL as u32) as usize
    );
    let slice = (WIDTH * HEIGHT * BYTES_PER_TEXEL as u32) as usize;
    let payload = &extracted.dds[148..];
    assert_ne!(&payload[..slice], &payload[slice..2 * slice]);
    assert!(payload.windows(2).any(|w| w[0] != w[1]));

    // Reference path: detile every Z slice through the public per-slice API and
    // assemble the volume subresource with the DDS writer.
    store.reset();
    let mut joined = Vec::with_capacity(region as usize);
    let mut reference_quality = Vec::new();
    for z in 0..DEPTH {
        let (linear, quality) = store.texture_mip(texture, 0, z).expect("per-Z detile");
        assert_eq!(linear.len(), slice);
        joined.extend_from_slice(&linear);
        reference_quality.extend(quality);
    }
    assert_eq!(joined.len(), region as usize);
    assert_eq!(&joined[..slice], &payload[..slice]);
    let reference = dds::write_dds(&texture.shape(), &[joined.clone()])
        .expect("reference DDS from joined Z slices");
    assert_eq!(reference, extracted.dds);
    assert_eq!(&reference[148..], joined.as_slice());

    // The reference path itself is uncached, and it shows the cost the
    // optimized path avoids: one region read per Z.
    let reference_stats = store.stats();
    assert_eq!(reference_stats.decodes, DEPTH as u64 * chunks as u64);
    assert_eq!(reference_stats.source_bytes, region * DEPTH as u64);
    assert_eq!(reference_stats.cache_hits, 0);
    assert_eq!(reference_stats.cache_bytes, 0);
    assert_eq!(reference_stats.peak_cache_bytes, 0);
    assert_eq!(
        reference_quality.len(),
        DEPTH as usize * chunks,
        "per-Z reads duplicate provenance per slice"
    );
}

#[test]
fn uncached_multimip_shared_tail_span_and_fallback_match_per_mip_reference() {
    // 64x64 RGBA8 with a full mip chain: mips 2..6 share one 4 KiB tail tile at
    // offset 0, mip 1 and mip 0 are standalone tiles. The complete input span is
    // therefore the whole 24 KiB allocation, while the largest single region is
    // 16 KiB — which lets one store take the bounded one-call branch and the
    // other the per-region fallback.
    const WIDTH2D: u32 = 64;
    const HEIGHT2D: u32 = 64;
    const MIPS: u32 = 7;
    let shape = TextureShape {
        width: WIDTH2D,
        height: HEIGHT2D,
        depth: 1,
        mips: MIPS,
        dxgi: DXGI_R8G8B8A8_UNORM,
        dimension: 1,
        mode: 2,
        non_tail: 0,
    };
    let expected = layout::layout(&shape).expect("compact 4 KiB-tile mip chain");
    assert_eq!(expected.kind, SurfaceKind::Texture2D);
    assert_eq!(expected.mips.len(), MIPS as usize);
    assert_eq!(expected.first_tail, 2, "mips 2..6 must share the tail tile");
    for mip in 2..MIPS as usize {
        assert!(expected.mips[mip].in_tail);
        assert_eq!(expected.mips[mip].offset, 0);
        assert_eq!(expected.mips[mip].region, PAGE as u64);
    }
    let span_start = expected.mips.iter().map(|m| m.offset).min().expect("mips");
    let span_end = expected
        .mips
        .iter()
        .map(|m| m.offset + m.region)
        .max()
        .expect("mips");
    let input_span = span_end - span_start;
    let largest_region = expected.mips.iter().map(|m| m.region).max().unwrap();
    assert_eq!(span_start, 0);
    assert_eq!(input_span, expected.total_bytes);
    assert_eq!(input_span, 6 * PAGE as u64);
    assert_eq!(largest_region, 16 * 1024);
    assert!(
        input_span > largest_region,
        "the fallback limit must be below the complete span"
    );

    let chunks = usize::try_from(expected.total_bytes / PAGE as u64).unwrap();
    let a = (WIDTH2D - 1) | ((HEIGHT2D - 1) << 14) | (MIPS << 28);
    let b = (DXGI_R8G8B8A8_UNORM << 3) | (1u32 << 21) | (2u32 << 23);
    let bytes = build_store(chunks, [a, b, 0, 0], 0x0123_4567_89ab_cdef);
    let fixture = Fixture::new(&bytes);

    // Branch 1: default limits, so the complete input span is one read.
    let mut bounded = Store::open(fixture.path(), 0, DecodePolicy::LosslessOnly).unwrap();
    assert_eq!(bounded.virtual_size(), expected.total_bytes);
    let catalog = bounded.textures().expect("catalog parses");
    let texture = &catalog[0];
    assert_eq!(texture.mips, MIPS);
    let extracted = bounded
        .texture_dds(texture)
        .expect("bounded span extraction");
    let stats = bounded.stats();
    assert_eq!(
        stats.decodes, chunks as u64,
        "each storage chunk is decoded exactly once for the whole texture"
    );
    assert_eq!(stats.source_bytes, input_span);
    assert_eq!(stats.cache_hits, 0);
    assert_eq!(stats.cache_bytes, 0);
    assert_eq!(stats.peak_cache_bytes, 0);
    assert_eq!(stats.evictions, 0);
    // One provenance entry per chunk in the single covering read.
    assert_eq!(extracted.quality.len(), chunks);
    assert!(
        extracted
            .quality
            .iter()
            .all(|q| *q == Quality::LosslessContainer)
    );

    let mut payload_len = 0usize;
    for mip in 0..MIPS {
        payload_len += expected.subresource_len(mip).unwrap();
    }
    assert_eq!(extracted.dds.len(), 148 + payload_len);
    assert_eq!(&extracted.dds[..4], b"DDS ");
    assert_eq!(&extracted.dds[84..88], b"DX10");
    assert_eq!(u32_at(&extracted.dds, 28), MIPS);

    // Independent reference: every visible mip through the per-slice API, then
    // the DDS writer. Same store, counters reset, still budget zero.
    bounded.reset();
    let mut subresources = Vec::new();
    let mut reference_quality = Vec::new();
    for mip in 0..MIPS {
        let (linear, quality) = bounded.texture_mip(texture, mip, 0).unwrap();
        assert_eq!(linear.len(), expected.subresource_len(mip).unwrap());
        subresources.push(linear);
        reference_quality.extend(quality);
    }
    let reference = dds::write_dds(&texture.shape(), &subresources).unwrap();
    assert_eq!(reference, extracted.dds);
    let mut joined = Vec::new();
    for sub in &subresources {
        joined.extend_from_slice(sub);
    }
    assert_eq!(&extracted.dds[148..], joined.as_slice());
    // The per-mip reference re-decodes the shared tail chunk once per tail mip.
    let reference_stats = bounded.stats();
    assert_eq!(reference_stats.decodes, 10);
    assert_eq!(
        reference_stats.source_bytes,
        5 * PAGE as u64 + 16 * 1024 + PAGE as u64
    );
    assert_eq!(reference_stats.cache_hits, 0);
    assert_eq!(reference_stats.cache_bytes, 0);
    assert_eq!(reference_stats.peak_cache_bytes, 0);
    assert_eq!(reference_quality.len(), 10);

    // Branch 2: the span (24 KiB) exceeds this limit (16 KiB = the largest
    // single region), so extraction must take the per-region fallback. Output
    // must be identical: the bounded branch is an optimization, not a coverage
    // change.
    let mut fallback = Store::open_with_limits(
        fixture.path(),
        0,
        DecodePolicy::LosslessOnly,
        Limits {
            max_range_bytes: 16 * 1024,
            ..Limits::default()
        },
    )
    .unwrap();
    let fallback_catalog = fallback.textures().expect("catalog parses");
    let fallback_texture = &fallback_catalog[0];
    let fallback_dds = fallback
        .texture_dds(fallback_texture)
        .expect("per-region fallback extraction");
    assert_eq!(fallback_dds.dds, extracted.dds);
    let fallback_stats = fallback.stats();
    assert_eq!(
        fallback_stats.decodes, 10,
        "fallback proves it re-read the shared tail per mip"
    );
    assert!(fallback_stats.decodes > stats.decodes);
    assert_eq!(
        fallback_stats.source_bytes,
        5 * PAGE as u64 + 16 * 1024 + PAGE as u64
    );
    assert_eq!(fallback_stats.cache_hits, 0);
    assert_eq!(fallback_stats.cache_bytes, 0);
    assert_eq!(fallback_stats.peak_cache_bytes, 0);
    assert_eq!(fallback_stats.evictions, 0);
    assert_eq!(fallback_dds.quality.len(), 10);
    assert!(
        fallback_dds
            .quality
            .iter()
            .all(|q| *q == Quality::LosslessContainer)
    );
}
