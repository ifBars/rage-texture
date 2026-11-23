# SixNG texture layout disagreements

Status: layout repair submitted as [SixNG PR #27](https://github.com/holdmysocks/SixNG/pull/27); Windows, Linux and macOS CI pass.
The combined SDK's `src/` was not modified. No harness scripts were modified.

- SixNG base: `7881042c294f3eeea1e8153e7bcafd84f9d787b9`
- Layout commit: `5b0b5884fc6c1246ba9fcf83d9a6e7bdffc4fd09`
- Worktree: `work/SixNG-layout-review`; local `validation/combined-texture-fixes` also contains the independent A8 repair for comparison.
- Patch: `work/sixng-layout-mips-volume.patch` (2 files, outside this repository)
- Comparison store: `$STORE` (read-only, outside this repository)

## Result

The pinned SixNG core reproduced three classes of failure in the graphics
preview comparison:

| Class | Textures | Failed subresources |
| --- | --- | --- |
| Mip count and standalone placement | #151 | 1 |
| Mip metadata only | #145 | 0 (count disagreed) |
| Cube trailing mip | #177 | 6 faces x 1 mip |
| 3D volume layout | #200 | 32 slices |
| Total | 4 | 39 of 608 |

After the patch, all 608 requested mip/slice previews are byte-identical to
this SDK, with `pixel_errors: 0` and `metadata_errors: 0`. The complete
single-mip RGBA8 preview at #151 also matches Leonida's independent linear
hash `5f58ac9c9ad030c125aa0fa21747447cd87ba267b4057b4a092a0bc3f63cbdef`.

## Root cause: mip count

SixNG used two different descriptor rules:

- non-cubes: `(d0 >> 28) + 1`
- cubes (tile modes 9-12): `(d0 >> 28).max(1)`

The descriptor semantics used by Leonida and this SDK are:

```
mips = (d0 >> 28) + (d1 & 7)
```

The low three `d1` bits are the non-tail mip count. Assuming a constant one
can overcount or undercount; the allocation-gap heuristic does not always
resolve it. Examples:

- #151, 16x8 R8G8B8A8_UNORM: packed 1, non-tail 0, one mip. SixNG read two.
  An extra mip moves mip 0 into the shared tail origin; the tail is zero, so
  the 512-byte output was exactly SHA-256 of 512 zero bytes
  (`076a27c7...`). The descriptor's one mip starts at offset 0.
- #145: eight source mips, while SixNG reported nine.
- #177, 64x64 BC7 cube: packed 6, non-tail 1, seven mips per face. The old
  cube rule used six, so the 1x1 mip was not exportable. On every face the
  1x1 pixel averages the 2x2 parent, confirming the seventh mip is real.

Correcting the count is necessary but not sufficient for #151: the 2D layout
also placed a one-mip surface at the fixed max-tail origin `(16, 0)`, which is
the unused part of the tile. A surface with one mip has no smaller mip to share
a tail with, so it must start at offset 0. Both changes are in the patch.

## Root cause: 3D volume layout

#200 is a 32x64x32 R16G16B16A16_FLOAT 3D volume (dimension 3, mode 2, one
mip). SixNG's `Layout::with_slices` models the depth as a 2D array: each slice
gets its own whole mip chain. AMD Gfx10 3D `SW_4KB_S` instead interleaves Z
into each 4 KiB micro-tile, with the tile grid row-major in X/Y and Z tile
slabs `z_tile_stride` apart.

Leonida's independent volume/array check agrees with this SDK:

```
volume_3d_z_diff         0.016793
volume_2d_control_z_diff 0.110316
```

The patched SixNG dimension-3 layout uses the measured 4 KiB micro-tile
extents and intra-tile bit order:

| Element bytes | Micro-tile (w, h, d) |
| --- | --- |
| 1 | 16, 16, 16 |
| 2 | 16, 8, 16 |
| 4 | 8, 8, 16 |
| 8 | 8, 8, 8 |
| 16 | 8, 4, 8 |

