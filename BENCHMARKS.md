# Cache-disabled measurements

## Current BC7-table checkpoint (November 22, 2026)

The current SDK incorporates SixNG's 34 additional fitted BC7 presets at
`d724355d`. All 5,167 affected chunks match the current independent reference;
251 hashes change and 839,119 BC7 missing-preset flags disappear. Every fitted
result stays provisional. The full earlier audit is preserved as a separate
stage; see [CORPUS_QA.md](CORPUS_QA.md).

After audit/compiler/worker activity stopped, seven alternating complete DDS
trials, both decoded caches disabled, equal bytes and zero native cache use:

| Workload | Native median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| plain | 0.200 | 1.674 | 8.4x |
| mips | 0.948 | 17.302 | 18.3x |
| cube | 0.426 | 8.502 | 20.0x |
| volume | 1.535 | 37.246 | 24.3x |
| volume64 | 2.657 | 79.720 | 30.0x |
| array | 5.263 | 134.659 | 25.6x |
| platform | 2.557 | 73.884 | 28.9x |

Warm OS cache, source guards included; startup, catalog parsing, verification
hashing and file writes excluded. A before/after control against the prior
consumer build uses three alternating runs of seven extractions: time ratios
are 0.966 (plain), 1.008 (mips), 0.957 (larger volume) and 1.026 (array).
That is a sampled variation within about 4.3%, not a universal no-regression
claim. Both versions return identical DDS bytes with zero decoded cache use.

The same 64-texture/46-store batch writes and fsyncs 29,706,888 equal bytes:
native median 0.197 s, Leonida
5.696 s, 29.0x (three alternating trials).
The common Python writer is included; startup/catalog/analysis/hash are not.

Selected provisional BC6H Python exports remain equal and uncached:
unsigned index 217, 1.557 vs 50.629 ms (32.5x, five trials); signed index 106,
118.370 vs 8,409.998 ms (71.0x, three trials). Both use the same externally
fitted profile. They are not an original-history oracle or random corpus sample.

A separate conservative control keeps Leonida's normal per-export reuse,
with the native decoded cache still disabled (five alternating trials):

| Workload | Native ms | Leonida ms | Ratio |
|---|---:|---:|---:|
| mips | 1.169 | 25.088 | 21.5x |
| cube | 0.582 | 3.628 | 6.2x |
| array | 5.945 | 138.645 | 23.3x |

This control has no persistent cache but is separate from the strict matrix.
All 1,076 saved DDS references and 843 sampled SixNG preview mip/slices pass.

Current CLI SHA-256:
`e0dc31bf96401a026817674e0f1e66f2fdfe7a272f1212ca63441297bab922cf`.
Current Python extension SHA-256:
`03c9349c52fe261772c116e0f7a05a65c1dd1e9b2b2ccf782571de3dbf42fe01`.
The all-BC7 parity runs include single-pass timing fields; concurrent work and
unequal timing boundaries make those fields unsuitable for speedup claims.
No overall SixNG speed lead is established. Earlier sections retain their own
artifact identities and measurement scope.

## Earlier exhaustive corpus and consumer QA checkpoint (November 22, 2026)

The full compatibility run is complete: all 1,026,833 chunks match pinned
SixNG, and all 357,115 full DDS outputs pass independent container validation.
See [CORPUS_QA.md](CORPUS_QA.md) and [CONSUMER_QA.md](CONSUMER_QA.md).
Fitted history is still provisional; the corpus contains 963,166 flagged
unfitted blocks in both implementations.

Separate measurements below ran after audit/compiler work stopped. Both
decoded caches are disabled, with zero native hits/residency/peak residency.
Seven alternating trials, complete DDS construction, warm OS cache:

| Workload | Native median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| plain | 0.194 | 1.572 | 8.1x |
| mips | 0.935 | 16.349 | 17.5x |
| cube | 0.416 | 8.242 | 19.8x |
| volume | 1.562 | 35.072 | 22.5x |
| volume64 | 2.679 | 70.653 | 26.4x |
| array | 4.859 | 127.966 | 26.3x |
| platform | 2.340 | 63.341 | 27.1x |

