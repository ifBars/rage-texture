# DDS consumer QA

Status: the independent container validator and the DirectXTex consumer audit
are implemented and passing on representative real textures from the fixed
Python wheel. The Rust DDS helper is frozen for the full audit binary.
This QA also exposed the BC1/BC3 preview repairs described below. A later
BC7 preset update is checked separately across all affected chunks in
[CORPUS_QA.md](CORPUS_QA.md); the original full-container run is preserved.

## Two separate checks

A DDS can have a coherent container and wrong pixels, or correct pixels and a
wrong header. The QA keeps those claims apart.

1. **Container** (`examples/support/dds_consumer.rs`, tests in
   `tests/dds_consumer.rs`): `ddsfile =0.5.2` parses the bytes. The validator
   checks the DX10 format, width, height, mip count, array size, cube misc and
   face flags, volume depth and dimension, top-level pitch/linear size, and the
   exact payload length of every mip/slice. Unknown formats and non-DX10 files
   fail; they are never passes.
2. **Pixels** (`scripts/verify_dds_consumers.py`): `texdiag info` independently
   parses the container, then `texconv` decodes to RGBA8 in a DX10 DDS. The
   script reads that lossless payload by subresource and compares it with the
   manifest reference. The reference is produced by the SDK, never by
   DirectXTex, so a pass is cross-decoder agreement.

The Rust validator is deliberately independent of `src/dds.rs` and
`src/layout.rs`: subresource sizes come from the parsed header and the format's
own pitch/block rules. The Python audit uses `texconv`'s output header for the
same purpose.

`ddsfile` is MIT-licensed and is a dev-dependency only. Its notice is mirrored
with the other dev-only notices at the repository root.

## Container coverage

`cargo test --test dds_consumer` runs seven tests:

- all 20 DXGI formats observed in the retail stores as full 2D chains
- the same 20 formats as 3D arrays and cubemaps
- every uncompressed one of the 20 as a 3D volume; block-compressed volumes
  must fail explicitly
- a leading partial chain against the shape that was actually written
- negative controls: truncation, trailing bytes, bad magic, missing DX10
  header, wrong format, wrong width/height, wrong mip count, wrong array size,
  wrong pitch, wrong cube flag/missing face, wrong volume depth/dimension and
  missing volume flag

Result: 7 passed, 0 failed. The complete corpus subsequently exported and
validated all 357,115 full DDS containers with no errors. It retained 49 bounded
representatives. Pixel comparisons below remain a separate sampled check.

## Pixel coverage and normalization

Four categories are reported separately:

- **exact**: all four channels compared, tolerance 0.
- **quantized**: all four channels compared, tolerance 1.
- **masked exact**: only the channels the format stores, tolerance 0.
- **masked quantized**: only the channels the format stores, tolerance 1.

A comparison mask must include every stored channel for the DXGI format. A
decoder may invent values for a missing channel, so comparing only stored
channels avoids treating that presentation choice as a payload difference. It
may not hide a stored channel: for example, BC1 must include alpha (1-bit
transparency), A8 must include only alpha, and BC5 must include both R and G.
Masks are compared with four strided channel slices; no per-byte index list is
built.

The ±1 allowance is for endpoint interpolation and normalized-float to RGBA8
rounding, not for larger channel storage. It is permitted only on block formats
and formats with more than 8 bits per channel, and the value must be 0 or 1.
Masked and quantized can overlap (BC4/BC6), which is why the masked counts are
reported separately instead of being called exact.

sRGB inputs are converted to `R8G8B8A8_UNORM_SRGB`, not linearised, so the
audit compares the encoded values the DDS stores.

The script rejects a SHA-256-only oracle when a channel mask is needed, because
a whole-byte hash cannot represent a masked comparison.

### Synthetic codec cases

`--self-test` builds spec-derived cases that do not depend on the SDK preview:

- BC1 3-colour block with index 3 transparent: expected alpha 0
- BC3 colour block with `c0 <= c1`: BC3 is always 4-colour, expected 85 for
  index 2 (the BC1 palette value would be 127)

DirectXTex decodes both to the spec reference exactly. The same blocks were
also placed in a synthetic TTPF store and decoded with the fixed Python wheel
(`de430fa7…`): the SDK output is byte-identical to the spec reference for both
cases. This is the SDK-versus-DirectXTex confirmation for the BC1/BC3 fixes.

