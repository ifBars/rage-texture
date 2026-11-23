# Release-blocker progress

Checkpoint: November 22, 2026. The SDK remains unpublished and in development.

## Completed in this checkpoint

- Exhaustive decoding: 1,026,833/1,026,833 chunks across 57 stores match pinned
  SixNG, including codec and unfitted/invalid diagnostics; no decoded cache.
- Current BC7 table: all 5,167 chunks match SixNG's later 34-preset update;
  251 chunk hashes change and all 839,119 BC7 missing-preset flags disappear.
  Every result remains provisional and all confirmed-policy attempts refuse.
  This affected-codec check is layered onto the frozen exhaustive audit.
- Exhaustive exports: 357,115/357,115 full DDS outputs pass independent
  container checks. They total 149,464,150,932 bytes, hashed without retaining
  the complete dump. All observed 2D, array, cube and volume descriptors pass.
- Consumer QA: DirectXTex checks 631 real mip/slice subresources across all
  20 observed formats. Exact, masked and one-level quantized comparisons are
  labelled separately. All six malformed/oracle/mask controls are rejected.
- Preview repairs: BC1 one-bit alpha and BC3 four-colour decoding fixed, with
  failing-before regressions and independent synthetic DirectXTex confirmation.
  Updated output matches 843/843 mip/slices against combined SixNG fixes.
- Regression checks: current CLI and Python SDK retain all 1,076 saved complete
  DDS references. Formatting and Clippy pass. Rust: 138 core, seven consumer
  and 16 example tests; Python/report/adapter: 33 tests.
- Reproducibility: independent pinned SixNG corpus harness and a reconciliation
  gate are included. Missing, duplicated, unordered or incomplete records,
  cache use, mismatched diagnostics and mismatched digests fail the gate.

## Uncached performance retained

After stopping audit/compiler work, seven alternating DDS trials measure
8.4x-30.0x over Leonida, with equal bytes. A 64-texture batch including
writes/fsync measures 29.0x; two selected real provisional BC6H Python exports
measure 32.5x and 71.0x. The native before/after control is within about 4.3%.

A separate control keeps Leonida's normal within-export reuse enabled while
the native cache stays disabled: 6.2x-23.3x on three workloads, equal bytes.
These sampled warm-OS-cache results do not establish a general SixNG speed lead.
Timing boundaries and fingerprints are in [BENCHMARKS.md](BENCHMARKS.md).

## Upstream

- [SixNG #28](https://github.com/holdmysocks/SixNG/pull/28), A8 alpha, merged.
- [SixNG #27](https://github.com/holdmysocks/SixNG/pull/27), mip/volume layouts,
  remains open with green Windows/Linux/macOS CI.
- [SixNG #35](https://github.com/holdmysocks/SixNG/pull/35), BC1/2/3 previews,
  merged after green Windows/Linux/macOS release builds and tests.
- [texture2ddecoder #18](https://github.com/K0lb3/texture2ddecoder-rs/pull/18)
  fixes signed BC6H and the unsigned maximum threshold without duplicating #17.
- [Leonida #4664](https://github.com/ItsNotRexed/Leonida/pull/4664) prevents
  padding from confirming history. The review follow-up reduces peak allocation
  by 29.1% in the reviewer's 1 MiB synthetic fixture; all 20 calibration tests
  pass. Fork CI awaits maintainer approval.

## Remaining release blockers

Independent confirmed BC6H/BC7 initial-history accuracy remains open.
The exhaustive built-in strict census refuses all 268 BC6H and 5,167 BC7 chunks.
The later BC7 preset update leaves 124,047 flagged BC1 blocks, down from
963,166 total flags in the frozen audit. Zero BC7 missing-preset flags and zero
invalid blocks do not make fitted output exact.

Full preview batching, broader layouts/builds, API stabilization and hosted
SDK cross-platform validation also remain open. The complete acceptance
matrix is [FEATURE_PARITY.md](FEATURE_PARITY.md).
