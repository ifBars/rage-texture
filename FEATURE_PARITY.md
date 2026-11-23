# Release acceptance matrix

Status: development, NOT releasable. This is a texture SDK and CLI.
The gate is texture feature/quality/coverage parity plus a reproducible
advantage. The SDK does not replace the other projects' runtime, model,
audio or world tools.

| Capability | Current evidence | Remaining gate |
|---|---|---|
| Catalogs and raw/zlib | All 57 stores, all descriptor fields and chunk tables; all 918,228 raw/zlib chunks match the independent SixNG run | Additional builds and variants |
| Six fitted BCPack codecs | All 108,605 chunks match the frozen SixNG profile; all 5,167 BC7 chunks also match the later 34-preset update, with zero unfitted/invalid BC7 blocks | Missing BC1 presets and independent original-history accuracy |
| Mips and layouts | All 357,115 descriptors exported as full DDS; all independent container checks pass; 1,076 complete DDS files match Leonida | Unmeasured shapes and build variants |
| Arrays, cubes, volumes | Complete corpus: 127 arrays, 567 cubes, three volumes; 843 representative mip/slice previews match combined SixNG fixes | Multi-mip volumes and other unmeasured modes |
| DDS consumers | Every full-corpus DDS passes ddsfile metadata/length validation; DirectXTex validates representative output across all 20 observed formats | Every pixel in the complete corpus is not independently decoded |
| PNG accuracy | 40 typed previews; BC1 transparency and BC3 palette repairs; 631 real DirectXTex subresources, six negative controls, synthetic codec edge cases | Full preview batching and encoding benchmarks |
| Batch/CLI | Atomic publication, per-texture errors, byte-verified resumption; 64-texture/46-store batch matches including writes/fsync | Complete mip/slice preview batching, cancellation and bounded parallel work |
| Cache correctness | Same-size edits, replacements, cross-file isolation, refresh/generation, history changes, eviction and budget-zero tests | Identity/timestamp-preserving edits require explicit refresh; concurrent writers are not snapshotted |
| History policy | Immutable snapshots and consumed-bit provenance; real provisional comparisons match Leonida; all 268 BC6H and 5,167 BC7 built-in strict attempts refuse missing history | Independent confirmed BPTC initial histories and broader custom-profile coverage |
| Future adapters | Private nonpersistent Store cache and explicit decoder/table identity | PS5/PC adapters unimplemented |
| Uncached performance | Seven DDS samples 8.4x-30.0x faster than Leonida; batch with fsync 29.0x; selected real BC6H Python exports 32.5x/71.0x | Whole-directory wall-clock/PNG measurements and a comparable overall SixNG result |
| BC6H block accuracy | 294,912 synthetic cases and 496,640 real raw/zlib blocks match independent bcdec half values | This does not confirm BCPack seed accuracy |
| Integration and regression | 138 core + seven DDS consumer Rust tests; 16 example tests; 33 Python/report/adapter tests; current CLI/wheel retain 1,076 references | API stabilization and hosted SDK cross-platform CI |
| Upstream | SixNG #24/#28/#35 merged; #27 has green three-platform release CI; Leonida #4664 and texture2ddecoder #18 submitted | Upstream acceptance is separate from this SDK's release gate |

The whole-corpus compatibility gate is closed for this supplied build.
It uses provisional fitted reconstruction. The later BC7 table update removes
all 839,119 BC7 missing-preset flags; 124,047 BC1 flags remain. The update is
validated across every affected BC7 chunk and layered onto the frozen full
audit, rather than presented as a second full export. Invalid blocks are zero.
Matching fitted output is compatibility evidence, not original-pixel proof.

See [CORPUS_QA.md](CORPUS_QA.md), [CONSUMER_QA.md](CONSUMER_QA.md),
[BENCHMARKS.md](BENCHMARKS.md) and
[PUBLIC_HISTORY_RESEARCH.md](PUBLIC_HISTORY_RESEARCH.md).
No SDK merge, publication or release has been performed.
