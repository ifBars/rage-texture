# Format and layout gaps

Status: scoped inventory for the preview work. This note does not change the
release gate in `FEATURE_PARITY.md`; it records what preview covers, what the
Xbox 1.0.2.1 corpus exercises, and which formats remain explicitly unsupported.

## Preview format matrix

`src/preview.rs` supports these typed DXGI formats. Typeless IDs are rejected
before any byte interpretation, and DDS remains the precision-preserving
export. UNORM_SRGB previews show stored bytes without an sRGB transfer
conversion.

| Family | DXGI IDs | Notes |
|---|---|---|
| BC1 | 71, 72 | UNORM and UNORM_SRGB |
| BC2 | 74, 75 | explicit 4-bit alpha |
| BC3 | 77, 78 | UNORM and UNORM_SRGB |
| BC4 | 80, 81 | UNORM and **SNORM (new)** |
| BC5 | 83, 84 | UNORM and **SNORM (new)** |
| BC6H | 95, 96 | UF16 and SF16 |
| BC7 | 98, 99 | UNORM and UNORM_SRGB |
| RGBA float | 2 | R32G32B32A32_FLOAT |
| RG float | 16 | R32G32_FLOAT |
| RGBA half | 10 | R16G16B16A16_FLOAT |
| RGBA UNORM | 11 | R16G16B16A16_UNORM |
| RGBA SNORM | 13 | R16G16B16A16_SNORM |
| R10G10B10A2 | 24 | UNORM |
| R11G11B10 | 26 | FLOAT, shared preview clamp |
| RGBA8 | 28, 29 | UNORM and UNORM_SRGB |
| RGBA8 SNORM | 31 | SNORM |
| RG half | 34 | R16G16_FLOAT |
| RG UNORM | 35 | R16G16_UNORM |
| RG SNORM | 37 | R16G16_SNORM |
| R float | 41 | R32_FLOAT, shown as grey |
| RG8 | 49 | UNORM |
| RG8 SNORM | 51 | SNORM |
| R half | 54 | R16_FLOAT, shown as grey |
| R UNORM | 56 | R16_UNORM, shown as grey |
| R SNORM | 58 | R16_SNORM, shown as grey |
| R8 | 61 | UNORM, shown as grey |
| R8 SNORM | 63 | SNORM, shown as grey |
| A8 | 65 | alpha only, opaque white where alpha is 255 |
| BGRA8 | 87, 91 | UNORM and UNORM_SRGB |
| BGRX8 | 88, 93 | UNORM and UNORM_SRGB, X ignored |
| Unsupported | all others | explicit `Unsupported` error |

`preview::supports(dxgi)` is the cheap public predicate used by
`examples/coverage.rs`; it does not decode anything.

## Signed BC4_SNORM / BC5_SNORM

The decoder follows the Microsoft Direct3D 10 block-compression tables:

- Endpoints are signed bytes. `-128` is clamped to `-127`, the minimum SNORM
  value, before the interpolation mode is chosen.
- If `red_0 > red_1` (after the clamp), six values are interpolated with the
  `6/7 ... 1/7` weights. Otherwise four values are interpolated with `4/5 ...
  1/5`, and codes 110/111 are `-1`/`+1`.
- Interpolation is done in floating point, then `preview::snorm` maps
  `[-1, +1]` to `[0, 255]`: `-1 -> 0`, `0 -> 128`, `+1 -> 255`. NaN maps to
  zero and out-of-range values are clamped.
- BC4_SNORM is shown as grey with opaque alpha.
- BC5_SNORM maps X (the first 8 bytes of each block) to red and Y (the second
  8 bytes) to green. Blue is zero and alpha is opaque. This preserves the
  signed X/Y channels instead of reinterpreting them as UNORM.
- Padded texels outside the visible width/height are skipped, so odd sizes use
  whole 4x4 source blocks but emit only visible pixels.

Sources checked:

- Microsoft, *Block Compression (Direct3D 10)*:
  <https://learn.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-block-compression>
- External `bcdec.h` oracle in `../../work/bc6h-oracle` (not shipped, not
  copied into this repository) using `BCDEC_BC4BC5_PRECISE` signed float
  decoders.

Oracle evidence: 200 deterministic synthetic blocks were decoded by bcdec's
signed BC4 and BC5 float paths and mapped through the same `snorm` rule. All
400 BC4/BC5 preview results matched this implementation byte-for-byte. The
checked-in tests also cover six-value mode, four-value mode, the `-128` clamp,
channel separation and a 2x2 edge crop (`tests/preview_formats.rs`).

## Baseline comparison

Leonida `tools/dds_png.py` at `aa2988cc` previews 24, 26, 27-29, 70-84, 87-93
and 97-99, but treats typeless IDs 27/90/92 as typed. Positive preview additions
here are BC4/BC5 SNORM (81/84), 24, 26, 88 and 93, plus SNORM and simple
float/unorm typed variants. Typeless IDs remain rejected.

SixNG `crates/sixng-core/src/texture.rs` lists 24, 26, 34 and 41 in
`dxgi_name`, but `Fmt::from_dxgi` sends 79-81 to the UNORM BC4 decoder, 82-84
to the UNORM BC5 decoder, and 30-32 to RGBA8. That is not correct signed
interpretation, so those mappings were not copied. SixNG's raw 79-84 decoding
is only a compatibility baseline.

## Layout constraints (existing, unchanged)

From `src/layout.rs` and `NOTES-layout.md`:

