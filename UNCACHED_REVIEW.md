# Uncached extraction review

Owner: uncached/performance task. Owned files: `tests/uncached.rs`, this file.
Snapshot reviewed: 2026-11-22 17:17 -08:00, after the parent added the bounded
one-call input span to `texture_dds` (hashes in section 7). No production source,
existing tests, or existing docs were edited.

Fixtures are synthetic RDS5 + ETPF stores built from patterned bytes in a
per-test temporary directory. No game data, decoded bytes, local store paths or
raw benchmark artifacts are involved.

---

## 1. What the new tests prove

Both tests run with `Store` cache budget **0**, i.e. decoded caching disabled
from the first call (not "empty at start"), and assert the counters directly.

### `uncached_volume_dds_decodes_storage_region_once_not_per_z`

Synthetic 16x8x32 RGBA8 volume (mode 2, 4 KiB tile) = 16 384-byte storage
region across four stored chunks, catalog base 0 so the computed layout span,
`allocation_end` and `virtual_size` agree.

* `stats.decodes == 4` (one per contributing chunk), `source_bytes == 16 384`
  (the region read once), `cache_hits == 0`, `cache_bytes == 0`,
  `peak_cache_bytes == 0`, `evictions == 0`.
* Provenance has one entry per contributing chunk, not one per Z slice.
* DDS checks: `DDS `, `DX10`, depth field 32, DX10 dimension 4, payload
  `16*8*4*32`, first two Z slices differ, payload is non-constant.
* Equality: `texture_dds` output is byte-identical to detiling all 32 Z slices
  through `texture_mip` and assembling them with `dds::write_dds`, and the
  reference path's own counters show the cost avoided
  (`decodes == 32*4 = 128`, `source_bytes == 32*16 384`), still with zero hits
  and zero residency.

### `uncached_multimip_shared_tail_span_and_fallback_match_per_mip_reference`

Synthetic 64x64 RGBA8 with a full 7-mip chain: mips 2..6 share one 4 KiB tail
tile at offset 0 (`first_tail == 2`), mip 1 is a standalone 4 KiB tile and mip 0
a 16 KiB region. Complete input span = 24 576 bytes (six chunks); largest single
region = 16 384 bytes.

* Bounded one-call branch (default limits): `decodes == 6` (each chunk exactly
  once for the whole texture), `source_bytes == 24 576`, zero hits/residency,
  one provenance entry per chunk.
* Reference: every mip through `texture_mip`, joined, then `write_dds` is
  byte-identical to the bounded output; the reference takes 10 decodes
  (`5` tail-mip reads of the shared tile + `1` + `4`), showing what the span
  saves and that the tail really is shared.
* Fallback branch (`max_range_bytes = 16 384`, below the span but at the largest
  region): extraction succeeds, DDS is **byte-identical** to the bounded output,
  and the counters prove the fallback (`decodes == 10`,
  `source_bytes == 40 960`), with zero hits/residency/evictions.
* Therefore the bounded-input branch is a pure optimization: it changes decode
  and read counts, not coverage or output.

Gate status of this repository after adding them: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, `cargo test` 72 passed
(24 lib + 2 cli + 19 independent + 17 layout + 8 reader + 2 uncached).

---

## 2. Audit of `src/extract.rs` (bounded span, volume path)

Read-only audit of the current file; no edits made.

| # | Item | Verdict |
|---|---|---|
| A1 | Span bounds: `input_start = min(offset)`, `input_end = max(offset + region)` over every `(slice, mip)`; `input_len` read once when `<= max_range_bytes`. | Correct. Every later `detile` slice is `bytes[offset - input_start .. + region]`, and `offset >= input_start` by construction, so slicing cannot under/over-run. |
| A2 | Slice bounds safety: `bytes.get(start..start + len)` with `start + len <= input_len`. | Correct and panic-free; a mismatch returns `InvalidData` ("mip exceeds texture input"). |
| A3 | Cache isolation: the span read goes through `read_range_inner` (seek/read/decode), never `read_chunk`, so it cannot touch `cache`, `cache_bytes` or `budget`. The buffer is a local dropped at return. | Correct. Confirmed by `cache_bytes == 0` and `peak_cache_bytes == 0` in both tests, including after the reference path. |
| A4 | Limits: the branch condition and `read_range_inner`'s own check use the same `max_range_bytes`; no path can allocate beyond it. `input_len as usize` is guarded by that comparison. | Correct on 32/64-bit. |
| A5 | Volume path: with the span branch taken, `volume_raw` is `None` and the Z loop slices from `input`; with the span branch skipped, `volume_raw` reads the region once per mip. | Correct, **but** see F2: for every *supported* volume the two branches coincide. |
| A6 | Timing: the span read is timed once; per-subresource reads are timed only when the span is absent; `detile` and DDS are timed per call. No double counting. | Correct. Timer objects created but unused in the bounded/volume branches are negligible; not a correctness issue. |
| A7 | Generation guard: `texture_layout` validates the source at entry and `texture_dds` again at exit, so a mid-call mutation yields `Err`, not a partially mixed DDS. The span branch narrows the window; the fallback keeps the same pre/post semantics. | Unchanged from before; correct. |

### Findings

