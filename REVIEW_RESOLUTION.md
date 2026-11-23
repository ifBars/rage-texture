# Review follow-through

2026-11-22 checkpoint. `REVIEW.md` and `UNCACHED_REVIEW.md` are independent
snapshots; this file records subsequent work without rewriting their history.

| Finding | Disposition |
|---|---|
| D1 cache eviction scan | Fixed with BTreeMap recency; existing budget/eviction tests pass |
| D2 batch decoded working-set bound | Fixed and independently tested |
| D3 exact zlib extent strictness | 891 sampled chunks pass; exhaustive corpus check remains open |
| D4/D5 metadata reservation | Documented; unmapped prefix fails explicitly; reader padding fixture renamed to avoid implying an unaligned catalog |
| D6 licenses | Notices cover bundled code and all direct dependencies, including tempfile; source headers distinguish Leonida's GPL-or-later grant from combined GPL-3.0-only distribution |
| D7 benchmark drift | Replaced historical claims with cache-disabled DDS and SixNG reader results, methodology, fingerprints and limitations |
| D8 CLI documentation | Complete help and README cover all commands, policies, selectors and resumption |
| D9 publication/resumption | tempfile-backed publication replaces hard-link implementation; existing outputs verified against fresh bytes; differing files remain untouched |
| D10 format interpretation | Review's identification of DXGI 10 as TYPELESS was incorrect: it is R16G16B16A16_FLOAT. Actual BC typeless previews now fail instead of silently selecting UNORM/signedness; typed float behavior and rejection are tested |
| D11 CLI tests | Real CLI fixture covers catalog, PNG, DDS batch, resumption, differing-output preservation and invalid DDS selectors |
| D12 counter epochs | README documents reset/refresh statistics scope |
| D13 allocation equality | All 1,076 sampled complete DDS exports still match with equality enforced; untested/slack allocations remain a coverage risk, not declared supported |
| D14 generated artifacts | All nested target and Python cache directories ignored; reference harness source remains separate from shipped library |
| F1 provenance cardinality | ExtractedTexture documents per-contributing-chunk-read order and possible fallback duplicates; never indexed by mip/slice |
| F2 unreachable volume branch | Removed redundant branch; supported single-mip volume uses bounded input buffer; existing region limit is preserved |
| F3 output allocation | Projected DDS payload checked before reading the temporary input span |
| F4/F5 uncached work metrics | Input span bytes, packed physical bytes and decode counts recorded; benchmark asserts zero cache hits/residency/evictions |

DXGI numeric identities were checked against Microsoft's
[DXGI_FORMAT enumeration](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format).
Signed BC4/BC5 and unimplemented preview formats remain explicit gaps. A cheap
header digest would not detect arbitrary timestamp-preserving payload edits;
explicit refresh remains necessary under the documented metadata-guard model.

Final code validation: formatting and clippy clean; 72 Rust tests pass. The
latest native binary rechecked all 1,076 saved DDS references without mismatch.
No release, publication or universal replacement claim is made.