- Known DXGI IDs with element sizes 1/2/4/8/16 and block 1 or 4 are accepted;
  12-byte formats 6/7/8 are known but have no measured compact layout rule and
  fail explicitly.
- 2D arrays require `dimension == 1`, `depth >= 2`, mode 1.
- Cubemaps require `dimension == 2`, `depth == 6`, `non_tail <= 3`, and
  `mode == 18 + 2 * non_tail`. Streaming page counts are validated against the
  descriptor.
- 3D volumes require `dimension == 3`, mode 2, exactly one mip, and an
  uncompressed format.
- Layout accepts block-compressed typeless IDs so the DDS can preserve raw
  format identity. Preview still rejects them because signedness and color
  space are not inferable.
- Unknown formats, unsupported modes, invalid mip counts and allocation-span
  disagreements all return explicit errors.

## Xbox 1.0.2.1 coverage run

Command (example target only; the CLI release binary was not rebuilt):

```text
cargo build --release --example coverage
target/release/examples/coverage.exe --stores <owned-root> --cases
```

Artifacts outside Git:

- `../../work/coverage-gta6-xbox-1.0.2.1.json`
  (95,703 bytes, SHA-256
  `7fac5f77f46c8b6d64bd9c60b1b912dbabe9dff66353e39ffbdd019bb94f454b`)
- `../../work/coverage-gta6-xbox-1.0.2.1.log` (per-store progress on stderr)

The scan used `Store::open(path, 0, LosslessOnly)` (zero decoded-byte budget),
`Store::textures()`, and per-descriptor `Store::texture_layout()`. It did not
read or decode texture payloads, and stdout contains only metadata.

Result:

| Measure | Count |
|---|---:|
| stores discovered/opened | 57 / 57 |
| textures | 357,115 |
| layout valid | 357,115 |
| layout failed | 0 |
| preview supported | 357,115 |
| preview unsupported | 0 |
| unsupported reasons/shapes | none |
| per-case failures | none |

Every texture in this build passed allocation-span validation. **There is no
failing shape in this corpus for the parent to fix**; the remaining layout
risk is other builds/adapters and unobserved shapes, not a reproduced Xbox
1.0.2.1 failure.

Store groups (all valid):

| Group | Textures |
|---|---:|
| graphics | 912 |
| interiors | 20,860 |
| lighting | 1,159 |
| maps | 144,512 |
| peds | 39,153 |
| props | 46,418 |
| terrain | 76,904 |
| textures_loose | 194 |
| ui | 10,062 |
| vehicles | 11,166 |
| vfx | 1,757 |
| weapons | 4,018 |

Observed formats:

| DXGI | Name | Textures |
|---:|---|---:|
| 10 | R16G16B16A16_FLOAT | 3 |
| 11 | R16G16B16A16_UNORM | 5 |
| 28 | R8G8B8A8_UNORM | 1,774 |
| 29 | R8G8B8A8_UNORM_SRGB | 2 |
| 54 | R16_FLOAT | 947 |
| 61 | R8_UNORM | 8 |
| 65 | A8_UNORM | 121 |
| 71 | BC1_UNORM | 174,674 |
| 72 | BC1_UNORM_SRGB | 82,177 |
| 74 | BC2_UNORM | 2 |
| 77 | BC3_UNORM | 3,045 |
| 78 | BC3_UNORM_SRGB | 3,880 |
| 80 | BC4_UNORM | 4,656 |
| 83 | BC5_UNORM | 6,904 |
| 87 | B8G8R8A8_UNORM | 50 |
| 91 | B8G8R8A8_UNORM_SRGB | 4 |
| 95 | BC6H_UF16 | 1,000 |
| 96 | BC6H_SF16 | 25 |
| 98 | BC7_UNORM | 52,612 |
| 99 | BC7_UNORM_SRGB | 25,226 |

No BC4_SNORM, BC5_SNORM, R10G10B10A2, R11G11B10 or BGRX8 textures appear in
this build; those preview paths are covered by synthetic exact-value tests.

## Remaining preview gaps

These are deliberately unsupported rather than silently guessed:

- All typeless formats, including 1, 5, 9, 15, 19, 22, 23, 27, 33, 39, 44, 48,
  53, 60, 70, 73, 76, 79, 82, 90, 92, 94 and 97.
- Integer (`UINT`/`SINT`) formats such as 3, 4, 7, 8, 12, 14, 17, 18, 25, 30,
  32, 36, 38, 42, 43, 50, 52, 57, 59, 62 and 64. Integer data has no natural
  `[0,1]` preview mapping.
- Depth/stencil formats 20-22, 39, 40, 44-47 and 55.
- Packed, shared-exponent, XR-bias and video formats 66-69, 85, 86, 89, 100-115,
  130-132 and 191. Some are straightforward to add if a real store uses them,
  but none is exercised by this corpus or by Leonida's preview set.
- 12-byte formats 6/7/8 are already refused by layout, so a preview cannot be
  reached through the normal extraction path.

## Verification run

```text
rustfmt --edition 2024 --check src/preview.rs tests/preview_formats.rs examples/coverage.rs
cargo clippy --example coverage -- -D warnings -A dead_code
cargo test --test preview_formats
cargo test --lib preview::
cargo check --example coverage
```

The preview test suite is 12 integration tests plus 5 in-module tests. A
separate temporary harness compiled the same `preview.rs` against only
`texture2ddecoder` and `png`; it passed clippy with `-D warnings`.
