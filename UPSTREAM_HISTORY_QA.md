# Reference history calibration QA

[Leonida #4664](https://github.com/ItsNotRexed/Leonida/pull/4664) prevents
padding outside a reference DDS from being mistaken for exact BC blocks.
The original synthetic regression marked 255 BC6H initial endpoint entries
as seen from a reference containing just one visible block. Only that block
can provide evidence after the fix.

The implementation retains a coverage map shared by mips in one tail tile.
Scalar BC, BC6H and BC7 inversion ignore blocks without reference coverage.
Complete raw reference pairs retain their existing behavior.

## Review follow-up

The reviewer correctly identified excess allocation: the first patch stored
one coverage byte per payload byte. The follow-up stores one byte per complete
BC block and bounds the temporary chunk mask to the first 256 blocks used by
initial-history inversion. For BC6H/BC7, the region coverage allocation is
one sixteenth of its former size; for BC1/BC4 it is one eighth.

The reviewer's synthetic measurement script was repeated on Windows x64,
CPython 3.12.7. Peak Python bytes allocated during `apply_ref_dds`:

| Block payload bytes | Base | Initial PR | Follow-up |
|---:|---:|---:|---:|
| 4,096 | 106,820 | 123,798 | 113,918 |
| 65,536 | 296,098 | 370,204 | 302,812 |
| 1,048,576 | 2,321,042 | 3,398,372 | 2,409,380 |

The largest case reduces peak allocation by 988,992 bytes (29.1%) versus the
first patch. Remaining overhead versus the base is 88,338 bytes. All versions
observe 255 endpoint entries in this fully covered synthetic fixture. This
measures peak allocation during the call, not retained memory or game-corpus
performance.

All 20 calibration tests pass, including padding, shared-tail coverage, strict
six-codec reconstruction and the new block-mask allocation regression. The
three text line-ending checks and Python compilation pass. No GPU or game data
is needed. Hosted checks await maintainer approval for the fork contribution.

The reviewer independently repeated the 1 MiB probe at `61fe62af` on Windows
11 / CPython 3.14.7: peak allocation fell from 3,524,224 to 2,511,256 bytes,
with the same 255 observed entries and all 20 tests passing.
[Reviewer confirmation](https://github.com/ItsNotRexed/Leonida/pull/4664#issuecomment-7091537319).

This repair strengthens the reliability of reference evidence. It does not
recover or independently confirm the missing real BC6H/BC7 initial histories.
