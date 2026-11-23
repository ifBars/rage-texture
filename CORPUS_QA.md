# Whole-corpus compatibility QA

Checked November 22, 2026 against the supplied Xbox v1.0.2.1 build.
Reports and sampled game-derived files remain outside this repository.

## Completed exhaustive stage (SixNG 7881042)

| Check | Result |
|---|---:|
| Stores | 57 / 57 |
| Chunks decoded | 1,026,833 / 1,026,833 |
| Ordered store digests equal pinned SixNG | 57 / 57 |
| Decoded chunk bytes compared | 151,568,699,392 |
| Full DDS exports and independent container checks | 357,115 / 357,115 |
| DDS bytes constructed | 149,464,150,932 |
| Decode, export, container or source-change failures | 0 |
| Decoded-cache hits, residency and peak residency | 0 |
| Observed DXGI formats | 20 |

The layouts comprise 356,418 2D textures, 127 arrays, 567 cubemaps and
three volumes. Every descriptor was exported with its full mip/slice chain.
The audit retained only 49 representative DDS files (32,239,104 bytes);
it did not retain the complete exported corpus.

Chunk counts: stored 5, zlib 918,223, BC1 102,725, BC3 35, BC4 77,
BC5 333, BC6H 268 and BC7 5,167. The native and SixNG runs agree on
counts, byte lengths, codec counts, unfitted/invalid diagnostics and every
ordered decoded-byte digest. SixNG is pinned at
7881042c294f3eeea1e8153e7bcafd84f9d787b9.

## Current BC7 presets (SixNG d724355d)

SixNG PR #31 merged 34 additional fitted BC7 endpoint presets during the
audit. The SDK adopts those rows with attribution. All existing preset
values, indices and decoder code are unchanged. A source comparison confirms
that only `bc7pack_tables.rs` differs among the BC1/3/4/5/6/7 decoder/table and
TTPF reader files between the two pinned revisions.

A separate current-reference run and installed-wheel check cover **all 5,167
BC7 chunks across 51 stores**, 1,280,880,640 bytes. Every length and SHA-256
matches; zero skipped, unfitted or invalid blocks; zero decoded cache use.
The preserved old wheel reproduces every old chunk hash and all 839,119 BC7
unfitted flags. The new table changes 251 chunk hashes and removes those flags.
Both wheels refuse all 5,167 chunks under the built-in confirmed policy.

This is layered validation: the million-chunk audit and full DDS container
pass remain the frozen stage above; the update rechecks every affected BC7
chunk, not another complete-corpus export. The unchanged BC1 decoder retains
124,047 missing-preset flags. A missing-preset flag is not a pixel-error count.
No preset here becomes confirmed history.

Current CLI and wheel retain all 1,076 saved complete DDS references and
843/843 sampled preview mip/slices match combined SixNG layout/preview fixes
plus the d724 BC7 table. Rust: 161 tests; Python/report/adapter: 33 tests.
The new reference and verifier are in
[scripts/bc7-current-reference](scripts/bc7-current-reference/README.md).
Five additional gate tests reject empty, partial, duplicated, unexpected,
skipped and wrong-length BC7 reference records before opening a store.

Current artifact SHA-256 values:

- CLI: `e0dc31bf96401a026817674e0f1e66f2fdfe7a272f1212ca63441297bab922cf`
- Python extension: `03c9349c52fe261772c116e0f7a05a65c1dd1e9b2b2ccf782571de3dbf42fe01`
- BC7 table: `8bd7c74254792ac0e90b517956f29b7bfd3e76a72b0678d5e0e29e16d1b45364`
- Independent current reference: `f8fb992b57df9dea5ee4368be7aceabae1d7494492bc3fbf3d5f8093e79b54ef`

## Accuracy boundary of the frozen exhaustive stage

Both runs explicitly use fitted BCPack reconstruction. They report
963,166 unfitted blocks: 124,047 BC1 and 839,119 BC7; invalid blocks: zero.
These are missing-preset diagnostics, not an estimate of erroneous pixels.
All fitted reconstruction remains provisional, including blocks without
an unfitted flag. Byte equality with the same fitted implementation does
not independently establish original initial-history values.

A separate strict census attempted every real BC6H/BC7 chunk using only
the built-in confirmed histories. All 268 BC6H and 5,167 BC7 chunks were
refused for missing history. No fitted fallback was allowed.

The DDS container pass checks headers, format, shape, pitch and exact
subresource lengths. It is separate from pixel decoding. Current CLI and
Python exports also retain all 1,076 saved complete DDS reference matches.
The updated previews match 843/843 mip/slices against combined SixNG
layout, alpha and BC1/2/3 repairs. Independent DirectXTex coverage is in
[CONSUMER_QA.md](CONSUMER_QA.md).

## Reproduce

Run from the SDK root; replace the bracketed paths with external locations.
The full-stage commands below compare the 7881042 table profile. For that
historical stage, use that revision's `src/bc7pack_tables.rs` in a separate
SDK copy. The current table intentionally changes 251 chunk hashes, so it
must not be compared to the old reference as though nothing changed. Use
the separate BC7 verifier above for the current layered check.

    cargo run --release --locked --example audit_corpus -- --stores <owned-root> --out <external>/native.jsonl --phase both --policy fitted
    cargo run --release --locked --manifest-path scripts/sixng-corpus-reference/Cargo.toml -- --manifest <external>/stores.json --out-dir <external>/sixng
    python scripts/summarize_corpus.py --native <external>/native.jsonl --sixng <external>/sixng/run-summary.json --out <external>/parity.json

The store manifest is a JSON array of absolute paths. Use fresh output paths.
The auditor can optionally write bounded representatives with --samples.
Every store opens with a zero-byte decoded cache. DDS construction may reuse
temporary input bytes within one request; they are discarded at return.

The aggregate hashes successful decoded bytes concatenated in numeric chunk
order, including chunk 0. The reconciliation gate rejects failed, missing,
duplicated or unordered records, absent completion, cache use, differing store
sets, diagnostics or reference digests. Seven synthetic gate tests protect
against false passes. A partial run is never accepted as exhaustive coverage.
The independent reference has no automatic resumption or result cache.

## Snapshot and current-source validation

The full native run used frozen executable SHA-256
994dad38f216e65a8177f900e91125d5ae7959ca8f5ad40b1e3993cf331f1abe,
with auditor source 8474c8ed205f4aae0579d266f9ea811fe669b8d1186d5db1de4f955dc910d842
and consumer helper 36bc141d9d2f55ea822aa386bd66602446e166a805eeaad7c4f4e4e39b597b0d.
The independent reference executable is
b2507f08abb0bda969d2613a2b4ef67a4ee8ebba029b32d7c99cf83f934372c1.

Later auditor changes group aggregate state and distinguish confirmed from
provisional history in reports. A current-source mixed-codec smoke run
(46 chunks, 17 full DDS files) has identical chunk and DDS aggregates to the
frozen run. Two provenance tests cover the history classification.
The product's later BC1/BC3 preview fixes do not change DDS extraction.

Audit wall times include hashing, report writes and concurrent development;
they are not comparable extraction benchmarks. Separate alternating
measurements are in [BENCHMARKS.md](BENCHMARKS.md). Broader builds,
original-history confirmation and complete preview batching remain open.