Output bytes agree on every trial. Source freshness checks are included;
startup/catalog lookup, verification hashing and file writes are excluded.
The two frozen native checkpoints retain equal bytes and zero-cache counters:
after/before median time ratios are 0.993 (plain), 1.014 (mips), 1.030 (array)
and 1.039 (larger volume), across three alternating runs of seven extractions.
This is a sampled control within about 4%, not a universal no-regression claim.

The 64-texture/46-store batch constructs and writes 29,706,888 equal bytes,
including writes, flush and fsync. Three alternating trials: native median
0.172 s, Leonida 5.100 s, 29.6x. Both use the same Python file writer;
this is separate from the CLI's atomic publication workflow.

Two real provisional BC6H Python exports also remain equal and uncached:
unsigned seven-mip index 217, 1.387 vs 49.077 ms (35.4x, five trials);
signed index 106, 115.442 vs 7,882.287 ms (68.3x, three trials).
Both use the same locally fitted external history. These selected successes
do not establish original-seed accuracy or a random whole-corpus speed estimate.

### Control with normal baseline reuse

Native DDS construction coalesces input regions within one request and discards
that temporary buffer at return. To make the performance boundary reviewable,
a separate comparison leaves Leonida's normal per-export decode dictionary
enabled while the native Store still has a zero-byte decoded cache:

| Workload | Native ms | Leonida ms | Ratio |
|---|---:|---:|---:|
| mips | 1.055 | 17.067 | 16.2x |
| cube | 0.451 | 3.142 | 7.0x |
| array | 4.692 | 125.109 | 26.7x |

Five alternating trials, equal bytes, no cache retained between exports.
These are comparison controls, not part of the strict both-caches-disabled
matrix. Reproduce with compare_python_sdk.py --baseline-per-export-cache.

Native executable SHA-256 for this earlier checkpoint:
970d58e66e2fa4ca131880da96d1aa258763bafd1cf13e375a59d44c1383a978.
Python extension SHA-256 for this earlier checkpoint:
de430fa7bd81df4bed2af29167a84514427aff1c2d1ac05edf88ba94c5896287.

Both retain all 1,076 saved complete DDS reference matches. The updated
previews match 843/843 mip/slices against combined SixNG PRs #27/#28/#35.
Independent DirectXTex covers 631 real subresources across all 20 observed
formats, with exact and declared one-level quantization checks kept separate.
No general SixNG speed lead or full original-history confirmation is claimed.

Earlier sections retain their own source fingerprints and measurement scope.

## Release-blocker checkpoint (November 22, 2026)

Current source fixes BC6H confirmation depth per colour channel, synchronizes
SixNG 7881 fitted presets (18 BC6H and 13 BC5 additions), and exposes uncached
single/batch exports. The combined SDK remains unpublished and development-only.

All seven complete DDS workloads remain byte-identical with decoded caches
disabled on both sides, seven alternating trials:

| Workload | Native median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| plain | 0.316 | 3.437 | 10.9x |
| mips | 1.339 | 32.822 | 24.5x |
| cube | 0.641 | 13.741 | 21.4x |
| volume | 2.410 | 76.024 | 31.5x |
| volume64 | 4.144 | 154.515 | 37.3x |
| array | 6.883 | 289.823 | 42.1x |
| platform | 3.583 | 126.178 | 35.2x |

These timings include source guards and complete DDS construction; they use
warm OS cache and exclude startup/catalog parsing, hashing and file writes.
The native executable SHA-256 is
`7c0b9532e7f07005c73f2a3ac31fcd757fc8ab5078610a6e45026860927766e6`.

Two real provisional BC6H DDS exports through the installed Python adapter:

| Lighting sample | Native median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| Signed, index 106, 2,097,300 bytes | 119.919 | 8620.982 | 71.9x |
| Unsigned mip chain, index 217, 174,900 bytes | 1.522 | 49.248 | 32.4x |

