//! Read-only RDS5 chunk access. No game content or decryption is included.
//! BC1 reconstruction is supplied by bcpack-rs by Ahmet Dedeler:
//! <https://github.com/Ahmet-Dedeler/bcpack-rs> (MIT OR Apache-2.0).
use serde::Serialize;
pub mod bc6h;
pub mod catalog;
pub mod dds;
pub mod extract;
pub mod history;
mod history_bc;
mod history_bptc;
pub use history::HistoryTables;
pub mod layout;
pub mod output;
pub mod preview;
mod source;
// Attributed SixNG codecs; keep upstream formatting separate from our code.
#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
mod bc3pack;
#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
mod bc45pack;
#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
mod bc6pack;
#[allow(dead_code, clippy::all)]
#[rustfmt::skip]
mod bc7pack;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Limits apply before allocation, including when the cache is disabled.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_chunks: usize,
    pub max_chunk_bytes: usize,
    pub max_range_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_chunks: 1_000_000,
            max_chunk_bytes: 16 * 1024 * 1024,
            max_range_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Chunk {
    pub file_offset: u64,
    pub packed_bytes: usize,
    pub virtual_offset: u64,
    pub decoded_bytes: usize,
    pub codec: u8,
}
impl Chunk {
    pub fn supported(&self) -> bool {
        matches!(self.codec, 0 | 1 | 3..=8)
    }
}