For 32x64x32 with 8-byte elements this gives `block_d = 8`,
`padded_d = 32`, `z_tile_stride = 131072`, and `total = 524288`, which equals
the store allocation span. `mip_span(0)` is the whole volume because the only
mip is Z-interleaved.

The 2-byte extent is exercised by #901 (256x32x8, R16_FLOAT): total 262144
matches the allocation span, and all eight slices decode identically.

## Bounded 3D subset

`TtpfTex::layout` now branches on `dimension == 3` before any 2D fallback and
returns `None` unless all of the following hold:

- raw mode (`d1 >> 23`) is 2
- mip count is 1
- the format is uncompressed

A future dimension-3 descriptor with another mode fails explicitly instead of
silently receiving a 2D array layout. `fix_mip_counts` skips every
dimension-3 texture, because its 2D gap heuristic cannot describe a volume.

## Validation

Worktree tests (`--no-default-features --lib`): 101 passed, 0 failed,
1 ignored. New tests cover:

- mip count from packed + non-tail bits, including #151 and the BC1 spacing case
- cube mips including the non-tail bit (#177)
- single-mip standalone placement (#151)
- volume totals and hand-picked Z addresses (#200)
- the 2-byte volume extent (#901)
- explicit rejection of mode 4, multiple mips, and block-compressed volumes
- range and truncation guards

`cargo clippy --locked -p sixng-core --no-default-features --lib` reports no
warnings in the changed files. The crate as a whole is not clippy-clean at
this newer toolchain, and the repository is not rustfmt-clean at the base
commit, so neither was applied repository-wide.

Real-input comparison, decoded caches disabled:

- `scripts/verify_sixng_subresources.py`: 11 textures, 608 subresources,
  608 equal, 0 pixel errors, 0 metadata errors
- all 32 slices of #200, all 32 of #575, and all 8 of #901 match this SDK
- #151 output matches this SDK and Leonida

## Attribution and licensing

The volume table and order logic are derived from the GPL-3.0-or-later
Leonida texture tools (`tools/tex_headers.py`, `tools/ttpf_mips.py`, pin
`aa2988`) and from this combined SDK's own `src/layout.rs`. The SDK is
GPL-3.0-only and is not published as a separate source URL. No game data is
part of the derivation or of the tests.

## Remaining uncertainty

- No real volume uses 1-, 4-, or 16-byte elements in the compared stores.
  Those micro-tile extents are table values, not measured here.
- Volumes with more than one mip are rejected. No real sample was available.
- The 608-subresource run covers the grouped graphics sample only. The
  broader store matrix, including pack stores, still belongs to the parent's
  parity run.

## Reproduction

```
$env:CMAKE_GENERATOR='Visual Studio 17 2022'
cargo test --locked -p sixng-core --no-default-features --lib

python scripts/verify_sixng_subresources.py \
  --store $STORE \
  --catalog target/release/rage-texture.exe \
  --native target/release/examples/preview_reference.exe \
  --reference <patched-sixng-reference.exe> \
  --out <report-outside-repository>.json
```

## Broader format comparison and A8

The broader three-store check initially had 835/843 matching subresources.
All eight remaining failures were the eight mips of an A8_UNORM peds texture:
SixNG mapped format 65 to R8, rendering stored alpha as opaque grayscale.
The SDK preserves the byte as alpha with white RGB for mask presentation.

[SixNG PR #28](https://github.com/holdmysocks/SixNG/pull/28) separates A8 from
R8 and preserves its alpha. The synthetic transparency test fails before the
repair; the R8 control remains unchanged. Its isolated core suite passes 98
tests with one existing ignored test. The layout and A8 fixes combined pass
103 core tests, one ignored, and all 843 real mip/slice comparisons across
31 textures, with zero pixel or metadata mismatches.

The combined local validation commit is
`e051660` (layout PR commit plus the A8 cherry-pick). The A8 PR head is
`2b732e50a6dcd4679d09c252c65bb9b89d597ea6`.
