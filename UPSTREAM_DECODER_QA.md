# Upstream decoder QA: texture2ddecoder BC6H

Owner: external upstream investigation. This document covers the local review
checkout at `../../work/texture2ddecoder-review`; it does not change this SDK's
source or dependency version.

## 1. Scope and upstream state

- Repository: [`K0lb3/texture2ddecoder-rs`](https://github.com/K0lb3/texture2ddecoder-rs)
  (MIT OR Apache-2.0); the previous UniversalGameExtraction URL redirects here.
- Base commit: `551fcbfbe86c05424ccc2ff942525c0ec7171d65`, crate version `0.1.2`.
- Open BC6H work upstream:
  - PR #17, "bc6: fix the endpoint mask for 16-bit transformed endpoints", is
    open. It addresses the 16-bit mask overflow. Do not duplicate it.
  - Issue #10, "Signed BC6 is wrong", is open with no PR. The maintainer
    confirmed the signed unquantize and finish-unquantize extraction is wrong.
- The unsigned maximum-threshold path has no upstream issue or PR. It is fixed
  here with the signed path because both arise from the same spec section.
- The 16-bit mask fix from PR #17 is not included in this branch.

## 2. Chosen defect and reproduction

The BC6H endpoint path extracted the sign with `value & 0x8000` and the
magnitude with `value & 0x7fff`. For a negative two's-complement endpoint this
turns the sign magnitude into a large positive value. Interpolation then uses
the wrong domain, and `finish_unquantize` applies the sign before scaling.

The unsigned path also used `1 << (bits - 1)` as the maximum special case. The
specification and bcdec special-case only `(1 << bits) - 1`.

Synthetic failing tests are in the upstream worktree at `tests/bc6.rs`. They
use one random synthetic block, not game data:

```text
0902e0f0d8d6ce98656d9c2b6300e9c5
```

Before the source change:

```text
cargo test --test bc6
signed_endpoints_keep_their_sign_and_magnitude ... FAILED
unsigned_maximum_threshold_is_full_range ... FAILED
```

After the change, both tests pass. Expected eight-bit channels were computed
from `bcdec_bc6h_half` at revision
`80859ed3b7afb1c527a2a99d70c61457bea72d0c`, converting each half to float and
applying the same `[0, 255]` clamp and truncation as the public API.

## 3. Patch

Branch: `bc6-signed-unquantize`
Commit: `c3dabbc` (authorship set to the submitting account after review).
Pull request: https://github.com/K0lb3/texture2ddecoder-rs/pull/18

Changed files:

- `src/bcn/bc6.rs`
- `tests/bc6.rs` (new)

Source changes:

- `unquantize` now works in `i32` for signed endpoints. It reads the signed
  value, computes the magnitude, clamps at the spec threshold, and reapplies the
  sign.
- The unsigned special case is now `(1 << bits) - 1`, not `1 << (bits - 1)`.
- `finish_unquantize` scales the magnitude first and sets the half-float sign
  only when the scaled value is negative. A negative interpolant that truncates
  to zero produces positive zero.
- Endpoint interpolation uses signed `i32` values. Public functions and RGBA
  channel packing are unchanged.

No GPL implementation was copied. The patch is original code based on the
Microsoft BC6H specification and the independent bcdec reference. bcdec was
used only as an external oracle.

## 4. Validation

- `cargo test --test bc6`: 2 passed after the fix; 2 failed on the base commit.
- `cargo test`: 42 existing tests plus the 2 new tests pass.
- `rustfmt --check src/bcn/bc6.rs tests/bc6.rs`: clean.
- Differential fuzz against bcdec, deterministic seed, 10 valid mode codes,
  40 synthetic blocks per mode:
  - signed: 1,526 mismatched channels before, 0 of 19,200 after.
  - unsigned: 51 mismatched channels before, 0 of 19,200 after.
- The differential compares RGB eight-bit channels in the public RGBA output
  order. It does not compare the half-float path because this crate exposes only
  eight-bit RGBA for BC6H.
- No game assets or decoded game data were used.

## 5. Limits

- The 16-bit transformed-endpoint mask remains PR #17's scope. The two patches
  touch the same function but different lines; a rebase is expected.
- Signed zero is not independently observable through this crate's eight-bit
  API. The finish order is corrected for spec consistency, but the test suite
  cannot distinguish `+0.0` from `-0.0` after the clamp.
- This is a block-decoder correctness fix, not a full texture extraction or
  history-recovery claim.

## 6. Upstream follow-up

The reviewed patch is submitted as PR #18. It references issue #10 and keeps
PR #17's mask correction separate. The two synthetic regressions and oracle
provenance are included. It has not been merged.

Parent validation repeated `cargo test` (44 tests pass), rustfmt and the diff
check. `cargo test --no-default-features --lib` fails in unchanged Crunch/PVRTC
code; this patch does not claim a passing no-alloc build.