The same self-test covers 2D, array, cube and volume samples, a wrong-geometry
sample, a wrong-array-size sample, a truncated sample, a wrong-oracle sample, an
incomplete channel mask and an excessive tolerance. All six negative controls
are rejected. Self-test result: 6 samples, 128 checks, 32 subresources, 0
errors.

## Real-input result

DirectXTex is built from `microsoft/DirectXTex` commit
`a8e5dbf42909aeeb9fcf9c2ae399ae3c06737379` under
`work/DirectXTex/build/bin/Release` (no game or derived data is committed).

Oracles were regenerated with the fixed Python wheel:

```
extension SHA-256: de430fa7bd81df4bed2af29167a84514427aff1c2d1ac05edf88ba94c5896287
wheel venv: work/python-sdk-venv
cache_bytes: 0
```

Manifests record `oracle_extension_sha256`, `oracle_extension_mtime` and the
Python version, so an oracle cannot be mistaken for a different wheel.

Bounded real audit, all subresources of the selected samples:

| Sample set | Samples | Checks | Subresources | Exact | Quantized | Masked exact | Masked quantized | Errors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| streamed audit samples | 29 | 1077 | 613 | 25 | 555 | 2 | 31 | 0 |
| extra formats | 5 | 93 | 13 | 2 | 10 | 1 | 0 | 0 |
| lossless BC5 | 1 | 21 | 5 | 0 | 0 | 0 | 5 | 0 |
| total | 35 | 1191 | 631 | 27 | 565 | 3 | 36 | 0 |

Coverage: all 20 retail formats, including A8, BC6H UF16/SF16 and BC5; 2D, 2D
array, cube and 3D volume (including the 48-slice BC7 array and the
R16/R16G16B16A16 volumes).

The BC5 sample is a lossless raw/zlib surface from a pack store
(`128x64`, five mips), not a BCPack reconstruction. Its five subresources are
compared on the stored RG channels. The earlier graphics BC5 candidate was
BCPack-encoded and refused by the exact decode policy; that refusal is recorded
as unsupported, not as a pass. No fitted block ever feeds a DirectXTex oracle,
and no seed-accuracy claim is made from fitted data.

## Manifest schema

Paths are relative to the manifest directory unless absolute.

```json
{
  "oracle_extension_sha256": "…",
  "samples": [
    {
      "file": "sample.dds",
      "kind": "2d",
      "dxgi": 28,
      "width": 16, "height": 16, "depth": 1, "mips": 4,
      "subresources": [
        {
          "mip": 0, "slice": 0,
          "reference": "refs/sample_m0_s0.rgba",
          "channels": "rgba",
          "tolerance": 0,
          "sha256": "…"
        }
      ]
    }
  ]
}
```

`slice` is the array item, cube face or volume Z. `channels` must contain every
stored channel for the DXGI format and may be `rgba`; omit `sha256` for a masked
comparison. `tolerance` is 0 or 1, and 1 is accepted only on the declared block
and >8-bit formats.

## Reproduce

```powershell
cargo test --test dds_consumer

$texdiag = 'work/DirectXTex/build/bin/Release/texdiag.exe'
$texconv = 'work/DirectXTex/build/bin/Release/texconv.exe'

python -I scripts/verify_dds_consumers.py --self-test `
  --samples work/dds-consumer-selftest `
  --texdiag $texdiag --texconv $texconv `
  --out work/dds-consumer-selftest.json

python -I scripts/verify_dds_consumers.py `
  --samples work/exhaustive-consumer-samples `
  --manifest work/exhaustive-consumer-manifest-wheel.json `
  --texdiag $texdiag --texconv $texconv `
  --out work/exhaustive-consumer-qa-wheel.json
```

## Scope after the complete run

- Full-corpus container validation is complete. The independent pixel set
  covers all 20 formats and the observed layout families, rather than every
  pixel of every store.
- Keep the fixed-wheel hash in regenerated manifests and rerun the synthetic
  BC1/BC3 store check whenever the wheel changes.
- No further edits to the frozen Rust DDS helper without a freeze revision.
