# Independent review and release assessment

Owner: test/review task (owns `tests/independent.rs` and this file only).
Final snapshot: 2026-11-22 17:01 -08:00, after the parent's D1/D2/D6/D9 fixes
landed. The tree moved repeatedly during this task; the hashes in section 8 pin
exactly what was reviewed, and section 1 records the gate as re-run.

Fixtures are synthetic bytes written to per-test temporary directories and
removed on drop. No game data, decoded bytes, store paths or benchmark artifacts
are referenced or produced here.

---

## 0. Verdict

**Not releasable yet.** The code-level findings raised in the first review that
matter for correctness are now fixed or explicitly bounded, the cache-generation
requirements are covered by executable tests, and the Rust gate is green. What
still blocks the user's release gate is evidence, not plumbing:

1. **BC1 accuracy has no oracle.** Fitted BC1 output disagrees with Leonida's
   provisional reconstruction and neither is established as correct
   (`BENCHMARKS.md`, `FEATURE_PARITY.md`). Matching another fitted decoder is
   compatibility, not accuracy.
2. **No SixNG output/coverage comparison and no recorded end-to-end benchmark.**
   The owner reports 989 base-store + 87 content-pack full-DDS outputs matching
   Leonida and a 27x single-`fullDDS` timing including the source guards. That is
   strong progress, but it is not yet reproducible from a committed report, and a
   single-texture timing is not the "end-to-end extraction + parity" gate.
3. **Corpus validation is sampled.** 891 chunk extents are exact; the exhaustive
   chunk scan and descriptor parity across all 57 stores are explicitly deferred
   (D3), and layout/DDS/PNG equality beyond synthetic tests is unproven.
4. **Documentation/claim drift remains** (D7 `BENCHMARKS.md`, D8 CLI docs) and
   a repo-hygiene problem was introduced with the SixNG harness (D14).
5. **Batch resume is now byte-verified** (D9 fixed) but the CLI paths that use it
   are still untested (D11).

---

## 1. Verification performed (final snapshot)

| Command | Result |
|---|---|
| `cargo fmt --check` | clean, exit 0 |
| `cargo clippy --all-targets -- -D warnings` | clean, exit 0 |
| `cargo test` | 68 passed, 0 failed: 23 lib unit, 1 `cli`, 19 `independent`, 17 `layout`, 8 `reader`, 0 doc |
| `tests/independent.rs` (owned) | 19 passed |
| `rustfmt --edition 2024 --check tests/independent.rs` | clean |

All 19 independent tests pass unchanged through the parent's D1/D2/D6/D9 edits.

---

## 2. Behaviour pinned by `tests/independent.rs`

* Fitted codecs: explicit opt-in (codec 1 refused under `LosslessOnly`), exact
  upstream bytes, per-chunk provenance (fitted never labelled lossless), cached
  repeats keep provenance, corrupt/truncated BC1 and non-BCPack payloads refuse
  with zero decode/cache bytes, codec 3 still needs `AllowFittedBc`.
* Chunk table (13 malformed branches): count bounds, truncated table, magic/size
  tag/virtual-size floor, sentinel offset and address mismatch, reversed and
  past-EOF extents, zero decoded/packed extents, data inside the metadata
  region, over-long padding, stored-extent mismatch, `max_chunks` boundary.
* Reserved codecs 2 and 9–15: `supported() == false`, `Unsupported` under all
  three policies, including through `read_range`, never filler bytes.
* zlib: exact round trip; one-byte and half truncation; corrupted Adler-32;
  trailing input; declared extent too large/too small; 8 MiB expansion declared
  as one page is bounded and refused.
* Padding/gaps: non-zero chunk padding and post-table metadata are ignored and
  not counted in `source_bytes`; a leading virtual hole errors instead of
  returning zeroes (the announced metadata prefix).
* Cache/source generation: cross-file isolation at equal size; same-size
  in-place update detected (cache flushed, no mixed generation, stale until
  `refresh`); rename-over replacement detected with layouts growing and
  shrinking; `refresh` discards the previous generation's bytes and counters
  (a retained index-keyed entry would show as a hit); failed `refresh` leaves
  the session stale; budget, policy and limits survive `refresh`.
* `read_batch`: one validation per batch, whole-batch failure, empty batch, no
  partial result or cache retention after mutation, and — new this round —
  `read_batch_caps_distinct_decoded_work` pins the D2 fix: 32 requested bytes
  touching two chunks are refused under a 5000-byte cap, repeated ranges are
  not double-counted, zero-length ranges cost nothing, and the exact boundary
  passes.
