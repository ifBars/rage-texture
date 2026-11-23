# BC6H/BC7 real-history evidence and remaining gate

Owner: real-history research and comparison scripts. Owns
`scripts/trace_history.py`, `scripts/verify_bptc_real.py`,
`tests/history_bptc.rs` and this file only. No game data, decoded bytes, raw
reports or local store paths are committed. All real runs are read-only and
write reports outside the repository.

## 0. Verdict

**Confirmed pixel accuracy is still open.** No genuine confirmed BC6H/BC7
history table has been found in public sources. The available public tables are
fitted. This work establishes:

1. **Provisional real coverage:** 73 real BC7 chunks and 11 real BC6H chunks
   rebuilt byte-identically by the frozen native build and unmodified Leonida
   using explicitly provisional external history. This is real
   API/JSON/algorithm coverage, not a confirmed-accuracy oracle.
2. **Structural dependency inventory:** 1,192 real BPTC chunks sampled across
   46 stores (bounded at 32 per codec per store) contain **zero** whole chunks
   that decode without initial history, and **zero** chunks whose consumed
   history bits are fully determined by anchor/p-bit constraints alone.
3. **BC6H root cause:** SixNG's packer truncates anchor MSB bits instead of
   rejecting inconsistent approximate history. Its "valid" BC6H blocks can
   therefore hide bad initial indices, while Leonida correctly refuses the same
   data (3,321 anchor violations on the first lighting chunk). This is not a
   Leonida or native decoder bug.
4. **Converter projection bug:** SixNG keeps all BC6H endpoint state fields
   16-bit MSB-aligned; Leonida expects `w` 16, `x` 10, `y`/`z` 6. A converter
   must shift `x >> 6` and `y`/`z` >> 10 after transposing endpoints. The
   current 7024 converter does not; the parent is correcting it.
5. **Confirmed-depth bug fixed by parent:** native BC6H used the R channel's
   delta width for all channels when checking confirmed endpoint bits. The
   parent changed it to per-channel widths and added a mode-4 regression test.
   This changes decoder identity; the old frozen binary remains valid only for
   the old-pin run recorded below.

## 0.1 Handoff freeze

- Chosen BC6H profile: Leonida self-calibration `--codec bc6h --limit 32
  --max-blocks 256`, lighting + props_0_3, 42 pool chunks, 32 textures. Result:
  64 scanned, 45 candidates, **11 equal, 34 both refused, 19 prefilter skips**,
  0 mismatch. No further calibration.
- BC7 old converter run: **73 equal over 143 scanned chunks**, 52 both refused
  overall, frozen binary and pinned 7024 JSON. Valid for that pinned pair only.
- Strict per-channel confirmed-depth regression is present in
  `tests/history_bptc.rs`; the history test suite is 15/15.
- Pending parent rerun: new frozen binary plus current JSON (SixNG 7881
  additions) and the chosen calibration-32 profile. Do not reuse the old binary
  hash as current identity.
- Scripts, tests and this document are frozen at the counts above. Reports and
  profiles stay outside Git.

## 1. Bounded real dependency inventory

`scripts/trace_history.py --root <ttdb> --codec 6,7 --chunks-per-codec 32`

- 46 stores, 1,192 chunks, 305,152 prefix blocks, 299,918,022 packed bytes,
  14.0 s.
- Prefix limit 256 blocks. Every direct initial-history reference is in blocks
  0..255, so the inventory is complete for initial dependencies; trailing
  in-chunk chains are not counted.
- 26 of 46 stores had more chunks than the 32-per-codec cap. The run is
  explicitly incomplete at corpus scale.
- **Self-contained chunks: 0.**
- Required initial bits observed: 301,288 endpoint bits and 2,534,208 index
  bits.
- Bits resolved by anchor/p-bit constraints alone: **4,634 index bits
  (0.18%)**, **0 endpoint bits**. No chunk is fully resolved.
- With the pinned 7024 SixNG table as a **prefilter only**, 1,099 chunks have
  present data for every consumed slot and 93 do not. A prefilter hit is not a
  decode proof; the bounded comparison below performs the actual gate.

Anchor/p-bit constraints narrow some index high bits but cannot determine
endpoint fields, and the index survivors are usually not singleton. Exact
anchor/p-bit constraints alone therefore cannot close the confirmed gate.

## 2. Real external provisional comparisons

All rows are **provisional**: the history files declare zero confirmed bits.
Native and Leonida must both accept and produce the same bytes; a mutual refusal
is reported as a refusal, never as success.

