# Public BC6H/BC7 history references

Checked November 22, 2026. Public sources can improve coverage and supply
another implementation to compare, but none of the candidates below establishes
an independent byte-exact initial-history oracle for this supplied build.

| Candidate | Evidence | Use here |
|---|---|---|
| [Leonida history calibration](https://github.com/ItsNotRexed/Leonida/blob/aa2988cc555bad7ecd508b48a702adbb32fe7c95/tools/bcpack_history_calib.py) | Distinguishes exact reference-pair inversion from provisional pool self-calibration. Embedded BPTC initial tables are absent. | Validate external JSON/algorithm compatibility; pool fits remain provisional. |
| [GTA6 Asset Info BCPack notes](https://github.com/TrinitronClient/GTA6-Asset-Info/blob/690a363674bc13771e59b252c7bf77fab898a78e/formats/06-bcpack.md) | Reports broad real coverage, fitted BC6H/BC7 histories and unresolved byte-exact reconstruction. | Corroborates the missing-oracle problem; coverage is separate from exactness. |
| [6to5m](https://github.com/DesignWithTommy/6to5m) | Ships a fitted history and credits Leonida's reconstructors/calibrator; documents remaining BC6H/BC7 errors. | Potential provisional baseline, not independent confirmed tables. No binary table imported into this SDK. |
| [bcpack-decoder](https://github.com/Tim0219800/bcpack-decoder) | BC1 reconstruction only; BC6H/BC7 entropy support. Learned tables are not bundled. | Another entropy implementation; no missing BPTC reconstruction oracle. |
| [Xbox2Windows](https://github.com/R0Y00/Xbox2Windows) | Public BC7 mode-6 synthetic fixture is referenced by other tools. | Synthetic reconstruction evidence does not establish real initial tables. |
| [PercyStar texture reader](https://github.com/rpatty-dev/PercyStar/blob/main/src/tex.rs), [Percy6Stars texture reader](https://github.com/Ahmet-Dedeler/Percy6Stars/blob/main/src/tex.rs) | Current source decodes BCPack BC1 through fitted `bcpack-rs` tables; other BCPack codecs use a reduced-mip/preview fallback. | No independent BC6H/BC7 initial-history table to import. |

The user's local dump contains the packed inputs, but no independently decoded
reference DDS/block pairs or confirmed history JSON were supplied. Comparing
the same fitted values through two reconstructors establishes compatibility
only. Feeding fitted output back into a reference-pair learner and labelling
the result confirmed would be circular and is not an acceptance test.

Useful evidence that can be collected without such an oracle:

- Exact lane consumption and dependency inventories for bounded real chunks.
- Successful real external-history reconstruction with provisional labels,
  compared against unmodified Leonida, with missing/refused cases separate.
- Independent block-to-pixel checks on original raw/zlib BC6H blocks. The
  real lighting sample passes 496,640 blocks and 23,838,720 half-float values
  against bcdec, across 14 signed/unsigned textures. Two unavailable samples
  are not counted as passes. This checks the pixel decoder, not BCPack seeds.
- Cross-tool mip/slice/layout comparisons, retaining missing mips and wrong
  pixels as failures until their causes are reproduced and repaired.

Confirmed real initial-history accuracy remains open until exact references,
independently justified full-bit recovery, or an equivalent oracle is available.

## Additional checks during exhaustive QA

An inventory across all 57 stores found no streamed-cubemap candidate
(dimension 2, non-tail mips present, DXGI 94-99) whose allocation snapshots
touch BC6H/BC7 BCPack chunks. The repeated streamed-cubemap-copy route therefore
supplies no reference pair in this build.

A separate scan groups 74,006 keyed BC6H/BC7 texture identities by format,
dimension and depth across the complete corpus. It finds no group containing
both a fully lossless raw/zlib export and a fitted export, even at different
sizes. Thus no duplicate-key raw reference candidate was found. Matching
identifiers alone would still need independent content-identity evidence.

The exhaustive built-in confirmed-history census attempts all 268 BC6H and
5,167 BC7 chunks. All are explicitly refused for missing history. The fitted
compatibility run initially reports 963,166 unfitted blocks: 124,047 BC1 and
839,119 BC7. The later SixNG PR #31 table update removes all BC7 flags across
all 5,167 chunks; the updated SDK matches every current-reference hash and
still refuses every chunk in confirmed mode. This closes missing fitted
presets, not the independent-history evidence gap.
Other recovery methods still require independent investigation.

A synthetic reference test exposed a false-confirmation path in Leonida's
calibrator: one valid BC6H block in a 4x4 DDS caused 255 initial endpoint entries
to be marked as seen because zero-filled tile padding was passed as reference
data. [Leonida #4664](https://github.com/ItsNotRexed/Leonida/pull/4664) adds
complete-block coverage masks for scalar BC, BC6H and BC7 inversion.
It retains shared-tail coverage and complete raw-pair behavior. The regression
fails before the fix; all 20 calibration tests pass after the review follow-up.
One coverage byte per block reduces peak allocation by 29.1% in the reviewer's
1 MiB fixture. [Review evidence](UPSTREAM_HISTORY_QA.md). The PR is open.

This prevents false evidence; it does not recover the missing real history.
Our self-calibrated comparisons remain provisional throughout.