* Catalog/extraction: synthetic ETPF descriptor parses; `texture_dds` yields a
  `DDS `/`DX10` file of exactly `148 + 16*16*8` bytes with `FittedBc1`
  provenance; `LosslessOnly` refuses it; after replacement + `refresh` the old
  descriptor is refused as `InvalidData` while a fresh descriptor exports.

Structural facts encoded by the builder: virtual addresses are 4096-byte units,
so every chunk extent is a positive multiple of 4096 and the `is_multiple_of(8)`
/ `is_multiple_of(16)` codec guards are defensive only; the first chunk starts
after the reserved 16-byte ETPF header at the 256-aligned metadata end.

---

## 3. Findings: status at the final snapshot

### D1 — LRU eviction was O(n) per eviction — **FIXED, verified**

`Store` now keeps `recency: BTreeMap<u64, usize>` with `pop_first()` for
eviction and `remove`/`insert` on touch (`src/lib.rs`). Eviction is O(log n)
per victim. No dedicated test from me; `tests/reader.rs` still pins exact
budget/eviction counters.

### D2 — `read_batch` bounded returned bytes but not decode work — **FIXED, verified**

`read_batch` now accumulates `decoded_work` over distinct touched chunks via a
`HashSet` and refuses with `InvalidData` above `limits.max_range_bytes`. Covered
by `read_batch_caps_distinct_decoded_work`.

### D3 — zlib chunks require an exact packed extent — **DEFERRED (owner), keep as gate item**

Owner answer: 891 sampled chunks all have exact extents; the exhaustive scan is
deferred and must be an explicit release-gate item, not an assumption. Until
then `total_in == packed_bytes` remains a strictness risk for unseen stores.

### D4 — unconditional 16-byte reservation after the metadata — **FIXED (documented)**

`src/lib.rs` now states the rule: ETPF's 16-byte header lives at
`metadata_end`, and the reserved virtual metadata prefix is not exposed as chunk
data. Residual: `tests/reader.rs` still writes an ETPF-looking magic at the
*unaligned* table end (offset 40), which reads as a contradicting fixture (Q1).

### D5 — store whose first chunk is not at virtual 0 — **INTENDED**

Owner confirms real stores start after the metadata reservation. The behaviour
(reads in the prefix fail, never zero-fill) is pinned by
`a_leading_virtual_hole_is_an_error_not_zeroes`.

### D6 — licensing notice — **FIXED, verified**

`THIRD_PARTY_NOTICES.md` was rewritten: the "no copied source" contradiction is
gone, SixNG files and Leonida ports are attributed with revisions, `png`,
`texture2ddecoder` and `same-file` are listed with licenses, and the separate
SixNG harness is called out. Bundled headers are intact.

### D7 — `BENCHMARKS.md` claims — **OPEN**

It still says "Other BCPack codecs explicitly fail as unsupported" and carries
no end-to-end table. The owner's new results (989 + 87 DDS outputs matching
Leonida, 27x single-`fullDDS`) must be recorded there with method, hashes and
the guard-inclusive boundary, or the file should stop making current-behaviour
statements.

### D8 — CLI docs and usage text — **OPEN (owner: next)**

`src/main.rs:26` usage string still lists only `inspect`/`run`; README still
documents only `inspect`/`read-range`. `list`, `export`, `export-all`,
`bench-export`, `--allow-fitted` and the `batch` plan field are undocumented.

### D9 — batch export resume — **FIXED, verified**

`src/output.rs` now has `publish_or_verify` (resume only when the existing file
is byte-identical, otherwise `AlreadyExists`) and `publish_manifest` (replaces
only a manifest whose `generator` is `rage-texture`), both via a temp file and
sync. `export-all` uses them. Residuals: `.rage-tmp-*` litter on crash,
`hard_link` unsupported on some output volumes, and no CLI test (D11).

### D10 — preview treats DXGI 10 (TYPELESS) as half float — **OPEN**

`src/preview.rs` maps 10 like 11 (`R16G16B16A16_FLOAT`). TYPELESS has no fixed
interpretation; either refuse it in the preview path or document the float
assumption in the HDR policy row.

### D11 — CLI coverage — **OPEN**

`tests/cli.rs` still exercises only `read-range`. `list`, `export`,
`export-all` (including resume/second-run and foreign-manifest refusal),
`bench-export` and `publish_*` are untested. Parent-owned; flagged, not edited.

### D12 — stats semantics across generations — **OPEN (doc)**

`refresh()`/`reset()` zero `Stats`. Tests pin per-generation counters; README
should say so for benchmark aggregation.

### D13 — catalog allocation span is a hard equality check — **OPEN (needs corpus answer)**

`catalog.rs` derives `allocation_end` from the next distinct base (or
`virtual_size`) and `extract.rs` requires `base + layout.total_bytes` to equal
it. Correct for tight compact spans; a retail allocation with slack padding
would now be refused. Confirm against the corpus or downgrade to a diagnostic.