### 2.1 Pinned SixNG 7024 fitted tables

- History: `sixng-provisional-history.json`, SHA-256
  `2671db39e3c4a9e32e664a60f3628fe1de1020373fe51d31391179038febd772`.
- Embedded provenance: source revision
  `702435cd56fd09de4f456ddc94d60886de7365ee`, target revision
  `aa2988cc555bad7ecd508b48a702adbb32fe7c95`, `confidence:
  provisional-fitted`, `confirmed_bits: 0`, 209 BC6H endpoint / 250 BC6H index /
  214 BC7 endpoint / 255 BC7 index known entries.
- Native: `release-blockers-native.exe`, SHA-256
  `299055ddcda8ae5ea4b23b94fb5aa9a29e6cd2ada430d254fbf2ab4330a57e87`
  (frozen before the per-channel depth fix).
- Bounded runs: 8 representative stores, 16 chunks per codec per store, decoded
  size cap 4 MiB, elapsed 157 s.

| Codec | Scanned | Prefilter candidates | Equal | Both refused | Mismatch |
|---|---:|---:|---:|---:|---:|
| BC6H | 36 | 34 | 0 | 34 | 0 |
| BC7 | 107 | 91 | 73 | 18 | 0 |
| Total | 143 | 125 | 73 | 52 | 0 |

No byte mismatches, no label-presence mismatches, no decoded-cache use. BC7
success is real external-history coverage. BC6H success is zero with the old
fitted index table; the first lighting chunk reproduces the parent's 3,321
anchor violations.

### 2.2 Leonida self-calibration BC6H profile

Bounded calibration from lighting + props_0_3 (`--codec bc6h --limit 32
--max-blocks 256`, 42 pool chunks, 32 textures) produces only provisional
classes 1/2 with zero confirmed bits and anchor-consistent index voting.

| Run | Scanned | Candidates | Equal | Both refused | Mismatch |
|---|---:|---:|---:|---:|---:|
| Lighting | 32 | 30 | 11 | 18 | 0 |
| Props_0_3 | 32 | 15 | 0 | 16 | 0 |
| Total | 64 | 45 | 11 | 34 | 0 |

Elapsed 48 s. **The 32-texture profile is the chosen handoff profile; no further
calibration runs are planned.** A larger 48-texture profile produced 10/32
lighting successes, not more; it is diagnostic only. These 11 chunks are genuine
real BC6H provisional successes with an independently generated (Leonida
calibration) profile. They do not establish confirmed values.

### 2.3 Aligned-projection probe

`bc6h-aligned-probe-history.json` applies the correct `x >> 6`, `y`/`z` >> 10
projection to the pinned 7024 endpoint values while keeping the approximate
SixNG index tables. Bounded BC6H comparison over lighting + props_0_3, 16
chunks per store, 32 scanned, 32 prefilter candidates:

- 0 equal, 32 both refused, 0 mismatch, 0 label or cache issues, 49 s.

This supports the diagnosis: fixing endpoint projection alone does not make
the approximate index tables anchor-consistent.

### 2.4 BC6H diagnosis details

- Rebuilding the first lighting chunk with SixNG state tables gives 3,321
  anchor-inconsistent indices. SixNG still emits a block because its packer
  clears/truncates the anchor MSB; the native/Leonida guard is correct to
  reject.
- Masking the fourth deinterleaved index bit before `unzig` (the suspected
  width bug) left the count at exactly 3,321, and also changed nothing with the
  corrected endpoint projection. That hunch is **not proved**.
- The endpoint projection issue is real and separate: SixNG stores all fields
  16-bit MSB-aligned, so Leoni `x` must shift right 6 and `y`/`z` right 10.
  Fixing projection alone did not make the approximate SixNG index table
  anchor-consistent.
- Correct response: keep the structural anchor guard, use anchor-consistent
  calibrated indices, and label the result provisional.

## 3. Public-source search for confirmed tables

Live search found no independently confirmed BC6H/BC7 history tables:

- `DesignWithTommy/6to5m` bundles `bcpack-history-v1.bin` from a
  Leonida-compatible self-calibrator; it is fitted, not confirmed, and
  incomplete for BC6H.
- `TrinitronClient/GTA6-Asset-Info` (690a3636) states byte-exact reconstruction
  is not established for any codec; BC3–7 are fitted.
- `Tim0219800/bcpack-decoder` implements BC1 only.
- `R0Y00/Xbox2Windows` has a synthetic BC7 mode-6 golden vector, not a real
  oracle.

