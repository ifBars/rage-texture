# SixNG preview colour review

Branch: `fix/preview-bc1-bc2-bc3`
Base: `origin/master` at `3c0b691` (includes #28)
Commit: `2037771` after parent review expanded the clipped BC2/BC3 cases.
Pull request: https://github.com/holdmysocks/SixNG/pull/35

Only `crates/sixng-core/src/texture.rs` changes: 227 insertions, 3 deletions.
The implementation is the 68-line `rgb565` + `repair_bc23_colour` helper plus
the three `to_rgba` match arms; the rest is six synthetic block tests.

No duplicate upstream work was found. #27 is the open tTPF mips/volumes
change; #31 (BC7 presets) and #28 (A8 alpha) are already on master. None
touches BC1/2/3 preview decoding.

## What was wrong

`to_rgba` called `texture2ddecoder::decode_bc1`, the opaque entry point. BC1's
three-colour mode (`c0 <= c1`) uses index 3 for transparent black; the opaque
decoder gives it alpha 255 instead.

`decode_bc2` and `decode_bc3` decode their colour half with the same BC1 block
decoder. That decoder switches to three colours when `c0 <= c1`, but BC2 and
BC3 always use four colours. Ascending and equal palettes therefore produced a
midpoint at index 2 and transparent black at index 3 instead of the 2/3 and
1/3 interpolants.

Microsoft's references for the mode rules:

* <https://learn.microsoft.com/en-us/windows/uwp/graphics-concepts/opaque-and-1-bit-alpha-textures>
* <https://learn.microsoft.com/en-us/windows/uwp/graphics-concepts/textures-with-alpha-channels>

## What changed

* BC1 now calls `decode_bc1a`, which sets index 3 to transparent black in the
  three-colour mode and stays opaque in the four-colour mode.
* BC2/BC3 still call `decode_bc2`/`decode_bc3`. After a successful decode,
  `repair_bc23_colour` walks the blocks and, only when `c0 <= c1`, replaces the
  RGB of index-2/index-3 pixels with `(2*c0+c1)/3` and `(c0+2*c1)/3`.
  Blocks with `c0 > c1` are untouched.
* Alpha is preserved by masking the existing packed pixel with `0xFF00_0000`.
  BC2 nibble replication and both BC3 alpha modes are unchanged.
* Clipping at the right and bottom edges is handled in the repair loop.
* The `to_rgba` API and the `0xAARRGGBB` packing are unchanged.

This keeps the change to the two reported mode bugs. It does not rewrite the
BC1/alpha decoders or change alpha interpolation rounding. The helper is
original code for SixNG's GPL-3.0-only tree; nothing was copied from the
MIT-licensed decoder.

## Checks

All commands used `--no-default-features`, so libopus/CMake was not involved.

| Check | Result |
|---|---|
| Same six tests against base `3c0b691` | **3 failed, 3 passed** |
| Six regression tests after the fix | **6 passed, 0 failed** |
| `cargo test -p sixng-core --no-default-features` | **109 passed, 1 ignored** |
| `cargo check -p sixng-cli --no-default-features` | **passed** |
| `cargo clippy -p sixng-core --no-default-features --all-targets` | **exit 0**; 83 warnings (76 duplicates) across the tree |
| Local release test run | Interrupted before completion; no passing result claimed |
| Upstream Windows/Linux/macOS CI on PR #35 | Release builds and tests all pass |

The before run used the final six tests, appended to the base `texture.rs`.
Failures were `bc1_three_colour_is_transparent`,
`bc2_always_uses_four_colours`, and `bc3_always_uses_four_colours`; the BC1
four-colour, BC3 alpha, and clipping tests already passed.

Clippy's only `texture.rs` warnings are the pre-existing OR-pattern suggestions
at lines 79 and 81 (`Fmt::from_dxgi`). The new code adds none. `-D warnings`
is not a clean gate on this tree because of the remaining warnings elsewhere.

The parent expanded the clipping test to use two BC2/BC3 blocks in a 5x3 image,
checking both edge guards, palette values and preserved alpha. All six final
colour tests pass. Full release builds/tests run in the upstream three-OS CI.

## PR draft

Title: `BC1/2/3 previews: follow the specification's colour modes`

Body:

> `to_rgba` decodes BC1 through `texture2ddecoder`'s opaque entry point, so a
> three-colour block's index 3 comes out opaque instead of transparent. It also
> decodes the BC2/BC3 colour half with the BC1 block decoder, which switches to
> three colours when `c0 <= c1`; BC2 and BC3 are always four-colour.
>
> This uses `decode_bc1a` for BC1 and repairs only the index-2/index-3 RGB of
> BC2/BC3 blocks with `c0 <= c1`. Alpha, the `0xAARRGGBB` packing, and the
> `to_rgba` API stay the same. The new block tests cover transparent BC1,
> ascending and equal BC2/BC3 palettes, both BC3 alpha modes, and clipped
> edges.
>
> Core tests: 109 passed, 1 ignored
> (`cargo test -p sixng-core --no-default-features`).

## Non-claims

* The tests are synthetic 4x4 blocks, not a visual comparison over game data.
* No claim is made about pixel accuracy beyond the covered mode and alpha
  cases.
* Alpha interpolation values are intentionally unchanged from the previous
  decoder, including its integer rounding.
* PR #35 merged as `d2467905a3585311f0969f433f079919aa22ef03`; Windows, Linux and macOS release CI passed.