/// Provenance, not a pixel-fidelity certificate. BC1's upstream tables are fitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Quality {
    LosslessContainer,
    FittedBc1 {
        unfitted_blocks: usize,
    },
    FittedBc {
        codec: u8,
        unfitted_blocks: usize,
        invalid_blocks: usize,
    },
    /// Reconstruction using the immutable history snapshot identified by its
    /// digest. Zero provisional dependencies means all consumed history bits
    /// were confirmed by those tables, not an independent pixel oracle.
    HistoryBc {
        codec: u8,
        provisional_dependencies: usize,
        history_digest: [u8; 32],
    },
}
#[derive(Clone, Debug)]
pub struct DecodedChunk {
    pub bytes: Arc<[u8]>,
    pub quality: Quality,
}
/// Strictly refuses the fitted BC1 path unless the caller explicitly selects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodePolicy {
    LosslessOnly,
    AllowFittedBc1,
    AllowFittedBc,
    /// Raw/zlib plus BCPack whose consumed history bits are all confirmed.
    ConfirmedOnly,
    /// Also allow known provisional history, with explicit provenance. Unknown
    /// history still fails rather than inventing placeholder data.
    AllowProvisional,
}
#[derive(Default, Clone, Copy, Debug, Serialize)]
pub struct Stats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decodes: u64,
    pub source_bytes: u64,
    pub cache_bytes: usize,
    pub peak_cache_bytes: usize,
    pub evictions: u64,
}
struct Entry {
    value: DecodedChunk,
    touched: u64,
}
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A session owns its file, immutable decode policy, and byte-bounded LRU cache.
/// Arc handles returned to callers can outlive eviction; their memory is caller-owned.
pub struct Store {
    file: File,
    source: source::Source,
    stale: bool,
    generation: u64,
    chunks: Vec<Chunk>,
    virtual_size: u64,
    limits: Limits,
    budget: usize,
    policy: DecodePolicy,
    cache: HashMap<usize, Entry>,
    recency: BTreeMap<u64, usize>,
    clock: u64,
    stats: Stats,
    tables: Option<(bcpack::bc1::Window, bcpack::bc1::QuadTable)>,
    history: Option<Arc<HistoryTables>>,
}
impl Store {
    pub fn open(path: impl AsRef<Path>, budget: usize, policy: DecodePolicy) -> io::Result<Self> {
        Self::open_with_limits(path, budget, policy, Limits::default())
    }
    pub fn open_with_limits(
        path: impl AsRef<Path>,
        budget: usize,
        policy: DecodePolicy,
        limits: Limits,
    ) -> io::Result<Self> {
        let history = matches!(
            policy,
            DecodePolicy::ConfirmedOnly | DecodePolicy::AllowProvisional
        )
        .then(HistoryTables::builtin);
        Self::open_configured(path.as_ref(), budget, policy, limits, history)
    }
    pub fn open_with_history(
        path: impl AsRef<Path>,
        budget: usize,
        policy: DecodePolicy,
        history: Arc<HistoryTables>,
    ) -> io::Result<Self> {
        Self::open_with_limits_and_history(path, budget, policy, Limits::default(), history)
    }
    pub fn open_with_limits_and_history(
        path: impl AsRef<Path>,
        budget: usize,
        policy: DecodePolicy,
        limits: Limits,
        history: Arc<HistoryTables>,
    ) -> io::Result<Self> {
        if !matches!(
            policy,
            DecodePolicy::ConfirmedOnly | DecodePolicy::AllowProvisional
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "custom history requires ConfirmedOnly or AllowProvisional policy",
            ));
        }
        Self::open_configured(path.as_ref(), budget, policy, limits, Some(history))
    }
    fn open_configured(
        path: &Path,
        budget: usize,
        policy: DecodePolicy,
        limits: Limits,
        history: Option<Arc<HistoryTables>>,
    ) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let source = source::Source::capture(path, &file)?;
        let file_len = file.metadata()?.len();
        let mut header = [0u8; 24];
        file.read_exact(&mut header)?;
        if &header[..4] != b"RDS5" {
            return Err(invalid("not an RDS5 store"));
        }
        let count = word(&header, 4) as usize;
        let size_word = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let virtual_size = size_word & 0x00ff_ffff_ffff_ffff;
        if size_word >> 56 != 22
            || virtual_size < 4096
            || count < 3
            || count - 2 > limits.max_chunks
        {
            return Err(invalid("unsupported RDS5 header or chunk limit"));
        }
        // The table has count-1 entries, including the end sentinel. ETPF may
        // start immediately afterwards; those bytes are not another table entry.
        let bytes = (count - 1)
            .checked_mul(8)
            .ok_or_else(|| invalid("table overflow"))?;
        if bytes as u64 > file_len.saturating_sub(24) {
            return Err(invalid("truncated chunk table"));
        }
        let mut table = vec![0; bytes];
        file.read_exact(&mut table)?;
        let entry = |i: usize| u64::from_le_bytes(table[i * 8..i * 8 + 8].try_into().unwrap());
        let offset = |v: u64| (v & 0x03ff_ffff) * 256;
        let address = |v: u64| ((v >> 30) & 0x3f_ffff) * 4096;
        let sentinel = entry(count - 2);
        if offset(sentinel) != file_len || address(sentinel) != virtual_size {
            return Err(invalid("RDS5 sentinel disagrees with source lengths"));
        }
        let metadata_end = (24 + (count - 1) * 8).div_ceil(256) * 256;
        let mut chunks = Vec::with_capacity(count - 2);
        for i in 0..count - 2 {
            let v = entry(i);
            let next = entry(i + 1);
            let start = offset(v);
            let end = offset(next);
            let a = address(v);
            let b = address(next);
            let pad = v >> 52;
            let packed = end
                .checked_sub(start)
                .and_then(|n| n.checked_sub(pad))
                .ok_or_else(|| invalid("reversed chunk or excessive padding"))?;
            let decoded = b
                .checked_sub(a)
                .ok_or_else(|| invalid("reversed virtual chunk"))?;
            let codec = ((v >> 26) & 15) as u8;
            // ETPF's 16-byte header lives at metadata_end. Its reserved virtual
            // metadata prefix is deliberately not exposed as decoded chunk data.
            if start < metadata_end as u64 + 16
                || end > file_len
                || packed == 0
                || decoded == 0
                || b > virtual_size
                || packed > limits.max_chunk_bytes as u64
                || decoded > limits.max_chunk_bytes as u64
                || (codec == 0 && packed != decoded)
            {
                return Err(invalid("chunk range or allocation limit invalid"));
            }
            chunks.push(Chunk {
                file_offset: start,
                packed_bytes: packed as usize,
                virtual_offset: a,
                decoded_bytes: decoded as usize,
                codec,
            });
        }
        source.validate(&file)?;
        Ok(Self {
            file,
            source,
            stale: false,
            generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            chunks,
            virtual_size,
            limits,
            budget,
            policy,
            cache: HashMap::new(),
            recency: BTreeMap::new(),
            clock: 0,
            stats: Stats::default(),
            tables: None,
            history,
        })
    }
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
    pub fn virtual_size(&self) -> u64 {
        self.virtual_size
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    /// Reopen the source and reparse its current layout. Failure leaves the
    /// session invalidated, never serving the previous generation's cache.
    pub fn refresh(&mut self) -> io::Result<()> {
        self.reset();
        self.stale = true;
        let fresh = Self::open_configured(
            self.source.path(),
            self.budget,
            self.policy,
            self.limits,
            self.history.clone(),
        )?;
        *self = fresh;
        Ok(())
    }
    /// Replace an already validated history snapshot transactionally. Changed
    /// content clears decoded bytes/counters and invalidates old descriptors.
    /// Replacing with the same digest preserves the existing generation.
    pub fn replace_history(&mut self, history: Arc<HistoryTables>) -> io::Result<()> {
        if !matches!(
            self.policy,
            DecodePolicy::ConfirmedOnly | DecodePolicy::AllowProvisional
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "history replacement requires a history-aware policy",
            ));
        }
        self.validate_source()?;
        if self
            .history
            .as_ref()
            .is_some_and(|old| old.digest() == history.digest())
        {
            return Ok(());
        }
        self.reset();
        self.history = Some(history);
        self.generation = NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    pub fn history_digest(&self) -> Option<[u8; 32]> {
        self.history.as_ref().map(|h| h.digest())
    }
    fn validate_source(&mut self) -> io::Result<()> {
        if self.stale {
            return Err(invalid("store is stale; refresh required"));
        }
        if let Err(error) = self.source.validate(&self.file) {
            self.reset();
            self.stale = true;
            return Err(error);
        }
        Ok(())
    }
    /// Clear resident entries and counters, retaining parsed metadata and fitted tables.
    pub fn reset(&mut self) {
        self.cache.clear();
        self.recency.clear();
        self.stats = Stats::default();
        self.clock = 0;
    }
    pub fn read_chunk(&mut self, index: usize) -> io::Result<DecodedChunk> {
        self.validate_source()?;
        let result = self.read_chunk_inner(index)?;
        self.validate_source()?;
        Ok(result)
    }
    fn read_chunk_inner(&mut self, index: usize) -> io::Result<DecodedChunk> {
        let c = self
            .chunks
            .get(index)
            .ok_or_else(|| invalid("chunk index out of bounds"))?
            .clone();
        self.clock += 1;
        if let Some(e) = self.cache.get_mut(&index) {
            self.recency.remove(&e.touched);
            e.touched = self.clock;
            self.recency.insert(e.touched, index);
            self.stats.cache_hits += 1;
            return Ok(e.value.clone());
        }
        self.stats.cache_misses += 1;
        if !c.supported()
            || (c.codec == 1 && self.policy == DecodePolicy::LosslessOnly)
            || (matches!(c.codec, 3..=7)
                && !matches!(
                    self.policy,
                    DecodePolicy::AllowFittedBc
                        | DecodePolicy::ConfirmedOnly
                        | DecodePolicy::AllowProvisional
                ))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "codec unsupported or BCPack reconstruction not enabled by decode policy",
            ));
        }
        let mut packed = vec![0; c.packed_bytes];
        self.file.seek(SeekFrom::Start(c.file_offset))?;
        self.file.read_exact(&mut packed)?;
        self.stats.source_bytes += packed.len() as u64;
        let (bytes, quality) = match c.codec {
            codec @ (1 | 3..=7)
                if matches!(
                    self.policy,
                    DecodePolicy::ConfirmedOnly | DecodePolicy::AllowProvisional
                ) =>
            {
                let history = self
                    .history
                    .as_ref()
                    .ok_or_else(|| invalid("missing history snapshot"))?;
                let decoded = history.decode(
                    &packed,
                    codec,
                    c.decoded_bytes,
                    self.policy == DecodePolicy::AllowProvisional,
                )?;
                (
                    decoded.bytes,
                    Quality::HistoryBc {
                        codec,
                        provisional_dependencies: decoded.provisional.len(),
                        history_digest: history.digest(),
                    },
                )
            }
            0 => (packed, Quality::LosslessContainer),
            8 => {
                let mut decoder = flate2::Decompress::new(true);
                let mut raw = Vec::with_capacity(c.decoded_bytes + 1);
                let status = decoder
                    .decompress_vec(&packed, &mut raw, flate2::FlushDecompress::Finish)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                if status != flate2::Status::StreamEnd || decoder.total_in() != packed.len() as u64
                {
                    return Err(invalid("zlib stream incomplete or has trailing input"));
                }
                (raw, Quality::LosslessContainer)
            }
            1 => {
                if !c.decoded_bytes.is_multiple_of(8) {
                    return Err(invalid("BC1 size is not block aligned"));
                }
                let (w, q) = self.tables.get_or_insert_with(|| {
                    (
                        bcpack::bc1::Window::default_table(),
                        bcpack::bc1::QuadTable::default_table(),
                    )
                });
                let result = bcpack::bc1::decode(&packed, c.decoded_bytes / 8, w, q)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                (
                    result.blocks,
                    Quality::FittedBc1 {
                        unfitted_blocks: result.unfitted_blocks,
                    },
                )
            }
            codec @ 3..=7 => {
                let block_bytes = if codec == 4 { 8 } else { 16 };
                if !c.decoded_bytes.is_multiple_of(block_bytes) {
                    return Err(invalid("BC chunk size not block aligned"));
                }
                let blocks = c.decoded_bytes / block_bytes;
                let failed = || invalid("BCPack entropy/block decode failed");
                let (bytes, unfitted, invalid_blocks) = match codec {
                    3 => {
                        let o = bc3pack::decode(&packed, blocks).ok_or_else(failed)?;
                        (o.blocks, o.unfitted_blocks, 0)
                    }
                    4 => {
                        let o = bc45pack::decode_bc4(&packed, blocks).ok_or_else(failed)?;
                        (o.blocks, o.unfitted_blocks, 0)
                    }
                    5 => {
                        let o = bc45pack::decode_bc5(&packed, blocks).ok_or_else(failed)?;
                        (o.blocks, o.unfitted_blocks, 0)
                    }
                    6 => {
                        let o = bc6pack::decode_bc6h(&packed, blocks).ok_or_else(failed)?;
                        (o.blocks, o.unfitted_blocks, 0)
                    }
                    7 => {
                        let o = bc7pack::decode(&packed, blocks).ok_or_else(failed)?;
                        (o.blocks, o.unfitted_blocks, o.invalid_blocks)
                    }
                    _ => unreachable!(),
                };
                (
                    bytes,
                    Quality::FittedBc {
                        codec,
                        unfitted_blocks: unfitted,
                        invalid_blocks,
                    },
                )
            }
            _ => unreachable!(),
        };
        if bytes.len() != c.decoded_bytes {
            return Err(invalid("decoded size differs from chunk extent"));
        }
        self.stats.decodes += 1;
        let result = DecodedChunk {
            bytes: bytes.into(),
            quality,
        };
        if c.decoded_bytes <= self.budget {
            while self.stats.cache_bytes > self.budget - c.decoded_bytes {
                let (_, oldest) = self.recency.pop_first().unwrap();
                let removed = self.cache.remove(&oldest).unwrap();
                self.stats.cache_bytes -= removed.value.bytes.len();
                self.stats.evictions += 1;
            }
            self.stats.cache_bytes += c.decoded_bytes;
            self.stats.peak_cache_bytes = self.stats.peak_cache_bytes.max(self.stats.cache_bytes);
            self.cache.insert(
                index,
                Entry {
                    value: result.clone(),
                    touched: self.clock,
                },
            );
            self.recency.insert(self.clock, index);
        }
        Ok(result)
    }
    /// Checked virtual read, returning provenance once per contributing chunk.
    pub fn read_range(&mut self, start: u64, len: usize) -> io::Result<(Vec<u8>, Vec<Quality>)> {
        self.validate_source()?;
        let result = self.read_range_inner(start, len)?;
        self.validate_source()?;
        Ok(result)
    }
    /// Validate one source generation around a bounded batch. On mutation no
    /// partial batch is returned. This amortizes file identity checks without
    /// allowing a stale cache to satisfy a new operation.
    pub fn read_batch(
        &mut self,
        ranges: &[(u64, usize)],
    ) -> io::Result<Vec<(Vec<u8>, Vec<Quality>)>> {
        let total = ranges
            .iter()
            .try_fold(0usize, |sum, (_, len)| sum.checked_add(*len))
            .ok_or_else(|| invalid("batch size overflow"))?;
        if total > self.limits.max_range_bytes || ranges.len() > 100_000 {
            return Err(invalid("batch exceeds limits"));
        }
        self.validate_source()?;
        let mut touched = HashSet::new();
        let mut decoded_work = 0usize;
        for &(start, len) in ranges {
            let end = start
                .checked_add(len as u64)
                .filter(|&n| n <= self.virtual_size)
                .ok_or_else(|| invalid("batch range exceeds source"))?;
            if len == 0 {
                continue;
            }
            let mut i = self
                .chunks
                .partition_point(|c| c.virtual_offset <= start)
                .checked_sub(1)
                .ok_or_else(|| invalid("batch range before mapped data"))?;
            while i < self.chunks.len() && self.chunks[i].virtual_offset < end {
                if touched.insert(i) {
                    decoded_work = decoded_work
                        .checked_add(self.chunks[i].decoded_bytes)
                        .ok_or_else(|| invalid("batch decode work overflow"))?;
                    if decoded_work > self.limits.max_range_bytes {
                        return Err(invalid("batch decoded working set exceeds limit"));
                    }
                }
                i += 1;
            }
        }
        let mut result = Vec::with_capacity(ranges.len());
        for &(start, len) in ranges {
            result.push(self.read_range_inner(start, len)?);
        }
        self.validate_source()?;
        Ok(result)
    }
    fn read_range_inner(&mut self, start: u64, len: usize) -> io::Result<(Vec<u8>, Vec<Quality>)> {
        let end = start
            .checked_add(len as u64)
            .ok_or_else(|| invalid("range overflow"))?;
        if len > self.limits.max_range_bytes || end > self.virtual_size {
            return Err(invalid("range exceeds limits"));
        }
        if len == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut index = self
            .chunks
            .partition_point(|c| c.virtual_offset <= start)
            .checked_sub(1)
            .ok_or_else(|| invalid("range before mapped data"))?;
        let mut pos = start;
        let mut spans = Vec::new();
        while pos < end {
            let c = self
                .chunks
                .get(index)
                .ok_or_else(|| invalid("range not covered"))?;
            let until = end.min(c.virtual_offset + c.decoded_bytes as u64);
            if pos < c.virtual_offset || pos >= until {
                return Err(invalid("range crosses unmapped data"));
            }
            spans.push((
                index,
                (pos - c.virtual_offset) as usize,
                (until - c.virtual_offset) as usize,
            ));
            pos = until;
            index += 1;
        }
        let mut bytes = Vec::with_capacity(len);
        let mut quality = Vec::with_capacity(spans.len());
        for (i, a, b) in spans {
            let decoded = self.read_chunk_inner(i)?;
            bytes.extend_from_slice(&decoded.bytes[a..b]);
            quality.push(decoded.quality);
        }
        Ok((bytes, quality))
    }
}
