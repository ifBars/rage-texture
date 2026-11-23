# Layout / detile / DDS integration notes

Owned files: `src/layout.rs`, `src/dds.rs`, `tests/layout.rs`, this note.
Upstream: Leonida `aa2988cc555bad7ecd508b48a702adbb32fe7c95` (GPL-3.0-only),
`tools/tex_headers.py` layout functions and `tools/ttpf_mips.py` detile/DDS path.

## Public API

```rust
pub struct TextureShape { width, height, depth, mips, dxgi, dimension, mode, non_tail: u32 }

pub fn layout(shape: &TextureShape) -> io::Result<SurfaceLayout>;
pub fn layout_with_chunk_pages(shape: &TextureShape, pages: Option<&[u32]>) -> io::Result<SurfaceLayout>;
pub fn detile(shape, layout, data: &[u8], mip: u32, slice: u32) -> io::Result<Vec<u8>>;
pub fn detile_storage(shape, layout, data, mip, slice) -> io::Result<Vec<u8>>;
pub fn write_dds(shape: &TextureShape, subresources: &[Vec<u8>]) -> io::Result<Vec<u8>>;
```

Also `SurfaceLayout::{mip, subresource_count, subresource_offset, subresource_len,
slice_len}`, `format_info(dxgi)`, `gfx10_tile_mode`, `gfx10_tile_bytes`.

## Reading one subresource

1. `layout = layout_with_chunk_pages(&texture.shape(), descriptor_pages)`.
   `descriptor_pages` are the decoded 4 KiB page counts from `(extra << 11) | meta`.
   For a streaming cubemap (`dimension == 2`, `non_tail > 0`) they are required to
   check the chunk count and set `data_offset` / `total_bytes`. For all other shapes
   they are ignored.
2. `offset = layout.subresource_offset(mip, slice)`; read `layout.mip(mip)?.region`
   bytes at `base + offset`. The offset already includes the cubemap `data_offset`
   and the array/cube slice stride.
3. `linear = detile(&shape, &layout, &raw, mip, slice)`.
   - 2D/array/cubemap: `slice` is the array slice or cube face; output is the
     tightly packed visible block grid (BC) or texel rows (uncompressed), cropped
     to the blocks/rows that hold visible texels. Use `detile_storage` if the full
     padded storage grid is needed instead.
   - 3D volume: read the whole mip once (`slice` only selects Z for detile); output
     is one Z slice in row-major order. Loop `z in 0..mip.pixels_d` and concatenate.
4. `layout.total_bytes` is the allocation span. For a non-streaming cubemap
   `layout(shape)` is enough; for a streaming cubemap use the page-aware call so the
   span check is meaningful.

## DDS subresource order and sizes

`write_dds` wants **slice-major, then mip**:

- 2D: mip 0..N.
- 2D array: slice 0 mips 0..N, slice 1 mips 0..N, ...
- Cubemap: face 0 mips 0..N, face 1 mips 0..N, ... (6 faces).
- 3D volume: one vector per mip, that vector contains all Z slices in order.

Each vector must be exactly `layout.subresource_len(mip)` bytes; short or long
input is rejected (no zero fill). A leading partial mip chain is accepted:
`subresources.len()` must be `slices * levels` (or `levels` for a volume) and the
DDS header records `levels`.

## Deliberate validation limits (match upstream)

- 2D: any known DXGI format with a compact element size 1/2/4/8/16 bytes.
  12-byte formats (DXGI 6/7/8) are known but have no measured compact rule and
  fail as unsupported.
- 2D arrays: `dimension == 1`, `depth > 1`, mode must be 1 (mip-major SW_64KB_D).
- Cubemaps: `dimension == 2`, `depth == 6`, `non_tail <= 3`, and
  `mode == 18 + 2 * non_tail`.
- 3D: `dimension == 3`, mode 2, exactly one mip, uncompressed only.
- BC formats use whole 4x4 blocks and block typeless IDs are accepted; the DDS
  writer derives top-mip linear size from visible blocks.

## Validation / current gate

`cargo test --locked` passes, including `tests/layout.rs` synthetic parity tests
for 4K/64K compact tails, odd-size cropping, arrays, cubemaps (including padded
final chunks), volumes, DDS headers/order, and upstream-derived detile hashes.
`cargo clippy --all-targets -- -D warnings` and `rustfmt --check` are clean for
the owned files. During development the layout and detile output were also
compared exactly against the upstream Python implementation over 150 randomized
synthetic shapes (2D, arrays, cubemaps, volumes; every mip and sampled slices),
with zero mismatches.
