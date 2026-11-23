# BC7 preset update reference

Separate GPL-3.0-only validation harness pinned to unmodified SixNG
`d724355df2398a646eb771d6d3da17c3b1bafe11` (PR #31). It is not linked into
the SDK. The older complete-corpus reference stays pinned independently.

Input is a JSON array of groups:

```json
[{"path":"<owned-store.ttpf>","chunks":[{"index":0,"start":0,"len":16384,"old_sha256":"<old-hash>"}]}]
```

`start` is the neutral RDS5 chunk offset; this adapter adds SixNG's 4096-byte
header page. Open each store once, decode without a result cache, hash each
chunk outside the read timer, and write JSONL to stdout. No output blocks are
retained. Keep manifests and reports outside the repository.

```powershell
cargo run --release --locked --manifest-path scripts/bc7-current-reference/Cargo.toml -- <manifest.json> > <reference.jsonl>
python scripts/verify_bc7_presets.py --manifest <manifest.json> --reference <reference.jsonl> --out <report.json>
```

The verifier requires complete reference coverage, exact byte hashes and
lengths, zero skipped/unfitted/invalid blocks, zero decoded cache residency,
provisional quality labels, and refusal by the built-in confirmed policy.
Use `--before` with the preserved older wheel to verify the manifest's old
hashes and count its missing presets. This is fitted compatibility evidence,
not independent initial-history confirmation. Single-pass read timings are
operational fields; use a separate alternating benchmark for speed claims.
