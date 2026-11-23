# rage-texture Python bindings (experimental)

Optional, separately built PyO3 bindings for the parent `rage-texture` crate.
They wrap the existing read-only RDS5/TTPF session and extraction APIs. This is
an experimental API and is not published. Local correctness and performance
evidence, with its limits, is recorded in the parent BENCHMARKS.md. No game data
is included.

The combined project is GPL-3.0-only. See `LICENSE` and
`THIRD_PARTY_NOTICES.md` in this directory (copies of the parent notices).

## Why it is separate

The parent crate does not depend on PyO3. This directory is
its own Cargo package with `crate-type = ["cdylib"]`, its own `Cargo.lock`, and
a path dependency on `../..`. Maturin builds the wheel.

```powershell
cd bindings/python
uvx --from maturin==1.16.0 maturin build --release --locked --out dist
# Set this to the Python executable in your own isolated environment:
uv pip install --python D:/scratch/venv/Scripts/python.exe `
  --reinstall dist\rage_texture-0.1.0-cp310-abi3-win_amd64.whl
```

The wheel installs the mixed package `rage_texture` with the native module
`rage_texture._native`. Python 3.10+ is supported via the limited API
(`abi3-py310`); Windows links through `python3.dll`. The wheel includes the
package stubs and `py.typed`, its GPL license under wheel metadata, and
`rage_texture/THIRD_PARTY_NOTICES.md`. The notices are copies of the parent's
current notices; keep them in sync when the parent changes them. Only Windows CPython
3.12 has been validated locally; ABI tags do not prove testing on other versions
or platforms.

The source distribution is also buildable independently. From the repository
root, run `uvx --from maturin==1.16.0 maturin sdist --manifest-path
bindings/python/Cargo.toml --out D:/scratch/source`. Extract the resulting
archive, change into its top-level directory, then run `uvx --from
maturin==1.16.0 maturin build --release --locked`. Building from the extracted
project root uses its rewritten Python package/manifest paths. Use a fresh
extraction/target directory when checking a new archive: reproducible archive
timestamps can otherwise let Cargo reuse an older local artifact. The archive
excludes compiled wheels, build trees and Python caches.

For local Rust checking without a Python development install:

```powershell
cargo check --manifest-path bindings/python/Cargo.toml
```

## Usage

```python
from pathlib import Path
from rage_texture import Store

with Store(Path("pool.ttpf"), cache_bytes=0, allow_fitted=False) as store:
    for texture in store.textures():
        print(texture.index, texture.width, texture.height, texture.dxgi)

    texture = store.textures()[0]
    dds = store.texture_dds(texture)      # Export
    Path("texture.dds").write_bytes(dds.data)
    print(dds.input_bytes, dds.total_seconds, dds.quality)

    mip = store.texture_mip(texture, mip=0, slice=0)
    png = store.texture_png(texture, mip=0, slice=0)
    raw = store.read_range(0x1000, 0x1000)
    detailed = store.read_range_result(0x1000, 0x1000)
    print(store.stats())
```

- `read_range` is the byte-only convenience path and drops provenance.
  `read_range_result` returns an `Export` with bytes, per-chunk `Quality`
  objects, `input_bytes`, and elapsed stage metrics. Use it for fitted-codec
  diagnostics after explicitly constructing the store with
  `allow_fitted=True`; the default still refuses fitted reconstruction.

- `cache_bytes=0` disables the application decoded cache. It is the intended
  baseline for extraction timing comparisons.
- `allow_fitted=False` rejects fitted BCPack reconstruction. Opt in explicitly
  with `allow_fitted=True`; fitted quality objects identify the codec and
  unfitted/invalid block counts.
- Descriptors belong to the `Store` generation that produced them. Passing a
  descriptor from another store or from before `refresh()` raises `StaleError`.
  After a source file changes, operations raise `StaleError` until `refresh()`.
- A closed store raises `ClosedError`. A second operation on the same store that
  overlaps a detached operation raises `BusyError`; separate stores can run in
  parallel.
- Errors are explicit `Error` subclasses: `UnsupportedError`, `CorruptError`,
  `StaleError`, `ClosedError`, `BusyError`, `BoundsError`.
- `texture_dds` reports the core's separate read/decode, detile and DDS stage
  timings. `texture_mip` and `read_range_result` put the single detached call's
  elapsed time in `read_decode_seconds`; `texture_png` puts its whole encoding
  call in `encode_seconds`. `total_seconds` is their sum.

History-aware policies, dictionary/path configuration, snapshot replacement and
leading DDS mip selection are documented in the parent [HISTORY.md](../../HISTORY.md).
The default remains lossless raw/zlib. `decode_policy="confirmed"` selects strict
history decoding; `"provisional"` permits known estimates with provenance.

## Tests

The unittest suite uses only synthetic RDS5/ETPF fixtures:

```powershell
# From the repository root, using the environment where the wheel is installed:
& D:/scratch/venv/Scripts/python.exe -m unittest discover -s bindings/python/tests -v
```

Run it after installing the wheel into the scratch virtual environment.