The parent also checked 14 real raw/zlib BC6H textures independently with
`bcdec` (496,640 blocks, 23,838,720 half values exact, 2 unavailable). That is
an independent plain-BC6H decoder check, not a BCPack initial-history oracle.

## 4. Masked visible-mip feasibility (not implemented)

A visible mip or slice could begin inside a chunk at a block whose endpoint and
index chains are self-contained even though the whole chunk is not. A masked
block-range API could then expose that region without external history.

This remains unproved and was not scanned. The whole-chunk inventory found no
self-contained chunk, so feasibility requires a block-to-visible-mip/slice map
plus a suffix analysis of each visible range. No core API change is proposed
here; do not count this as available coverage.

## 5. Scripts

### `scripts/trace_history.py`

Read-only bounded dependency inventory. Accepts `--store`/`--root`, `--codec`,
`--chunks-per-codec`, `--prefix-blocks`, optional `--history`, and an `--out`
report outside the repository. Emits per-store counts, prefilter coverage,
incomplete status, elapsed time, and the exact bit-resolution totals above.
The embedded provenance of the history JSON is carried into the report; a
caller label is only an annotation.

### `scripts/verify_bptc_real.py`

Bounded native-vs-Leonida byte comparison. Accepts `--leonida`, `--exe`,
`--store`/`--root`, `--history`, `--codec`, `--chunks-per-codec`,
`--prefix-blocks`, `--max-decoded-bytes`, and `--out` outside the repository.

- Prefilter candidates are selected by present data, then both decoders must
  accept and agree byte-for-byte.
- Reports `scanned`, `prefilter_candidates`, `skipped_prefilter`,
  `both_refused`, `python_refused`, `native_refused`, `acceptance_mismatch`,
  `equal`, `mismatch`, `label_presence_mismatch`, `cache_used`, errors, elapsed
  and bounded/incomplete status.
- Refusals are not success. The default exit status fails when there is no
  byte-equal case; `--inventory-only` is required to accept an inventory-only
  run.
- Provenance includes the history hash, embedded `_provenance` (source and
  target revisions, confirmed bits), native hash, baseline Leonida revision and
  store list. It never relabels provisional data as confirmed.

## 6. Remaining blockers

1. **No confirmed initial-history oracle.** All public tables are fitted. The
   confirmed pixel-accuracy gate remains open.
2. **BC6H real coverage is partial and provisional.** 11 lighting chunks pass
   byte equality with a self-calibrated profile; props_0_3 produced none in the
   bounded sample.
3. **BC7 coverage is stronger but still provisional.** 73/91 candidates match
   with pinned fitted tables; 18 are mutual refusals.
4. **Corpus inventory is incomplete.** 26 stores were capped at 32 chunks per
   codec; no full-corpus claim is made.
5. **Current fits remain provisional.** SixNG 7881 adds 18 BC6H and 13 BC5
   endpoint presets relative to 7024. The SDK has synchronized them; current
   BC7 external-table validation reproduces 73 successes. BC6H converter output
   now projects endpoint widths to 16/10/6/6; self-calibrated real coverage
   remains separate from fitted-table compatibility.
6. **Whole-chunk history dependence is universal** in the bounded sample. No
   strict no-history real BC6H/BC7 success exists.

## 7. Reproduction

```text
python -I scripts/trace_history.py --leonida <Leonida> --root <ttdb> \
  --codec 6,7 --chunks-per-codec 32 --history <provisional.json> \
  --out <outside-repo>/trace.json

python -I scripts/verify_bptc_real.py --leonida <Leonida> --exe <native> \
  --store <store.ttpf> ... --codec 6,7 --chunks-per-codec 16 \
  --history <provisional.json> --out <outside-repo>/verify.json
```

Game files are read-only inputs. Reports and any generated profiles remain
outside Git.

## Final integration verification

The final SDK rerun reproduces 11 BC6H matches from the bounded calibration-32
profile (64 scanned, 34 mutual refusals, 19 prefilter exclusions) and 73 BC7
matches from current 7881 fitted tables (107 scanned, 18 mutual refusals,
16 prefilter exclusions). All successful outputs match unmodified Leonida,
with matching label presence and zero decoded cache use. No original-pixel
confidence is inferred from these provisional successes.

The parent also corrected the native BC6H confirmed-depth check: each colour
channel needs its own delta width for every consumed delta endpoint. A failing
synthetic regression demonstrated the previous false refusal; the repair also
refuses a weak green field. Unmodified Leonida independently agrees with both
policy outcomes.