### D14 — SixNG harness build artifacts are inside the repo and not ignored — **NEW, OPEN**

`scripts/sixng-reference/target/` exists with a full release build tree.
`.gitignore` ignores only `/target/` (repo root), `/work/` and binary
extensions, so this generated tree would be committed. It also embeds local
paths and is exactly the "raw benchmark artifact" class that `AGENTS.md`
requires to stay out of Git. Fix: add `/scripts/sixng-reference/target/` (or
`**/target/`) to `.gitignore`. The harness itself is fine: it depends on the
pinned SixNG git revision, not a vendored copy.

---

## 4. Release-gate blockers (product level)

1. **BC1 accuracy oracle.** Needed before "equal or better accuracy": original
   BC1 blocks captured at encode time, or a GPU-decode comparison of the
   reconstructed blocks. Absent that, the fitted label stays advisory (it does).
2. **SixNG comparison and recorded end-to-end evidence.** The reported
   989/87 DDS match and 27x `fullDDS` result need a committed methodology
   (sample selection, hashes, cold/warm, guard overhead included) and a SixNG
   arm. A single-texture timing is a useful data point, not the gate.
3. **Exhaustive/sampled corpus validation.** D3 (packed extents), descriptor
   parity for all 57 stores, and BC3–BC7 chunk equality remain sampled or
   synthetic.
4. **Layout/DDS/PNG parity on real data.** Currently synthetic parity only.
5. **Batch/CLI operability.** Resume exists; CLI coverage and docs (D8, D11)
   are the remaining "usable CLI" gaps.
6. **Explicit unsupported scope.** PS5/PC adapters are not implemented; keep
   that statement in release text and keep unknown codecs failing explicitly
   (tested).

---

## 5. Open questions

**Q1. ETPF fixture offset.** `catalog.rs` reads ETPF at the 256-aligned
`metadata_end`; `tests/reader.rs` writes `ETPF1234` at the raw table end
(offset 40). The catalog reading now matches the documented reservation — please
reword or move the reader filler so the repository has one story.

**Q3. Policy naming.** `AllowFittedBc` also permits codec 1; confirm it is the
intended "all fitted" spelling and decide whether the CLI needs a BC1-only flag.

**Q6. BC1 accuracy authority.** Who defines and signs off the reference for
"equal or better accuracy"?

**Q7. Output naming.** Confirm the keyed/keyless file-name contract used by
`export-all` for downstream consumers.

**Q8. Allocation slack.** Is `base + layout.total_bytes == next_base` true for
every supplied store (D13), or should it be a diagnostic?

Answered this round: Q2 (sampled exact extents, D3), Q4 (decoded-work cap now
exists), Q5 (metadata prefix intended).

---

## 6. Residual risks not covered by these tests

* **Timestamp-preserving in-place edits.** The guard is identity + length +
  modified/created (`src/source.rs`). A writer that restores mtime, or a
  coarse-resolution filesystem landing two writes in one tick, defeats it. A
  cheap digest of the header/table captured at open would close this.
* **Test timing assumption.** The in-place test sleeps 20 ms so the NTFS
  timestamp moves; it would need adjusting (not weakening) on coarse filesystems.
* **Concurrent sessions.** Two `Store`s on the same path are independent by
  construction but untested under interleaved refresh/read.
* **Real layouts.** Streaming cubemaps, volume micro-tiles and odd-size crops
  rest on synthetic parity tests.
* **Preview channel policy** for typeless/unsigned-float variants (D10).
* **Memory transients.** `texture_dds` holds subresource vectors, the DDS
  payload and the final buffer at once (≤ ~3x the 256 MiB cap).

---

## 7. Reproduction

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test independent
rustfmt --edition 2024 --check tests/independent.rs
```

`tests/independent.rs` needs no corpus, environment variables or network; it
builds its own stores, synthetic BC1 entropy streams from the pinned
`bcpack-rs` revision, and a synthetic ETPF catalog.

---

## 8. Reviewed snapshot (first 16 hex digits of SHA-256)

```
lib.rs          4A2AB24F7E01AFDA
source.rs       18F40EDC7A259FB1
catalog.rs      1C7A276D9D08384C
extract.rs      29CAF3577B7F3A1A
dds.rs          1A407F4D626ACBDF
preview.rs      50EEDBA82D8827DD
output.rs       2839A45E09AF937D
main.rs         67A997F206DEFDBE
independent.rs  4ABF5A371542881F
```

If any of these change, re-run section 7 and re-check D1–D14 before trusting
this assessment.

No commits, no publication, and no edits outside this task's owned files were
made.
