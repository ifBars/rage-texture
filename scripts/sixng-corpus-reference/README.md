# Independent SixNG chunk reference

This separate GPL validation crate links pinned SixNG, not `rage-texture`.
It parses and decodes every real chunk, including chunk 0, with no decoded
cache or automatic reuse of prior results. Unknown codecs, decode failures
and wrong decoded lengths are recorded as failures.

Build with Cargo and provide a JSON array of absolute paths to your own
read-only TTPF stores:

```text
cargo build --release --locked --manifest-path scripts/sixng-corpus-reference/Cargo.toml
scripts/sixng-corpus-reference/target/release/sixng-corpus-reference \
  --manifest <external>/stores.json --out-dir <external>/sixng-audit
```

On Windows, append `.exe`. The output directory must be fresh, outside the
input root and outside this repository. Output includes one JSONL file per
store and `run-summary.json`; it contains local paths, so keep it external.

Compare each `store_done.aggregate_sha256` with the native auditor's
`store_done.chunk_aggregate.sha256`. Both hash successful decoded bytes
concatenated in numeric chunk-table order, without headers. Also compare
counts, byte lengths and failure indices. An equal digest with incomplete
coverage is not a full-corpus pass.

The reference preserves SixNG's zlib behavior; it does not add the SDK's
strict trailing-input validation. Source stamps and table digests are
diagnostics, not content identities, and are never used to skip work.
Pixel decoding and BC initial-history accuracy are separate checks.