Both use the same locally self-calibrated profile, disabled decoded caches and
five alternating trials. Output bytes agree. Python/Rust conversion and
descriptor checks are included; opening/catalog/table setup and writes are
excluded. The sample search found seven unavailable and 78 raw/zlib textures
before selecting these two history-dependent successes. This is not a random
corpus-wide performance estimate. Reproduce with `compare_python_sdk.py`,
`--allow-provisional --history PROFILE`, and the indexes above.

A sampled 64-texture batch across 46 stores writes 29,706,888 equal DDS bytes,
including new-file creation, writes, flush and fsync. Three alternating trials:
native median 0.181 s, Leonida 5.982 s, **33.0x**. Both decoded caches are
disabled. Startup/catalog/analysis and verification hashing are excluded;
writers are identical Python code on both sides, rather than the CLI's atomic
publication workflow. Use `scripts/compare_python_batch.py` with saved reference
reports. Fresh temporary output files are removed after verification.

An alternating native-before/native-after control on plain, mip, array and
larger-volume workloads uses three runs of seven uncached extractions each.
After/before time ratios are 1.029, 0.992, 0.953 and 0.964 respectively; outputs
and zero-cache counters agree. This compares frozen binaries across the
confirmed-depth repair under the same local conditions. It supports no material
sampled regression, not a guarantee for every input. Use
`scripts/compare_native_versions.py` for the same control on later changes.

Current Python extension SHA-256:
`596752ff0a9f1223afc8a3ba629cfde7b4fc6be97820f4553b4a4d7f439db64a`.
It passes 1,076 saved DDS references, 17 SDK/report tests and four adapter tests.
Native core tests: 135 passing. Converter example tests: 14 passing (including
vendored codec tests). Formatting and Clippy pass with warnings denied.

### Accuracy and compatibility changes

- Real external-history comparisons with unmodified Leonida: 11 BC6H matches
  out of 64 bounded samples (34 mutual refusals, 19 prefilter exclusions), and
  73 BC7 matches out of 107 (18 mutual refusals, 16 prefilter exclusions).
  Successful outputs and label presence agree, with zero decoded cache use.
  These are provisional histories; no original-pixel confirmation is inferred.
- Original raw/zlib BC6H blocks from 14 real signed/unsigned textures produce
  23,838,720 exact half values across 496,640 blocks against bcdec. Two
  unavailable samples remain separate. This checks block-to-pixel decoding.
