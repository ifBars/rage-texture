# Experimental Leonida DDS backend

This adapter shows a narrow integration with Leonida's existing Python export
flow. It uses the in-process Python SDK, preserving source-generation checks
and avoiding subprocess/JSON overhead. No changes to Leonida are required to
try it locally. It is not a proposed default backend yet.

Build and install [the Python wheel](../../bindings/python/README.md) into an
isolated environment. Import Leonida's `tools/ttpf_mips.py` as usual, then use:

```python
from native_dds import NativeDdsBackend  # add this directory to sys.path
import ttpf_mips

pool = ttpf_mips.Pool("pool.ttpf")
try:
    analysis = ttpf_mips.analyze(pool, 0, allow_bcpack=True)
    with NativeDdsBackend(pool.path) as backend:
        result = backend.export(pool, analysis)
        dds_bytes = result.data
        print(result.quality, result.input_bytes)
finally:
    pool.close()
```

`backend.texture_dds(pool, analysis)` returns bytes for an existing bytes-based
call site. `backend.export` additionally retains provenance and metrics. Parsed
catalogs stay in memory while the backend is open, but decoded caching is
disabled. Every extraction checks source identity and timestamps. After an
update, close the backend and re-open/re-analyze both readers.

The adapter checks descriptor agreement and refuses blocked/unavailable
analysis. It supports complete or leading partial DDS mip chains, strict confirmed
histories and explicitly enabled provisional histories. Use constructor
`history=PATH_OR_DICT` and `allow_provisional=True` when requested, and
`backend.export(pool, analysis, mips=COUNT)`. A per-call history override is
loaded afresh and invalidates the catalog if its digest changes. Decode policy
is fixed per backend; conflicting per-call policy requests fail explicitly.
Missing history bits remain unavailable; see [HISTORY.md](../../HISTORY.md).
Use a separate backend per worker thread.

Validated against texture tools at `aa2988cc555bad7ecd508b48a702adbb32fe7c95`;
the same texture-tool files are unchanged at the inspected current Leonida head
`03b09a086592a197db64d91655c6e65c30764d96`.

Run synthetic adapter tests after installing the wheel:

```powershell
python -m unittest discover -s integrations/leonida -p "test_*.py" -v
```

Use `scripts/compare_python_sdk.py` to compare the real in-process adapter
against unmodified Leonida, with decoded caches disabled on both sides. The
benchmark includes descriptor checks and Python/Rust bytes conversion. It
excludes opening/catalog parsing, startup, hashing and file writes and uses
warm OS cache. It fails on byte disagreement or any native cache use.
