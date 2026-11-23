# Classic BC history: port review and integration notes

Owned files for this task:

| File | Role | SHA-256 |
|---|---|---|
| `src/history_bc.rs` | BC1/BC3/BC4/BC5 rebuilders, schema validation, module tests | `C0FD93F4A41B622EC9A8028B47BBE0AEF54A233EAA073A30011469F05CBE79C5` |
| `src/history_bc_tables.json` | Bundled codebooks and built-in history (50,643 bytes) | `2A6846EB9012783E672D9DEE8D1F7895EE02A51762108FA6C838B5690E763159` |
| `tests/history_bc.rs` | Public `HistoryTables` parsing/validation/digest tests | `47CB4275AB5975D0A0591B24A556724E10F3456E25A9533ADAC007F7E003073D` |

No production source, `src/lib.rs`, existing tests or other workers' files were
edited. Synthetic fixtures only; no game data, store files or decoded output are
in the repository.

## 1. Source and licence

* Leonida `tools/bcpack.py` and `tools/bcpack_tables.py`, revision
  `aa2988cc555bad7ecd508b48a702adbb32fe7c95`
  (<https://github.com/ItsNotRexed/Leonida>). Leonida's README states
  "GPLv3 (or any later version)"; the combined project remains GPL-3.0-only and
  the port keeps the attribution here and in the module header.
* `bcpack_tables.py` describes its constants as codec-level values "recovered
  clean-room from public specifications (US patent 11,234,023 and BCn block
  formats)". The bundled JSON contains only those constants: symbol-rank tables
  and initial endpoint history. It contains no texture payloads.
* The entropy layer, rANS models and lane scheduling are **not** ported here.
  `HistoryTables::decode` receives already-decoded per-lane symbols from the
  parent's `bcpack::rans` path and only rebuilds blocks.

## 2. What was ported

| Codec | Lanes | Symbols per block | Endpoint moduli | Codebooks |
|---|---|---|---|---|
| BC1 | 12 | 1 | `32,64,32,32,64,32` | 256-byte quadrant rank table |
| BC3 | 12 | 2 | `256,256,32,64,32,32,64,32` | 64 alpha pairs, 256 colour quadrants |
| BC4 | 12 | 1 | `256,256` | 64 index pairs |
| BC5 | 12 | 2 | `256,256,256,256` | 64 index pairs |

The rebuilders reproduce Leonida's arithmetic exactly:

* `unzig` zig-zag lane deltas;
* endpoint updates modulo the per-component bit width (`rem_euclid`, matching
  Python's non-negative `%`);
* backward references `d` into the current chunk, with `d == 0` selecting the
  codec's zero base (`(0,…)`, `(128,128,…)`, `_BC3_NONE`);
* `_inter8` for BC3/BC5 lane-0 pairs;
* rank-table index packing (`12` bits per 3-bit pair group, little-endian
  output) and the shared BC1/BC3 colour-quadrant byte order;
* the built-in confirmed and provisional history maps, kept separate.

Built-in table sizes after validation:

| Codec | Confirmed | Provisional |
|---|---|---|
| BC1 | 252 | 3 |
| BC3 | 0 | 35 |
| BC4 | 2 | 170 |
| BC5 | 5 | 209 |

This is the correction for the existing BC1 disagreement: the bundled
definitions come from Leonida, not from the older decoder paths. Legacy BC1
decoding elsewhere is untouched.

## 3. External document schema

`Tables::new(history)` accepts `None` or a JSON object. A full document uses
codec keys; a single-codec section may carry `"codec": "bc1"` (and the parent's
`normalize` also wraps such documents under the matching key).

```json
{
  "bc1": {
    "codec": "bc1",
    "replace": false,
    "confirmed": { "1": [7, 29, 6, 8, 30, 7] },
    "provisional": { "6": [12, 24, 11, 7, 14, 6] }
  }
}
```

* `confirmed` and `provisional` are either a dense 256-entry array or a sparse
  object keyed by decimal index `0..255`. Each row has exactly the codec's
  component count and every value is in `0..modulus-1`.
* `replace` defaults to `false`. Overlay merges confirmed entries over the
  built-ins, adds external provisional entries, then removes every entry that
  is confirmed. Replace uses only the external maps, then removes confirmed
  entries from provisional.
* A confirmed entry always wins over a provisional entry, regardless of source.
* Unknown keys are ignored, matching `load_bc_history`. Wrong types, wrong row
  lengths, out-of-range indices/values, non-integer numbers, a mismatched
  `"codec"` marker or a non-boolean `replace` are `InvalidInput`.
* No path is accepted and no file is opened. The parent owns `from_path` and
  passes a parsed document.

Parsing happens once per `Tables::new`. The bundled constants are parsed once
per process behind `OnceLock`; codebooks are shared by reference, and only the
two 256-entry maps are cloned per snapshot. Decoding never parses JSON.

## 4. Strictness and error mapping

| Condition | `io::ErrorKind` |
|---|---|
| Codec not 1/3/4/5 | `Unsupported` |
| Unconfirmed history entry with `allow_provisional == false` | `Unsupported` |
| Unknown history entry (absent from confirmed **and** provisional) | `Unsupported` |
| Wrong lane count/length, symbol wider than its lane, non-zero reserved lane 1, size overflow | `InvalidData` |
| Malformed history document/table | `InvalidInput` |

`allow_provisional` permits only *known* provisional entries and records each
use as `"BCx history <index>"` in `Rebuilt::provisional` (sorted, unique). It
never invents a placeholder for an unknown entry. This is deliberately stricter
than Leonida's BC3/BC5 provisional mode, which substitutes a zero-like base for
a bounded number of unknown roots. The differential run below counts those
cases explicitly: 135 fixtures where Python produced placeholder output and
this module returned `Unsupported`.

## 5. Table export

`src/history_bc_tables.json` was generated by importing only
`bcpack_tables.py` from the pinned Leonida checkout. The script validates every
rank, history key, component count and modulus before writing. It was run from
outside the repository (temp directory) so no extra file enters the project;
the full script is reproduced in the appendix.

The JSON is the build input for `include_str!`; `history.rs` hashes it (with
`history_bc.rs` and the decoder identity) into the public snapshot digest.

## 6. Verification

Module unit tests (`src/history_bc.rs`, 11) use synthetic lanes directly:

* BC1 zero-delta base and rank-0 quadrants;
* BC1 confirmed entry 1 used verbatim;
* BC1 provisional entry 6: strict rejection, opt-in success, label;
* replaced history creating an unknown entry: rejected in both modes;
* BC1 overlay versus replace: override applies, replacement drops defaults;
* BC4 confirmed entry 52 plus pair-rank packing;
* BC3 provisional entry 2: strict rejection, opt-in success, colour packing;
* BC5 confirmed entry 105 for both channels;
* BC5 provisional overlay label and replacement dropping defaults;
* lane count, lane length, symbol width and reserved-lane validation;
* malformed external sections (wrong width, index, modulus, marker, `replace`).

Public-API tests (`tests/history_bc.rs`, 7) cover the bundled digest stability,
overlay/replace digest changes, dense-versus-sparse documents, single-codec
documents, boundary values for all four codecs, malformed documents, and
file/directory snapshot loading.

Randomized differential check against Leonida's Python `_bc1_blocks`,
`_bc3_blocks`, `_bc4_blocks` and `_bc5_blocks` (scratch harness outside the
repository, deterministic seed, synthetic lanes only):

```
cases=3020 exact=1733 errors_matched=1152 unknown_rejected=135
```

* `exact`: identical block bytes and identical provisional labels.
* `errors_matched`: both sides rejected the fixture (history dependency).
* `unknown_rejected`: Python fabricated an unknown BC3/BC5 placeholder; this
  module returned `Unsupported`, as required.
* The suite includes every history index `1..255` for all four codecs under a
  complete replacement table, plus multi-block random, overlay and partial
  replacement cases.

Repository test status at this snapshot: `cargo test` passes **128 tests**
(40 lib including these 11, 7 `history_bc`, 7 `bc6h_accuracy`, 10
`history_bptc`, 4 `history_session`, 19 independent, 17 layout, 12 preview
formats, 8 reader, 2 uncached, 2 CLI).

Formatting/lint status: the final repository gate is green at this snapshot:
`cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean,
`cargo test` 128 passed, and `cargo build --release` succeeds. The two
previously outstanding items in `src/preview.rs` and `src/history_bptc.rs` were
resolved by their owners; this task did not edit those files.

## 7. Integration notes for the parent

* `HistoryTables::from_json` hashes canonical JSON as supplied. The parent has
  fixed the identity contract: ordering and whitespace are canonical, but a
  dense array and a sparse object may produce different digests. That is
  deliberate and conservative (a replacement forces a cache reset rather than a
  false hit), not a semantic identity. Cache-correctness tests cover actual
  content changes with preserved mtime and unchanged JSON ordering, so no broad
  digest normalization is planned.
* The Store path passes `out_bytes / block_bytes` blocks and 12 lanes; both are
  validated before allocation. `Rebuilt.bytes.len()` is always
  `nblocks * block_bytes`.
* `Rebuilt.provisional` is a sorted unique list of labels; the Store only needs
  `len()` for `Quality::HistoryBc::provisional_dependencies`.
* BC6H/BC7 are not handled here. `decode` returns `Unsupported` for codecs 6
  and 7 so the parent's `history_bptc` path stays authoritative.
* No real-corpus BCPack stream was exercised in this task; the entropy layer and
  store integration are the parent's scope. The differential oracle is public
  Python source plus synthetic lanes, not game data.

## 8. Open items

1. Keep the 135-case unknown-history divergence explicit in parent tests: a
   provisional-mode decode must never silently substitute unknown roots.
2. When Leonida moves, regenerate the JSON and re-run the differential script;
   the module header and this document pin `aa2988cc`.

## Appendix: exporter

```python
"""Export Leonida's public BC1/BC3/BC4/BC5 codec constants to rage-texture JSON."""
import json
import sys

sys.path.insert(0, r"...\Leonida\tools")
import bcpack_tables as t  # noqa: E402

REVISION = "aa2988cc555bad7ecd508b48a702adbb32fe7c95"
OUT = r"...\rage-texture\src\history_bc_tables.json"

MODS = {
    "bc1": [32, 64, 32, 32, 64, 32],
    "bc3": [256, 256, 32, 64, 32, 32, 64, 32],
    "bc4": [256, 256],
    "bc5": [256, 256, 256, 256],
}


def check_map(name, table, mods):
    out = {}
    for key, value in table.items():
        assert 0 <= key < 256, (name, key)
        assert len(value) == len(mods), (name, key, value)
        for v, mod in zip(value, mods):
            assert 0 <= v < mod, (name, key, value, mod)
        out[str(key)] = list(value)
    return dict(sorted(out.items(), key=lambda kv: int(kv[0])))


def check_bytes(name, seq, expected):
    values = list(seq)
    assert len(values) == expected, (name, len(values), expected)
    assert all(0 <= v < 256 for v in values), name
    return values


def check_pairs(name, seq, expected):
    values = [[int(a), int(b)] for a, b in seq]
    assert len(values) == expected, (name, len(values), expected)
    assert all(0 <= a < 8 and 0 <= b < 8 for a, b in values), name
    return values


doc = {
    "format": "rage-texture/classic-bc-history-v1",
    "source": "Leonida tools/bcpack_tables.py",
    "source_revision": REVISION,
    "license": "GPL-3.0-or-later",
    "note": (
        "Codec-level constants only: symbol-rank tables and built-in initial endpoint "
        "history. Confirmed and provisional entries stay separate; unknown entries are "
        "never fabricated."
    ),
    "codecs": {
        "bc1": {
            "mods": MODS["bc1"],
            "quadrant": check_bytes("BC1_QUADRANT", t.BC1_QUADRANT, 256),
            "confirmed": check_map("BC1 confirmed", t.BC1_HISTORY_CONFIRMED, MODS["bc1"]),
            "provisional": check_map("BC1 provisional", t.BC1_HISTORY_PROVISIONAL, MODS["bc1"]),
        },
        "bc3": {
            "mods": MODS["bc3"],
            "alpha_pair": check_pairs("BC3_ALPHA_PAIR", t.BC3_ALPHA_PAIR, 64),
            "colour_quadrant": check_bytes("BC3_COLOUR_QUADRANT", t.BC3_COLOUR_QUADRANT, 256),
            "confirmed": check_map("BC3 confirmed", t.BC3_HISTORY_CONFIRMED, MODS["bc3"]),
            "provisional": check_map("BC3 provisional", t.BC3_HISTORY_PROVISIONAL, MODS["bc3"]),
        },
        "bc4": {
            "mods": MODS["bc4"],
            "pair": check_pairs("BC4_PAIR", t.BC4_PAIR, 64),
            "confirmed": check_map("BC4 confirmed", t.BC4_HISTORY_CONFIRMED, MODS["bc4"]),
            "provisional": check_map("BC4 provisional", t.BC4_HISTORY_PROVISIONAL, MODS["bc4"]),
        },
        "bc5": {
            "mods": MODS["bc5"],
            "pair": check_pairs("BC5_PAIR", t.BC5_PAIR, 64),
            "confirmed": check_map("BC5 confirmed", t.BC5_HISTORY_CONFIRMED, MODS["bc5"]),
            "provisional": check_map("BC5 provisional", t.BC5_HISTORY_PROVISIONAL, MODS["bc5"]),
        },
    },
}

with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
    json.dump(doc, fh, indent=1, ensure_ascii=True)
    fh.write("\n")
```
