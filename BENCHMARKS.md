# Benchmarks

Measured on Windows x64 against the supplied Xbox build. Both decoded caches
were disabled. Every timed trial produced equal DDS bytes.

## Complete DDS construction

Seven alternating trials per workload:

| Workload | Native median ms | Leonida median ms | Ratio |
|---|---:|---:|---:|
| plain | 0.200 | 1.674 | 8.4x |
| mips | 0.948 | 17.302 | 18.3x |
| cube | 0.426 | 8.502 | 20.0x |
| volume | 1.535 | 37.246 | 24.3x |
| volume64 | 2.657 | 79.720 | 30.0x |
| array | 5.263 | 134.659 | 25.6x |
| platform | 2.557 | 73.884 | 28.9x |

Times include source checks and DDS construction. They exclude process startup,
store/catalog setup, verification hashing and file writes. The filesystem
cache was warm. These selected workloads do not estimate whole-corpus speed.

A separate 64-texture batch across 46 stores writes 29,706,888 equal bytes,
including flush and fsync. Across three alternating trials, the native median
was 0.197 seconds and Leonida's was 5.696 seconds (29.0x). Both implementations
used the same Python file writer.

A control with Leonida's normal within-export reuse enabled measured 6.2–23.3x
on three workloads while the native decoded cache stayed disabled. This is
separate from the strict cache-disabled comparison above. An overall SixNG
speed lead has not been established.

## Reproduce

`scripts/compare_export_matrix.py` runs named complete DDS workloads against a
local Leonida checkout. `scripts/compare_python_batch.py` measures sampled
batches with file writes. Both accept external output paths and verify bytes
and cache counters. Run each script with `--help` for the required inputs.

Use your own local stores and keep generated reports outside the repository.
Record source identity, tool revisions and workload selection when sharing
results. OS cache state and hardware affect these measurements.
