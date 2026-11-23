# BC6H accuracy checkpoint

The SDK now decodes BC6H UF16/SF16 blocks to exact RGB half-float bits through
`bc6h::decode_block` or cropped row-major images through `bc6h::decode_rgb16f`.
This API preserves HDR range and signed values. PNG previews use the same
decoder, then clamp to [0,1] and reduce to RGBA8; DDS retains the original blocks.

The implementation is an attributed adaptation of the existing
[SixNG PR #23](https://github.com/holdmysocks/SixNG/pull/23), pinned at
`2979c57fdeb35863bb8885e6b48d09090cd1f7de`, GPL-3.0-only. It addresses the
upstream-reported unsigned endpoint, 16-bit endpoint-mask and signed endpoint
errors in texture2ddecoder 0.1.2. We additionally corrected sign selection after
final scaling so negative interpolants truncating to zero produce positive zero,
as [Microsoft's BC6H specification](https://learn.microsoft.com/en-us/windows/win32/direct3d11/bc6h-format)
requires. Credit for the larger decoder correction belongs to that upstream work.

An independent C implementation, [bcdec](https://github.com/iOrange/bcdec) at
`80859ed3b7afb1c527a2a99d70c61457bea72d0c`, was compiled separately as an
offline oracle. It is not linked into the SDK. Its output agrees bit for bit
for 147,456 deterministic synthetic blocks per signedness: 14 valid mode codes
and four reserved codes, each with 8,192 randomized endpoint/partition/index
patterns. Total: **294,912 block/signedness cases and 14,155,776 RGB half values,
zero discrepancies**. Reserved codes yield black. Independent targeted tests
cover the edge cases and image boundaries without any game fixtures.

The same synthetic set changes the old decoder's clamped preview for 43,348
unsigned and 79,208 signed blocks. Those counts describe this adversarial random
test set, not the fraction of real game textures affected. This verifies the
BC6H-block-to-RGB stage; it does not establish that fitted BCPack reconstruction
recovered original BC6H blocks or resolve unrelated layout disagreements.

## Reproduce

1. Obtain bcdec.h at the pinned revision outside this repository and inspect its
   license. Build `scripts/bc6h_oracle.c` with that external include directory
   using a C compiler, for example `cl /O2 /I D:/source/bcdec scripts/bc6h_oracle.c`.
2. `cargo build --release --locked --example bc6h_oracle`
3. Run the following with output outside the repository:

```powershell
python scripts/verify_bc6h.py --native target/release/examples/bc6h_oracle.exe --reference D:/scratch/bc6h_oracle.exe --out D:/scratch/bc6h-parity.json --per-mode 8192
```

The helper generates synthetic blocks in a temporary directory, checks binary
fingerprints before/after validation, and removes temporary decoded data. The
source vectors are reproducible from the fixed seed; raw reports remain local.
A focused upstream contribution is prepared under [integrations/sixng](integrations/sixng/PR_DRAFT.md).
