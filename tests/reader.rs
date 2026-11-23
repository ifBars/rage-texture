use rage_texture::{DecodePolicy, Limits, Quality, Store};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static ID: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rage-test-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("synthetic.ttpf");
        fs::write(&path, bytes).unwrap();
        Self(path)
    }
    fn open(&self, budget: usize) -> Store {
        Store::open(&self.0, budget, DecodePolicy::LosslessOnly).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_file(&self.0).unwrap();
        fs::remove_dir(self.0.parent().unwrap()).unwrap();
    }
}
fn fixture(codecs: &[u8]) -> Vec<u8> {
    let mut data = vec![0u8; 4096];
    data[..4].copy_from_slice(b"RDS5");
    data[4..8].copy_from_slice(&((codecs.len() + 2) as u32).to_le_bytes());
    let size = ((codecs.len() + 1) * 4096) as u64;
    data[8..16].copy_from_slice(&(size | (22u64 << 56)).to_le_bytes());
    for (i, &codec) in codecs.iter().enumerate() {
        let raw = vec![(i + 1) as u8; 4096];
        let packed = if codec == 8 {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(&raw).unwrap();
            e.finish().unwrap()
        } else {
            raw
        };
        let padded = packed.len().div_ceil(256) * 256;
        let pad = padded - packed.len();
        let entry = (data.len() as u64 / 256)
            | ((codec as u64) << 26)
            | (((i + 1) as u64) << 30)
            | ((pad as u64) << 52);
        data[24 + i * 8..32 + i * 8].copy_from_slice(&entry.to_le_bytes());
        data.extend_from_slice(&packed);
        data.resize(data.len() + pad, 0);
    }
    let entry = (data.len() as u64 / 256) | (((codecs.len() + 1) as u64) << 30);
    let at = 24 + codecs.len() * 8;
    data[at..at + 8].copy_from_slice(&entry.to_le_bytes());
    data
}
#[test]
fn stored_and_zlib_cross_chunk() {
    let f = Fixture::new(&fixture(&[0, 8]));
    let mut s = f.open(8192);
    let (b, q) = s.read_range(4096 + 4000, 200).unwrap();
    assert_eq!(&b[..96], &[1; 96]);
    assert_eq!(&b[96..], &[2; 104]);
    assert_eq!(q, vec![Quality::LosslessContainer; 2]);
}
#[test]
fn exact_lru_budget_and_arc_survives_eviction() {
    let f = Fixture::new(&fixture(&[8, 8, 8]));
    let mut s = f.open(8192);
    let held = s.read_chunk(0).unwrap();
    s.read_chunk(1).unwrap();
    s.read_chunk(0).unwrap();
    s.read_chunk(2).unwrap();
    assert_eq!(s.stats().cache_hits, 1);
    assert_eq!(s.stats().evictions, 1);
    assert_eq!(s.stats().cache_bytes, 8192);
    s.read_chunk(0).unwrap();
    assert_eq!(s.stats().cache_hits, 2);
    s.read_chunk(1).unwrap();
    assert_eq!(s.stats().decodes, 4);
    assert_eq!(held.bytes.as_ref(), &[1; 4096]);
    assert!(s.stats().peak_cache_bytes <= 8192);
    s.reset();
    assert_eq!(s.stats().decodes, 0);
    assert_eq!(s.stats().cache_bytes, 0);
}
#[test]
fn no_cache_and_too_small_cache() {
    let f = Fixture::new(&fixture(&[8]));
    for budget in [0, 4095] {
        let mut s = f.open(budget);
        s.read_chunk(0).unwrap();
        s.read_chunk(0).unwrap();
        assert_eq!(s.stats().decodes, 2);
        assert_eq!(s.stats().cache_bytes, 0);
    }
}
#[test]
fn malformed_sources_return_errors() {
    let base = fixture(&[8]);
    for cut in [0, 4, 23, 40, base.len() - 1] {
        let f = Fixture::new(&base[..cut]);
        assert!(Store::open(&f.0, 0, DecodePolicy::LosslessOnly).is_err());
    }
    for byte in [0, 4, 15, 32] {
        let mut b = base.clone();
        b[byte] ^= 255;
        let f = Fixture::new(&b);
        assert!(Store::open(&f.0, 0, DecodePolicy::LosslessOnly).is_err());
    }
}
#[test]
fn bounds_and_limits() {
    let f = Fixture::new(&fixture(&[8]));
    let mut s = f.open(0);
    assert!(s.read_range(0, 1).is_err());
    assert!(s.read_range(u64::MAX, 1).is_err());
    assert!(s.read_range(8191, 2).is_err());
    assert!(s.read_chunk(1).is_err());
    assert!(s.read_range(8192, 0).unwrap().0.is_empty());
    assert!(
        Store::open_with_limits(
            &f.0,
            0,
            DecodePolicy::LosslessOnly,
            Limits {
                max_chunk_bytes: 4095,
                ..Limits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn unsupported_is_never_zero_filled() {
    let f = Fixture::new(&fixture(&[7]));
    let mut s = f.open(4096);
    assert_eq!(
        s.read_chunk(0).unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(s.stats().cache_bytes, 0);
}
#[test]
fn truncated_zlib_checksum_and_wrong_extent_are_rejected() {
    let base = fixture(&[8]);
    let entry = u64::from_le_bytes(base[24..32].try_into().unwrap());
    let pad = (entry >> 52) as usize;
    let end = base.len() - pad;
    let mut b = base.clone();
    b[end - 1] ^= 255;
    let f = Fixture::new(&b);
    assert!(f.open(4096).read_chunk(0).is_err());
    let mut b = base.clone();
    let truncated = (entry & ((1u64 << 52) - 1)) | (((pad + 2) as u64) << 52);
    b[24..32].copy_from_slice(&truncated.to_le_bytes());
    let f = Fixture::new(&b);
    assert!(f.open(4096).read_chunk(0).is_err());
    // Inflate 8192 bytes into a declared 4096-byte chunk: refuse, do not grow without bound.
    let mut b = base;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(&vec![7; 8192]).unwrap();
    let packed = e.finish().unwrap();
    b[4096..4096 + packed.len()].copy_from_slice(&packed);
    let newpad = b.len() - 4096 - packed.len();
    let en = (entry & ((1u64 << 52) - 1)) | ((newpad as u64) << 52);
    b[24..32].copy_from_slice(&en.to_le_bytes());
    let f = Fixture::new(&b);
    assert!(f.open(4096).read_chunk(0).is_err());
}
#[test]
fn nonzero_padding_can_follow_sentinel_without_zero_entry() {
    let mut b = fixture(&[8]);
    // Arbitrary table padding, not the aligned ETPF catalog header.
    b[40..48].copy_from_slice(b"padding!");
    let f = Fixture::new(&b);
    assert_eq!(f.open(0).read_chunk(0).unwrap().bytes.len(), 4096);
}
