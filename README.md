# rage-texture

A Rust library and CLI for extracting textures from RDS5/TTPF stores, with
DDS export, PNG previews and optional Python bindings.

I'm building this as a foundation for asset tools: fast reads, usable APIs,
and output quality you can check. It reuses work from SixNG, Leonida and
bcpack-rs, with attribution. The combined project is GPL-3.0-only.

**Early development.** Tested against one Xbox build. Independent confirmation
of the original BC6H/BC7 initial histories is still open, and PS5/PC adapters
aren't implemented. Multi-mip volumes and broader build coverage also remain open.

## What works

- Indexed reads with checked chunk bounds and strict raw/zlib validation.
- BC1/3/4/5/6H/7 BCPack decoding with explicit quality diagnostics.
- DDS export preserving format, compressed blocks, HDR, mip chains and slices.
- Measured Gfx10 layouts for 2D textures, arrays, cubemaps and single-mip volumes.
- Typed PNG previews, full-precision BC6H half-float decoding, and batch DDS export.
- Optional per-store caching, source-change checks and explicit refresh.
- Python bindings and a [Leonida adapter](integrations/leonida/README.md).

Lossless raw/zlib decoding is the default. Fitted reconstruction requires
opt-in and stays labelled provisional, even when every preset is available.
Matching another fitted decoder doesn't prove the original history is correct.
[History policies](HISTORY.md) explain the distinction.

## Build and use

Install a current stable Rust toolchain, then build from source:

```sh
git clone https://github.com/ifBars/rage-texture.git
cd rage-texture
cargo build --release --locked
```

On Windows, the executable is `target/release/rage-texture.exe`.
On Linux, use `target/release/rage-texture`.

```sh
rage-texture list pool.ttpf
rage-texture export pool.ttpf 0 texture.dds
rage-texture export pool.ttpf 0 preview.png --mip 0 --slice 0
rage-texture export-all pool.ttpf exports --allow-fitted
rage-texture bench-export pool.ttpf 0 7 --no-cache
```

Use the executable's full path or add its directory to PATH. Run `--help` for
all commands. Sources are read-only inputs. Export commands preserve existing
files; batch export supports byte-verified resumption.

### Rust

```rust,no_run
use rage_texture::{DecodePolicy, Store};

fn main() -> std::io::Result<()> {
    let mut store = Store::open("pool.ttpf", 0, DecodePolicy::LosslessOnly)?;
    let textures = store.textures()?;
    let output = store.texture_dds(&textures[0])?;
    std::fs::write("texture.dds", output.dds)?;
    Ok(())
}
```

The zero in `Store::open` disables the decoded cache. Higher-level APIs include
`texture_mip`, `texture_dds_mips`, `texture_png` and batched reads. The API may
change during development. [Python setup](bindings/python/README.md) is separate
from the Rust build.

## What we've checked

The frozen full-corpus run compares all **1,026,833 chunks across 57 stores**
with pinned SixNG. All **357,115 DDS exports** pass independent container checks.
A later check covers every affected BC7 chunk after adopting SixNG's 34 added
presets: all 5,167 match, with zero missing-preset flags. This update remains
provisional and is recorded separately from the full run.

Current CLI and Python exports retain 1,076 saved full DDS reference matches.
Independent DirectXTex checks cover 631 sampled mip/slice subresources across
all 20 observed formats, with exact and quantized comparisons reported separately.
The synthetic suites include 161 Rust tests and 33 Python/report/adapter tests.
Game data and raw local reports are excluded from this repository.

Seven sampled complete DDS workloads measured **8.4–30.0x faster than Leonida**
with decoded caches disabled and equal output bytes. A 64-texture batch including
writes and fsync measured 29.0x. These use warm filesystem cache and aren't a
whole-corpus speed estimate. An overall SixNG speed lead hasn't been established.
[Benchmarks](BENCHMARKS.md) include timing boundaries, fingerprints and controls.

## Cache behaviour

The decoded cache belongs to one store, source generation and decode policy.
It's optional and doesn't persist between sessions. Normal source updates or
path replacements make the store stale until `refresh()` reparses it; reload
texture descriptors afterward. History replacement also invalidates cached data.
Changes that preserve file identity and every timestamp require explicit refresh.
Concurrent writes aren't snapshotted. Tests cover replacement, refresh,
cross-file isolation, history changes, eviction and a zero-byte cache budget.

## Contributing

Bug reports with a small synthetic fixture or reproducible command are useful.
Please keep game files and extracted assets out of issues and pull requests.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

The tests use synthetic fixtures. External-corpus scripts need your own local
inputs; run each script with `--help` for inputs and options.

## License and credits

The combined project is [GPL-3.0-only](LICENSE). The historical MIT grant in
`LICENSE-MIT` doesn't relicense the GPL-derived code or this combined project.
[Third-party notices](THIRD_PARTY_NOTICES.md) record the source revisions and
adaptations, including:

- [SixNG](https://github.com/holdmysocks/SixNG): BCPack decoders, fitted presets
  and the existing BC6H decoder work.
- [Leonida](https://github.com/ItsNotRexed/Leonida): measured layout/DDS rules
  and history-aware reconstruction.
- [bcpack-rs](https://github.com/Ahmet-Dedeler/bcpack-rs): entropy decoding.

No game data or game executable is included. This is an independent project.
