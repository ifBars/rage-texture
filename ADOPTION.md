# Adoption path

The next useful product is an asset extraction toolkit backed by this SDK.
A PC game port needs far more than texture extraction; it is not the current
product commitment. The SDK should also be useful when someone already has a
viewer, port or extraction tool and only wants a better texture backend.

## What can be used now

- Rust consumers can use the checked Store API, validated layouts, complete
  DDS output and full-precision BC6H decoder directly.
- Python consumers can use the separate native wheel without subprocess or
  JSON overhead. Descriptors keep their source generation, quality diagnostics
  remain available, and read/decode work releases the GIL.
- The experimental Leonida adapter bridges strict/provisional history-aware DDS
  exports and leading mip selection. Three-mip cube/array workloads are 10.0x
  and 23.9x faster, with identical
  bytes and decoded caches disabled. All 1,076 saved DDS references also pass
  through the installed Python SDK.
- [SixNG PR #24](https://github.com/holdmysocks/SixNG/pull/24) contributes the
  canonical signed-zero fix and a regression to the existing BC6H PR #23.
  Both PRs have merged; the decoder work remains credited to that author. Our independent half-bit
  comparison and targeted vectors support the fix.

## What makes adoption worthwhile

Consumers need usable interfaces, accurate output and reproducible gains.
The speed comparisons are one part of that evidence. Explicit failures,
per-chunk provenance, source-update handling and fixtures that do not require
someone else's game files make the library easier to evaluate and maintain.
GPL-3.0-only is compatible with Leonida and SixNG; consumers with different
licensing requirements need to assess that before adoption.

## Next product milestones

1. Extend the implemented confirmed/custom-history path with real external
   BPTC table validation, resolve remaining baseline disagreements and validate
   unmeasured layouts. Signed BC4/5 previews are implemented. Keep independent BC6H block accuracy separate from fitted BCPack
   reconstruction accuracy.
2. Make batch extraction complete: selected formats, every requested mip and
   slice, progress/cancellation, bounded parallel work, source-aware resumption
   and a machine-readable coverage/error report. Preserve original DDS/HDR
   data alongside optional previews.
3. Measure entire batch wall-clock time, memory and output equality. Include
   writes/PNG encoding and both baselines, with decoded caches disabled first.
   Keep disk-cache conditions and any unsupported cases visible.
4. Evaluate a general Leonida backend now that leading mip requests and
   strict/provisional custom-history policies are bridged. Missing-history
   coverage differences and real BPTC table validation still need resolution.
5. Build a desktop asset browser on this tested backend once batch extraction
   and source indexing are dependable. Searching, inspecting and exporting
   assets should share the same APIs and diagnostics.

## Upstream work

Prefer small fixes maintainers can reproduce and review over a broad dependency
replacement. The SixNG fix is an example: one file, a failing synthetic test,
Microsoft's specified behavior and an independent reference. SixNG #28 merged;
#35 is merged; #27 has green three-platform CI. Leonida #4664 protects reference
history evidence, and texture2ddecoder #18 repairs BC6H endpoint decoding. Upstream
acceptance remains separate from the SDK's release gate.

The SDK currently vendors attributed SixNG codec modules to avoid pulling in
its whole core dependency graph. A separately exposed upstream codec crate or
feature could reduce that maintenance burden later. Do not make our consumers
wait for that refactor or remove attribution from the current copies.

No SDK release, universal replacement claim or popularity guarantee follows
from these milestones. Current evidence and unresolved gates live in
[FEATURE_PARITY.md](FEATURE_PARITY.md), [BENCHMARKS.md](BENCHMARKS.md) and
[ACCURACY.md](ACCURACY.md).
