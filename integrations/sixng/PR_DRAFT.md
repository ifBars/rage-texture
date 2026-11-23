# BC6H: select the signed half-float sign after final scaling

Prepared as a focused follow-up to [SixNG PR #23](https://github.com/holdmysocks/SixNG/pull/23),
based on its head `2979c57fdeb35863bb8885e6b48d09090cd1f7de`.
Published as [SixNG PR #24](https://github.com/holdmysocks/SixNG/pull/24),
head `36058e0fbf1ce1fb945e226e60cc2fb80514b4e1`. It remains open; no hosted
checks were reported when inspected. The patch is retained for local reuse.

The proposed decoder selects the half-float sign from the interpolated value
before the final 31/32 magnitude scaling. An interpolant of -1 truncates to
zero, but the early sign selection returns 0x8000 instead of 0x0000.

[Microsoft's finish_unquantize algorithm](https://learn.microsoft.com/en-us/windows/win32/direct3d11/bc6h-format#unquantization-of-color-endpoints)
scales the signed integer first, then sets the sign only when that result is
negative. The patch follows that order and adds both a focused arithmetic
regression and a complete synthetic block exercising the same edge case.

Validation:

- The added regression fails on the unchanged PR head (32768 versus 0).
- With the patch: `cargo test --locked -p sixng-core --no-default-features --lib`
  passes 84 tests, with one existing real-data test ignored. Audio decoding is
  outside this change; disabling its optional Opus dependency avoids an
  unrelated native build.
- `git diff --check` passes.
- Our adapted decoder was independently checked against pinned bcdec C code
  across 147,456 synthetic blocks for each signedness, all 14 valid mode codes
  and four reserved codes. Before this fix, 32 signed half-channel values
  differed, all negative versus positive zero. Afterward, all 14,155,776
  half-channel values agree exactly.

This changes half-float bit accuracy, not the clamped PNG's appearance. The
larger BC6H fixes and decoder implementation remain credited to the existing
upstream PR's author. No game data is included in this patch or regression.

Apply to the PR head with `git apply bc6h-signed-zero.patch`.