- All 843 mip/slice previews across 31 grouped graphics, lighting and peds
  textures match a locally combined SixNG patch, with zero metadata errors.
  The layout repairs are in [SixNG #27](https://github.com/holdmysocks/SixNG/pull/27);
  alpha-only preview repair is in [SixNG #28](https://github.com/holdmysocks/SixNG/pull/28).
  Current upstream without those patches still has the documented disagreements.
- [PUBLIC_HISTORY_RESEARCH.md](PUBLIC_HISTORY_RESEARCH.md) records the public
  source search and why fitted tables are not a confirmed-history oracle.

The final source archive was rebuilt in a fresh Cargo target directory. Its
installed wheel passed all 21 Python/report/adapter tests and all 1,076 saved
DDS references. Source-built extension SHA-256:
`cc0b71f3e257f710d397f249475a7f9d293fe5360e6bf9ab81eea81288472e8e`.
Both SixNG PRs #27/#28 pass Windows, Linux and macOS CI and remain open.

Earlier sections below retain their own fingerprints and measured scope.

## History and format checkpoint (November 22, 2026)

After integrating strict/provisional custom histories, expanded typed previews
and leading DDS mip selection, all 1,076 saved complete DDS reference outputs
still match through both the native CLI and installed Python SDK. Native tests:
129 passing. Python SDK/adapter tests: 19 passing. Root and binding Clippy pass
with warnings denied. Hosted cross-platform CI has not run.

Seven alternating trials, decoded caches disabled on both sides:

| Workload | Rust median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| Plain | 0.196 | 1.594 | 8.1x |
| Mip chain | 0.857 | 16.567 | 19.3x |
| Cube | 0.422 | 8.301 | 19.7x |
| Volume | 1.544 | 35.309 | 22.9x |
| Larger volume | 2.527 | 71.008 | 28.1x |
| Array | 4.258 | 128.284 | 30.1x |
| Platform texture | 2.405 | 64.005 | 26.6x |

Every output matches and cache hits/residency/evictions remain zero. These are
complete DDS construction timings with warm filesystem cache, source guards
included; startup/catalog parsing, hashing and file writes excluded. They do
not establish full batch throughput or a SixNG performance lead.

The real Python adapter also matches three-mip cube and array output against
Leonida: 0.527 vs 5.273 ms (10.0x), and 5.252 vs 125.537 ms (23.9x), five
alternating trials. These include descriptor checks and Python bytes conversion.
Use `scripts/compare_python_sdk.py --mips 3` with the cube/array cases below.

The full structural scan checks all 357,115 descriptors across 57 stores:
zero layout/span failures and zero unsupported preview-format IDs in this
build. This scan performs no pixel decode. Newly added signed/typed format
paths are synthetic-tested; their existence in a future build is not assumed.
Signed BC4/5 additionally match 400 deterministic external bcdec oracle cases.

History validation and explicit refusal differences are in [HISTORY.md](HISTORY.md).
Use `scripts/verify_history.py` for real chunks, with optional `--history` and
`--allow-provisional`; matching refusals are reported separately from matching
bytes. Raw reports remain outside Git.

Current executable SHA-256:
`39401a256dccd38b09b174bdb82b595c41d037f8297b82c884e1c8a3a2df2f38`.
Python extension SHA-256:
`3fef1d5cbf2402e07f18c1d8e6a24669d28dff5385d946501a17864687d0d7cc`.

The current source archive was independently rebuilt with a fresh Cargo target
directory. Its installed wheel also passed all 19 Python/adapter tests and all
1,076 saved DDS references. Source-built extension SHA-256:
`a62457b23f16f5c35fac747f7607e4242f58d90ff4498547448b431adbdcc56e`.
Wheel structure, typed stubs, licenses/notices and source-archive exclusions pass.

The sections below retain earlier measurements and their original fingerprints.

Measured locally on 2026-11-22 (America/Los_Angeles), Windows x64,
Intel Core i5-11600K, Rust 1.97.1 release build, Python 3.12.7.
This is a development checkpoint, not a release-gate pass.

## Complete DDS construction versus Leonida

Unmodified baseline: Leonida `tools/ttpf_mips.py` at
`aa2988cc555bad7ecd508b48a702adbb32fe7c95`.
Seven alternating trials per workload; medians below.
**Decoded-result caches are disabled on both sides**, including within each
extraction. Every output agrees byte for byte. Each native trial asserts zero
cache hits, current/peak resident cache bytes and evictions.

| Workload | Native virtual input bytes | Complete DDS bytes | Leonida | Rust | Ratio |
|---|---:|---:|---:|---:|---:|
| plain (32x32x1, 1 mip(s)) | 4,096 | 4,244 | 1.581 ms | 0.193 ms | 8.18x |
| mips (128x1024x1, 8 mip(s)) | 192,512 | 174,964 | 16.555 ms | 0.982 ms | 16.86x |
| cube (64x64x6, 7 mip(s)) | 49,152 | 33,076 | 8.138 ms | 0.427 ms | 19.07x |
| volume (32x32x32, 1 mip(s)) | 262,144 | 262,292 | 35.570 ms | 1.525 ms | 23.32x |
| array (512x512x3, 8 mip(s)) | 1,179,648 | 1,048,708 | 127.356 ms | 4.724 ms | 26.96x |
| platform (1024x512x1, 1 mip(s)) | 524,288 | 524,436 | 64.089 ms | 2.365 ms | 27.10x |
| volume64 (32x64x32, 1 mip(s)) | 524,288 | 524,436 | 69.875 ms | 2.570 ms | 27.18x |

The measured region includes read/decode, detiling, cropping, every visible
mip/face/slice, DDS header/payload construction and native source freshness
checks. It excludes startup, opening/catalog analysis, hashing and output file
writes. The OS filesystem cache is warm. These are complete DDS-construction
timings, not cold-disk, PNG encoding or whole-directory wall-clock timings.
All selected textures use raw/zlib input; fitted BCPack throughput is unmeasured.

Native input spans include tile/row padding. A bounded call-local input buffer
avoids decoding shared tail chunks repeatedly. It is discarded after each call;
Store cache budget remains zero. If the complete span exceeds the configured
range limit, extraction uses the existing per-region fallback. The baseline
uses its unmodified extraction algorithm through a local subclass passing a
fresh decode dictionary for every `read_range`; no decoded bytes survive for
reuse between those calls. Read granularity therefore differs by design.

These representative workloads cover a small 2D texture, an eight-mip BC7
chain, a seven-mip six-face cube, two float16 volumes, a three-slice eight-mip
BC5 array, and a larger BC7 texture. They were selected from known-successful
parity cases, not sampled to estimate the entire corpus's speed distribution.
Separate profiling identifies native detiling versus Python loops as the main
source of the Leonida lead. This is not a claim of a new BCPack algorithm.

## In-process Python SDK checkpoint

The separately built Python wheel was installed into an isolated CPython 3.12
Windows environment and exercised through the experimental Leonida adapter.
This measures the actual Python call, descriptor agreement checks, source
freshness checks, complete DDS construction, return-object creation and
Rust-to-Python bytes conversion. Seven alternating trials, decoded caches
**disabled on both sides**, warm OS cache. Startup/catalog/hash/file writes
remain outside the measured region.

| Workload | Leonida median | Python SDK median | Ratio | Output |
|---|---:|---:|---:|---|
| 1024x512 BC7 | 64.097 ms | 2.935 ms | 21.84x | Identical |
| 512x512x3 BC5 array, eight mips | 129.614 ms | 4.661 ms | 27.81x | Identical |

The final source archive rebuilt a working wheel from its extracted project
root. That installed wheel passed 11 SDK tests and four adapter tests. All
1,076 saved DDS references were rechecked through Python with no mismatches,
no unexpected fitted output and zero cache hits/residency/evictions.
The extension SHA-256 for this run is `34fd7ab73187d58915b54835564cd9e417c04765ec0ed3c86b4e6b51acc33b62`.

Reproduce with `scripts/compare_python_sdk.py --leonida CHECKOUT --store POOL
--index INDEX --out EXTERNAL_REPORT.json` using the interpreter where the wheel
is installed. Use `scripts/recheck_python_exports.py` with the saved external
reference reports for the full sampled set. These measurements do not establish
full-directory I/O performance or complete backend feature parity.

The native CLI table above is the earlier uncached checkpoint, pinned by its
own executable fingerprint below. The newer BC6H accuracy work and optional
Python adapter do not change that historical measurement's scope.

## SixNG reader comparison

Unmodified SixNG core at `702435cd56fd09de4f456ddc94d60886de7365ee`, linked
through `scripts/sixng-reference`. Both readers have no decoded-result cache.
The baseline harness only adapts SixNG's extra 4096-byte virtual-address origin.

On a 49,101-chunk maps store, six evenly spaced lossless chunks were read as
4 KiB ranges and as full chunks (12 cases, seven alternating trials). All bytes
agreed and SixNG reported zero skipped bytes. Median ratios range from
**0.83x to 1.47x**. One case is slower; several are near parity. This reader
primitive excludes startup/catalog parsing/hash/file writes, uses warm OS cache,
and includes native source guards. **No overall SixNG extraction lead is claimed.**

## Correctness evidence

- All descriptor fields/keys and chunk tables compared across 57 supplied stores.
- 891 stratified full raw/zlib chunks, 239,964,160 decoded bytes, equal Leonida.
- 1,076 sampled complete DDS files equal Leonida; saved hashes rechecked after
  the uncached optimization with zero mismatches. 109 baseline-unavailable
  cases are recorded separately and are not counted as successes.
- 92 sampled chunks equal unmodified SixNG: stored 4, BC1 16, BC3 1, BC4 8,
  BC5 8, BC6H 8, BC7 23, zlib 24. No skipped bytes or chunk disagreements.
- Nine grouped graphics PNG/RGBA previews equal SixNG; two disagreements remain
  (mip-count interpretation and volume layout). BC1 also disagrees with
  Leonida's provisional reconstruction. Neither side is silently chosen as
  an accuracy authority.
- At the earlier checkpoint, 84 synthetic Rust tests passed, including uncached volume/shared-tail/fallback
  equivalence, malformed input, source mutation/replacement, refresh, generation
  isolation, cache budget, provenance, layouts, DDS and CLI resumption.

This is sampled validation, not exhaustive corpus extraction or an independent
original-pixel oracle. Real external BPTC history coverage, additional builds, unresolved baseline
disagreements and broader batch/consumer tests remain release blockers;
see [FEATURE_PARITY.md](FEATURE_PARITY.md).

## Reproduction

Use external unmodified checkouts at the revisions above and keep all inputs,
decoded files and reports outside Git. Build the native CLI with
`cargo build --release --locked`, then run:

```powershell
python scripts/compare_export.py --leonida D:/source/Leonida --store D:/game/data/ttdb/ui/platform.ttpf --index 0 --exe target/release/rage-texture.exe --out D:/scratch/platform.json --no-cache
python scripts/compare_export_matrix.py --leonida D:/source/Leonida --exe target/release/rage-texture.exe --out D:/scratch/matrix --case platform D:/game/data/ttdb/ui/platform.ttpf 0 --case mips D:/game/data/ttdb/graphics/ttdb_graphics.ttpf 544
```

The recorded matrix additionally used graphics indexes 439 (plain), 177 (cube),
575 (volume), 200 (volume64), and peds `ttdb_peds_0_3.ttpf` index 3903 (array).
Indexes describe this supplied build, not a stable identifier across updates.

For independent validation:

```powershell
python scripts/verify_corpus.py --leonida D:/source/Leonida --stores D:/game/data/ttdb --exe target/release/rage-texture.exe --out D:/scratch/chunks.json
python scripts/verify_exports.py --leonida D:/source/Leonida --stores D:/game/data/ttdb --exe target/release/rage-texture.exe --per-store 30 --out D:/scratch/exports.json
python scripts/recheck_exports.py --reference-report D:/scratch/exports.json --exe target/release/rage-texture.exe --out D:/scratch/recheck.json
```

Content-pack stores were validated separately with 15 grouped samples per store.
`recheck_exports.py` refuses changed input size/mtime or a binary changed during
validation; its saved-hash shortcut is local test tooling, not a product cache.

Build the SixNG reference separately with
`cargo build --release --locked --manifest-path scripts/sixng-reference/Cargo.toml`.
On this machine its native dependency required a command-local
`CMAKE_GENERATOR=Visual Studio 17 2022`; no global configuration was changed.
Then use `scripts/verify_sixng.py` for chunk/pixel equality and
`scripts/compare_sixng_range.py` for cache-disabled reader timing. Each takes
`--store`, `--exe`, `--reference` and an external `--out`. Known pixel
mismatches correctly make the validator return nonzero.

## Fingerprints

Measured native executable SHA-256: `aba66b6966a6b9f63c4d1590635d6716947abc47c802a8190f32a7e5c7924026`.

Cargo.lock SHA-256: `ebbca862a462819fc30e849b01fa7886ac45d6ba30652ccef698645606fe520b`.

SixNG reference executable SHA-256: `51a26a075f653e4e2eb67de35f9eff6de2a7020139d3919c29a1bb0c20e17b39`.

Earlier cached-reader microbenchmarks are historical and do not establish this
checkpoint's performance or releasability. Raw reports remain outside Git.

The BC6H accuracy checkpoint independently compares 14,155,776 half values
against a pinned C oracle; see [ACCURACY.md](ACCURACY.md). The focused signed-zero
fix is open upstream as [SixNG #24](https://github.com/holdmysocks/SixNG/pull/24).