**F1 (medium, documentation): provenance length is branch-dependent.**
`ExtractedTexture.quality` has one entry per chunk in the single span read
(bounded branch) but one entry per read in the fallback path (so the shared tail
is reported 5x for the 7-mip fixture: 6 vs 10). It is a provenance bag, not a
subresource-aligned vector. The public doc comment for `texture_dds` should say
that the length depends on the read strategy and must not be indexed by
mip/slice. No in-repo consumer indexes it (the CLI serializes it), so this is a
documentation/API-risk item, not a regression introduced by the span.

**F2 (low, dead code): the volume fallback is unreachable for supported
volumes.** `layout_volume` rejects `mips != 1`, so a volume's span equals its
single region; either the span branch is taken (`region <= max_range`) or the
per-mip `volume_raw` read would exceed `max_range` and fail as well. The
`volume_raw` branch therefore cannot execute for any currently accepted volume.
The Z-slice single-decode property is now delivered by the span branch (pinned
by the volume test). Recommend deleting the branch, or adding a comment that it
exists for a future multi-mip volume, so nobody assumes an independent fallback
covers it.

**F3 (low, memory bound): span + outputs + DDS coexist.** The span can hold up to
`max_range_bytes` (64 MiB default) while subresource vectors (≤256 MiB cap), the
DDS payload and the final buffer are also live. The 256 MiB output cap is
checked inside the second loop, after the span read. Since the projected output
total is computable from `subresource_len` before reading, checking it first
would avoid allocating a 64 MiB span for a DDS that is certain to be refused.
The "bounded" claim still holds; this is peak-memory accounting, and should be
reported in the gate as span bytes + output bytes.

**F4 (info): the span is a range, not the sum of regions.** Padding between
tiles, array/cube slice strides and the packed tail are all included, so the
gate's "input bytes read" is larger than the visible subresource total (test:
24 576 span vs 21 844 visible). Bounded by `max_range_bytes` and by
`layout.total_bytes`, but reports should quote span bytes so a Python baseline
that reads per region is not compared against a different read granularity
without saying so.

**F5 (info): decode/read counts are a good uncached metric.** With budget 0,
`stats.decodes` and `stats.source_bytes` exactly express work done:
`6`/24 576 bounded vs `10`/40 960 fallback vs `10`/40 960 per-mip reference for
the 7-mip fixture, and `4`/16 384 vs `128`/524 288 for the volume. These are
cheap invariants for the gate harness to assert per texture.

---

## 3. Audit of `src/source.rs`

Unchanged by the uncached work (`18F40EDC…` before and after).

* A8: the guard is same-file identity + `len` + `modified`/`created` stamp,
  compared for both the open handle and the path, before and after public
  operations. With cache budget 0 a stale *cached* entry cannot exist, but the
  open handle can still serve pre-mutation bytes if a writer changes the file
  without changing length and timestamps. That residual risk is unchanged and
  is recorded in the main review; it is not a regression from the span
  optimization.
* A9: no global/static cache or keyed store exists; the span buffer lives inside
  one `texture_dds` call, so cross-file and cross-build isolation still hold
  trivially for the uncached path.
* A10: the guard runs per public call, not per chunk; the span optimization
  reduces the number of internal reads, so it cannot weaken the guard frequency
  (entry and exit are still checked exactly once per call).

---

## 4. Gate mechanics for "caching disabled in both implementations"

* `bench-export STORE INDEX TRIALS --no-cache` opens the store with budget 0, so
  caching is genuinely disabled from process start — it is not an
  empty-at-start cache that fills during the run. `run` with a plan containing
  `"cache_bytes": 0` is equivalent for read-range workloads.
* `bench-export` now prints per-trial `stats` and `stages`; a gate report must
  assert `cache_hits == 0`, `cache_bytes == 0`, `peak_cache_bytes == 0` and
  `evictions == 0` for every trial, and must not call `store.reset()` between
  timed trials as a substitute for disabled caching (reset only clears, it does
  not prevent refilling).
* Warm **filesystem** cache is separate from the decoded-application cache and
  should be reported explicitly per run (cold-OS vs warm-OS), together with the
  span-vs-visible byte counts from F4.
* The Python baseline arm must run with its decoded dictionary retention
  disabled; otherwise the comparison is cache-assisted on one side only.

---

## 5. Open items for the parent (not blockers for these tests)

1. Document the branch-dependent `quality` length in `texture_dds` (F1).
2. Remove or annotate the unreachable `volume_raw` branch (F2).
3. Consider checking the projected DDS output size before the span read (F3).
4. When recording the uncached benchmark, quote span bytes and assert the four
   cache counters per trial (sections 4 and F5).

---

## 6. Verification commands

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test uncached
rustfmt --edition 2024 --check tests/uncached.rs
```

`tests/uncached.rs` needs no corpus, environment variables or network.

---

## 7. Reviewed snapshot (first 16 hex digits of SHA-256)

```
extract.rs   E6184F29966EF3AB
source.rs    18F40EDC7A259FB1
lib.rs       4A2AB24F7E01AFDA
dds.rs       1A407F4D626ACBDF
layout.rs    F7D8CC4CD9C40F0A
catalog.rs   1C7A276D9D08384C
main.rs      9C4308AD0AD0454E
uncached.rs  A9EC75D90E1AA34B
```

If any of these change, re-run section 6 and re-check section 2/3 before
trusting this assessment. No commits and no publication were made.
