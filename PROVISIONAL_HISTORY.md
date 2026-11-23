# Exporting provisional history tables

`examples/export_provisional_history.rs` exports the public SixNG fitted
BC6H/BC7 tables already bundled in this SDK, in Leonida-compatible JSON.
It reads no game files. Source revision:
`7881042c294f3eeea1e8153e7bcafd84f9d787b9` (GPL-3.0-only).

```text
cargo run --release --example export_provisional_history -- --out fresh-history.json
cargo test --example export_provisional_history
```

The output path is created exclusively; existing files are preserved. Without
`--out`, JSON goes to stdout. `--compact` omits indentation. The document
includes source/schema revisions and confidence metadata under `_provenance`.

| Codec | Present endpoint entries | Present index entries |
|---|---:|---:|
| BC6H | 226 | 251 |
| BC7 | 214 | 255 |

BC6H source states are 16-bit MSB-aligned for every endpoint. Leonida's schema
uses 16/10/6/6-bit fields, so each output channel is:
`[w, x >> 6, y >> 10, z >> 10]`. The index state is already four-bit aligned.
BC7 states flatten to 24 endpoint values and 32 index values.

All entries remain provisional. BC6H class is 1; confirmed-bit rows are zero.
BC7 stored depths are 8/4, while confirmed depths remain zero. Absent entries
are omitted and remain unknown. Stored depth describes the supplied fitted
value, not its accuracy. The 14 example tests include the vendored codec tests
and converter invariants; synthetic checks also exercise strict refusals and
provisional dependency labels through Leonida.

Use the output with `--history PATH --allow-provisional`, or pass it to Python
`Store(..., history=PATH, decode_policy="provisional")`. Strict mode refuses
consumed fitted entries. Unknown values remain refused in provisional mode.

These tables are not an original-pixel oracle. Real BC7 external-history
comparisons succeed on the bounded measured sample. BC6H fitted presets can
fail anchor checks even when every referenced entry exists; the successful
real BC6H comparisons use a locally self-calibrated Leonida profile instead.
Do not remove structural checks to make fitted output pass.

See [HISTORY.md](HISTORY.md), [HISTORY_BPTC_REVIEW.md](HISTORY_BPTC_REVIEW.md)
and [PUBLIC_HISTORY_RESEARCH.md](PUBLIC_HISTORY_RESEARCH.md) for the scope and
remaining confirmed-accuracy limitation.
