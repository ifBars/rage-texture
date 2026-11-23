# History-aware BCPack decoding

The Rust SDK, CLI and Python SDK now offer a Leonida-compatible history path
for BC1, BC3, BC4, BC5, BC6H and BC7. This is separate from the older SixNG
fitted path. A history entry marked confirmed is a statement about the supplied
table and consumed bits; it is not an independent original-pixel certificate.

## Policies and configuration

`DecodePolicy::ConfirmedOnly` refuses every consumed unconfirmed dependency.
`AllowProvisional` permits known provisional values and reports their labels.
Both refuse missing history values. The older `AllowFittedBc` and
`AllowFittedBc1` policies keep their existing behavior and diagnostics.

`HistoryTables::builtin()` returns an immutable shared snapshot. Use
`HistoryTables::from_json(bytes)` or `from_path(file_or_directory)` for local
tables, then `Store::open_with_history`. Files are read afresh when loaded;
there is no path/mtime lookup cache. Direct reconstruction is available through
`HistoryTables::decode_chunk`, including provisional dependency labels.

Combined JSON has `bc1`, `bc3`, `bc4`, `bc5`, `bc6h` and/or `bc7` sections.
Leonida single-codec documents and directories containing those codec-named
JSON files are accepted. Documents are capped at 8 MiB total. Malformed
sections, ambiguous numeric index aliases, and empty directories fail.

```powershell
rage-texture export pool.ttpf 0 texture.dds --confirmed
rage-texture export pool.ttpf 0 texture.dds --history D:/tables --mips 3
rage-texture range-hash pool.ttpf 4096 65536 --allow-provisional --history D:/tables
```

`--history` alone selects confirmed mode. Explicit policy flags cannot be
combined with `--allow-fitted`. CLI defaults remain lossless raw/zlib only.

```python
from rage_texture import Store

with Store("pool.ttpf", cache_bytes=0, decode_policy="confirmed",
           history="D:/tables") as store:
    texture = store.textures()[0]
    result = store.texture_dds(texture, mips=3)
    print(store.history_digest, result.quality)
```

Python `history` accepts a dictionary or path. A dictionary is copied into an
owned snapshot; mutating the original does not alter decoding. Use
`replace_history(new_dictionary_or_path)` explicitly to load changes. Quality
identifies `confirmed_history` or `provisional_history`, dependency count and
history digest. `invalid_blocks` is `None` for provisional history output,
because generic dependency labels do not establish a block-error count.

## Cache and descriptor identity

The digest includes canonical configuration JSON and the bundled decoder/table
source identity. Equivalent JSON whitespace/key order has the same digest;
different encodings can conservatively have different digests even if they
produce equivalent tables. Configured tables never mutate underneath a Store.
Replacing a changed digest clears decoded entries/counters and advances the
descriptor generation. Reload the catalog afterward. Identical digests retain
the generation and cache. A failed replacement preserves the previous snapshot.
`refresh()` reparses the source while retaining the selected history snapshot.

## Coverage

The built-in tables do not independently confirm the original BC6H/BC7 initial
histories for the tested build. Fitted decoder agreement is compatibility
evidence; it does not establish original-pixel accuracy. Missing dependencies
remain errors under the confirmed policy.
