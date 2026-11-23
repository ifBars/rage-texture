//! Synthetic layout, detile and DDS tests.
//!
//! All fixtures are generated in memory. Expected layout numbers and detiled
//! hashes were produced by Leonida's `tools/tex_headers.py` and
//! `tools/ttpf_mips.py` at revision `aa2988cc555bad7ecd508b48a702adbb32fe7c95`
//! on synthetic buffers; no game data is involved.

use rage_texture::dds::write_dds;
use rage_texture::layout::{
    FormatInfo, SurfaceKind, TextureShape, detile, detile_storage, format_info, layout,
    layout_with_chunk_pages,
};
use std::io::ErrorKind;

#[allow(clippy::too_many_arguments)]
fn shape(
    width: u32,
    height: u32,
    depth: u32,
    mips: u32,
    dxgi: u32,
    dimension: u32,
    mode: u32,
    non_tail: u32,
) -> TextureShape {
    TextureShape {
        width,
        height,
        depth,
        mips,
        dxgi,
        dimension,
        mode,
        non_tail,
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

fn pattern_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i.wrapping_mul(37).wrapping_add(11)) & 0xFF) as u8)
        .collect()
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn format_table_matches_measured_sizes() {
    assert_eq!(
        format_info(71),
        Some(FormatInfo {
            element_bytes: 8,
            block: 4
        })
    );
    assert_eq!(
        format_info(98),
        Some(FormatInfo {
            element_bytes: 16,
            block: 4
        })
    );
    assert_eq!(
        format_info(11),
        Some(FormatInfo {
            element_bytes: 8,
            block: 1
        })
    );
    assert_eq!(
        format_info(65),
        Some(FormatInfo {
            element_bytes: 1,
            block: 1
        })
    );
    assert_eq!(format_info(999), None);
}

#[test]
fn layout_2d_bc1_4k_matches_upstream() {
    // 1024x1024 BC1, 9 mips, mode 2 => SW_4KB_S.
    let shape = shape(1024, 1024, 1, 9, 71, 1, 2, 0);
    let layout = layout(&shape).unwrap();
    assert_eq!(layout.kind, SurfaceKind::Texture2D);
    assert_eq!(layout.total_bytes, 700_416);
    assert_eq!(layout.tile, 4096);
    assert_eq!(layout.element_bytes, 8);
    assert_eq!(layout.block, 4);
    assert_eq!(layout.first_tail, 4);
    assert_eq!(layout.block_w, 32);
    assert_eq!(layout.block_h, 16);
    assert_eq!(layout.levels, 11);
    assert_eq!(layout.slices, 1);
    assert_eq!(layout.mips.len(), 9);

    assert_eq!(layout.mips[0].offset, 176_128);
    assert_eq!(layout.mips[0].region, 524_288);
    assert_eq!(layout.mips[1].offset, 45_056);
    assert_eq!(layout.mips[1].region, 131_072);
    assert_eq!(layout.mips[2].offset, 12_288);
    assert_eq!(layout.mips[2].region, 32_768);
    assert_eq!(layout.mips[3].offset, 4096);
    assert_eq!(layout.mips[3].region, 8192);

    assert!(layout.mips[4].in_tail);
    assert_eq!(layout.mips[4].offset, 0);
    assert_eq!(layout.mips[4].region, 4096);
    assert_eq!(layout.mips[4].padded_w, 32);
    assert_eq!(layout.mips[4].padded_h, 16);
    assert_eq!((layout.mips[4].tile_x, layout.mips[4].tile_y), (16, 0));
    assert_eq!(layout.mips[4].tail_offset, Some(2048));
    assert_eq!(layout.mips[5].tail_offset, Some(1536));
    assert_eq!((layout.mips[6].tile_x, layout.mips[6].tile_y), (0, 12));
    assert_eq!(layout.mips[8].tail_offset, Some(768));
    assert_eq!((layout.mips[8].tile_x, layout.mips[8].tile_y), (8, 4));

    assert_eq!(layout.subresource_len(0).unwrap(), 524_288);
}

#[test]
fn layout_2d_bc1_64k_matches_upstream() {
    // mode 0 => SW_64KB_D, with the 12-entry tail.
    let shape = shape(1024, 1024, 1, 9, 71, 1, 0, 0);
    let layout = layout(&shape).unwrap();
    assert_eq!(layout.total_bytes, 720_896);
    assert_eq!(layout.tile, 65_536);
    assert_eq!(layout.first_tail, 2);
    assert_eq!(layout.block_w, 128);
    assert_eq!(layout.block_h, 64);
    assert_eq!(layout.mips[0].offset, 196_608);
    assert_eq!(layout.mips[1].offset, 65_536);
    assert_eq!(layout.mips[2].region, 65_536);
    assert_eq!(layout.mips[2].tail_offset, Some(32_768));
    assert_eq!((layout.mips[2].tile_x, layout.mips[2].tile_y), (64, 0));
    assert_eq!(layout.mips[8].tail_offset, Some(1280));
}

#[test]
fn layout_odd_uncompressed_tail_and_crop_geometry() {
    // 17x13 R8, 5 mips: every mip fits the 4 KiB packed tail.
    let shape = shape(17, 13, 1, 5, 61, 1, 2, 0);
    let layout = layout(&shape).unwrap();
    assert_eq!(layout.total_bytes, 4096);
    assert_eq!(layout.first_tail, 0);
    assert_eq!(layout.block_w, 64);
    assert_eq!(layout.block_h, 64);
    assert_eq!(layout.levels, 5);
    assert!(layout.mips.iter().all(|m| m.in_tail && m.region == 4096));

    assert_eq!(layout.mips[0].blocks_w, 17);
    assert_eq!(layout.mips[0].blocks_h, 13);
    assert_eq!(layout.mips[0].tile_x, 32);
    assert_eq!(layout.mips[0].tail_offset, Some(2048));
    assert_eq!(layout.mips[1].blocks_w, 9);
    assert_eq!(layout.mips[1].blocks_h, 7);
    assert_eq!(layout.mips[1].pixels_w, 8);
    assert_eq!(layout.mips[1].pixels_h, 6);
    assert_eq!((layout.mips[1].tile_x, layout.mips[1].tile_y), (16, 32));
    assert_eq!(layout.mips[1].tail_offset, Some(1536));
    assert_eq!(layout.mips[4].blocks_w, 2);
    assert_eq!(layout.mips[4].blocks_h, 1);
    assert_eq!((layout.mips[4].tile_x, layout.mips[4].tile_y), (16, 16));
    assert_eq!(layout.mips[4].tail_offset, Some(768));
}

#[test]
fn layout_bc7_tail_origins_match_upstream() {
    let shape = shape(512, 512, 1, 10, 98, 1, 2, 0);
    let layout = layout(&shape).unwrap();
    assert_eq!(layout.total_bytes, 352_256);
    assert_eq!(layout.first_tail, 4);
    assert_eq!(layout.block_w, 16);
    assert_eq!(layout.block_h, 16);
    assert_eq!(layout.mips[0].offset, 90_112);
    assert_eq!(layout.mips[3].offset, 4096);
    assert_eq!(layout.mips[4].tail_offset, Some(2048));
    assert_eq!((layout.mips[4].tile_x, layout.mips[4].tile_y), (8, 0));
    assert_eq!(layout.mips[5].tail_offset, Some(1536));
    assert_eq!((layout.mips[5].tile_x, layout.mips[5].tile_y), (4, 8));
    assert_eq!(layout.mips[9].tail_offset, Some(512));
    assert_eq!((layout.mips[9].tile_x, layout.mips[9].tile_y), (4, 0));
}

#[test]
fn layout_array_is_mip_major_with_per_slice_tail() {
    // 512x256 BC4 (8-byte blocks), 8 mips, 3 slices, mode 1 => 64 KiB tiles.
    let shape = shape(512, 256, 3, 8, 80, 1, 1, 0);
    let layout = layout(&shape).unwrap();
    assert_eq!(layout.kind, SurfaceKind::Array2D);
    assert_eq!(layout.total_bytes, 393_216);
    assert_eq!(layout.slice_size, 131_072);
    assert_eq!(layout.tile, 65_536);
    assert_eq!(layout.first_tail, 1);
    assert_eq!(layout.slices, 3);
    assert_eq!(layout.mips[0].offset, 196_608);
    assert_eq!(layout.mips[0].region, 65_536);
    assert_eq!(layout.mips[0].slice_stride, 65_536);
    assert_eq!(layout.mips[1].offset, 0);
    assert_eq!(layout.mips[1].region, 65_536);
    assert_eq!(layout.mips[1].slice_stride, 65_536);
    assert_eq!(layout.mips[1].tail_offset, Some(32_768));
    assert_eq!((layout.mips[1].tile_x, layout.mips[1].tile_y), (64, 0));
    assert_eq!(layout.subresource_offset(0, 2).unwrap(), 327_680);
    assert_eq!(layout.subresource_offset(1, 2).unwrap(), 131_072);
    assert_eq!(layout.subresource_count().unwrap(), 24);
}

#[test]
fn layout_cubemap_snapshots_and_chunk_validation() {
    // 128x128 BC1_SRGB, 8 mips, 6 faces.
    let plain = shape(128, 128, 6, 8, 72, 2, 18, 0);
    let layout = layout(&plain).unwrap();
    assert_eq!(layout.kind, SurfaceKind::Cubemap);
    assert_eq!(layout.total_bytes, 73_728);
    assert_eq!(layout.data_offset, 0);
    assert_eq!(layout.face_size, 12_288);
    assert_eq!(layout.slice_size, 12_288);
    assert_eq!(layout.padded_full_bytes, 73_728);
    assert_eq!(layout.snapshots, 1);
    assert_eq!(layout.slices, 6);
    assert_eq!(layout.mips[0].offset, 4096);
    assert_eq!(layout.mips[0].slice_stride, 12_288);
    assert_eq!(layout.subresource_offset(0, 3).unwrap(), 40_960);
    assert_eq!(layout.subresource_count().unwrap(), 48);

    // Three self-contained snapshots: two earlier chunks plus the 18-page
    // final full-resolution chunk.
    let streaming = shape(128, 128, 6, 8, 72, 2, 22, 2);
    let pages = [6u32, 12, 18];
    let layout = layout_with_chunk_pages(&streaming, Some(&pages)).unwrap();
    assert_eq!(layout.total_bytes, 147_456);
    assert_eq!(layout.data_offset, 73_728);
    assert_eq!(layout.padded_full_bytes, 73_728);
    assert_eq!(layout.snapshots, 3);
    assert_eq!(layout.subresource_offset(0, 3).unwrap(), 114_688);

    // The final chunk is padded to 64 KiB when six faces reach 36 pages.
    let big = shape(512, 512, 6, 10, 98, 2, 20, 1);
    let pages = [204u32, 528];
    let layout = layout_with_chunk_pages(&big, Some(&pages)).unwrap();
    assert_eq!(layout.face_size, 352_256);
    assert_eq!(layout.padded_full_bytes, 2_162_688);
    assert_eq!(layout.data_offset, 835_584);
    assert_eq!(layout.total_bytes, 2_998_272);

    // Missing pages give the final snapshot alone.
    let layout = layout_with_chunk_pages(&big, None).unwrap();
    assert_eq!(layout.data_offset, 0);
    assert_eq!(layout.total_bytes, 2_162_688);
}

#[test]
fn layout_volume_micro_tiles_match_upstream() {
    let r16f = shape(256, 32, 8, 1, 54, 3, 2, 0);
    let l = layout(&r16f).unwrap();
    assert_eq!(l.kind, SurfaceKind::Volume3D);
    assert_eq!(l.total_bytes, 262_144);
    assert_eq!((l.block_w, l.block_h, l.block_d), (16, 8, 16));
    assert_eq!((l.mips[0].padded_w, l.mips[0].padded_h), (256, 32));
    assert_eq!(l.padded_d, 16);
    assert_eq!(l.z_tile_stride, 262_144);
    assert_eq!(l.mips[0].region, 262_144);
    assert_eq!(l.mips[0].pixels_d, 8);
    assert_eq!(l.subresource_len(0).unwrap(), 131_072);
    assert_eq!(l.subresource_count().unwrap(), 1);

    let rgba16f = shape(32, 64, 32, 1, 10, 3, 2, 0);
    let l = layout(&rgba16f).unwrap();
    assert_eq!(l.total_bytes, 524_288);
    assert_eq!((l.block_w, l.block_h, l.block_d), (8, 8, 8));
    assert_eq!(l.padded_d, 32);
    assert_eq!(l.z_tile_stride, 131_072);
    assert_eq!(l.mips[0].pixels_d, 32);
    assert_eq!(l.subresource_len(0).unwrap(), 524_288);
}

#[test]
fn detile_matches_upstream_reference_hashes() {
    // Reference values from Leonida ttpfs detile/_visible_rows on the same
    // deterministic synthetic tiled bytes.
    let case = shape(4, 4, 1, 1, 61, 1, 2, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(4096), 0, 0).unwrap();
    assert_eq!(out.len(), 16);
    assert_eq!(fnv1a(&out), 0x4AFF_3FDD_B70C_FE85);

    let case = shape(17, 13, 1, 5, 61, 1, 2, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(4096), 1, 0).unwrap();
    assert_eq!(out.len(), 48);
    assert_eq!(fnv1a(&out), 0x95FD_331D_2D32_7AA5);
    let storage = detile_storage(&case, &l, &pattern_bytes(4096), 1, 0).unwrap();
    assert_eq!(storage.len(), 9 * 7);

    let case = shape(1024, 1024, 1, 9, 71, 1, 2, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(l.mips[0].region as usize), 0, 0).unwrap();
    assert_eq!(out.len(), 524_288);
    assert_eq!(fnv1a(&out), 0xDB96_B8EE_9332_2325);

    let out = detile(&case, &l, &pattern_bytes(4096), 4, 0).unwrap();
    assert_eq!(out.len(), 2048);
    assert_eq!(fnv1a(&out), 0x5D0B_3FB6_1C71_3325);

    let case = shape(512, 256, 3, 8, 80, 1, 1, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(l.mips[0].region as usize), 0, 2).unwrap();
    assert_eq!(out.len(), 65_536);
    assert_eq!(fnv1a(&out), 0x61E7_ED1A_48D7_2325);
    let out = detile(&case, &l, &pattern_bytes(65_536), 1, 2).unwrap();
    assert_eq!(out.len(), 16_384);
    assert_eq!(fnv1a(&out), 0xECFB_A25A_5C4F_6325);

    let case = shape(128, 128, 6, 8, 72, 2, 18, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(4096), 1, 3).unwrap();
    assert_eq!(out.len(), 2048);
    assert_eq!(fnv1a(&out), 0x5D0B_3FB6_1C71_3325);

    let case = shape(256, 32, 8, 1, 54, 3, 2, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(l.total_bytes as usize), 0, 3).unwrap();
    assert_eq!(out.len(), 256 * 32 * 2);
    assert_eq!(fnv1a(&out), 0x93BB_4662_6BDD_A325);

    let case = shape(32, 64, 32, 1, 10, 3, 2, 0);
    let l = layout(&case).unwrap();
    let out = detile(&case, &l, &pattern_bytes(l.total_bytes as usize), 0, 17).unwrap();
    assert_eq!(out.len(), 32 * 64 * 8);
    assert_eq!(fnv1a(&out), 0xED99_74EA_AFEE_2325);
}

#[test]
fn detile_rejects_short_or_mismatched_inputs() {
    let case = shape(17, 13, 1, 5, 61, 1, 2, 0);
    let l = layout(&case).unwrap();
    let err = detile(&case, &l, &pattern_bytes(4095), 1, 0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    let err = detile(&case, &l, &pattern_bytes(4096), 5, 0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let err = detile(&case, &l, &pattern_bytes(4096), 0, 1).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);

    let mut other = case;
    other.dxgi = 71;
    let err = detile(&other, &l, &pattern_bytes(4096), 0, 0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    let cube = shape(128, 128, 6, 8, 72, 2, 18, 0);
    let cube_layout = layout(&cube).unwrap();
    let err = detile(&cube, &cube_layout, &pattern_bytes(4096), 0, 6).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let err = detile(&cube, &cube_layout, &pattern_bytes(4095), 0, 0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    let volume = shape(256, 32, 8, 1, 54, 3, 2, 0);
    let volume_layout = layout(&volume).unwrap();
    let err = detile(&volume, &volume_layout, &pattern_bytes(262_144), 0, 8).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    let err = detile(&volume, &volume_layout, &pattern_bytes(262_143), 0, 0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn unsupported_shapes_fail_explicitly() {
    assert_eq!(
        layout(&shape(1024, 1024, 1, 12, 71, 1, 2, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        layout(&shape(4, 4, 1, 1, 999, 1, 2, 0)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    // 12-byte texels are in the DXGI table but have no measured compact rule.
    assert_eq!(
        layout(&shape(4, 4, 1, 1, 6, 1, 2, 0)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(64, 64, 2, 1, 71, 1, 0, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(128, 128, 6, 8, 72, 2, 18, 2))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(128, 128, 6, 8, 72, 2, 22, 1))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(256, 32, 8, 2, 54, 3, 2, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(256, 32, 8, 1, 71, 3, 2, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(256, 32, 8, 1, 54, 3, 1, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        layout(&shape(4, 4, 1, 1, 61, 7, 2, 0)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );

    // A cubemap chunk list with the wrong number of chunks is rejected.
    let cube = shape(128, 128, 6, 8, 72, 2, 22, 2);
    assert_eq!(
        layout_with_chunk_pages(&cube, Some(&[6, 12]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    // A final chunk that is not the computed full-resolution size is rejected.
    assert_eq!(
        layout_with_chunk_pages(&cube, Some(&[6, 12, 17]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn dds_2d_header_and_payload() {
    let case = shape(1024, 1024, 1, 9, 71, 1, 2, 0);
    let l = layout(&case).unwrap();
    let subs: Vec<Vec<u8>> = (0..9)
        .map(|mip| vec![mip as u8; l.subresource_len(mip).unwrap()])
        .collect();
    let dds = write_dds(&case, &subs).unwrap();
    assert_eq!(&dds[..4], b"DDS ");
    assert_eq!(u32_at(&dds, 4), 124);
    assert_eq!(u32_at(&dds, 8), 0x1007 | 0x8_0000 | 0x2_0000);
    assert_eq!(u32_at(&dds, 12), 1024);
    assert_eq!(u32_at(&dds, 16), 1024);
    assert_eq!(u32_at(&dds, 20), 524_288);
    assert_eq!(u32_at(&dds, 24), 0);
    assert_eq!(u32_at(&dds, 28), 9);
    assert_eq!(u32_at(&dds, 76), 32);
    assert_eq!(u32_at(&dds, 80), 4);
    assert_eq!(&dds[84..88], b"DX10");
    assert_eq!(
        &dds[128..148],
        &[71, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(dds.len(), 148 + subs.iter().map(Vec::len).sum::<usize>());
    assert_eq!(&dds[148..152], &[0, 0, 0, 0]);
    let second = 148 + subs[0].len();
    assert_eq!(dds[second], 1);

    // A leading partial chain is legal and writes the requested mip count.
    let partial = write_dds(&case, &subs[..4]).unwrap();
    assert_eq!(u32_at(&partial, 28), 4);
    assert_eq!(u32_at(&partial, 8), 0x1007 | 0x8_0000 | 0x2_0000);
}

#[test]
fn dds_array_is_slice_major_and_reports_array_size() {
    let case = shape(512, 256, 3, 8, 80, 1, 1, 0);
    let l = layout(&case).unwrap();
    let mut subs = Vec::new();
    for slice in 0..3u8 {
        for mip in 0..8u8 {
            subs.push(vec![
                slice * 16 + mip;
                l.subresource_len(mip as u32).unwrap()
            ]);
        }
    }
    let dds = write_dds(&case, &subs).unwrap();
    assert_eq!(u32_at(&dds, 8), 0x1007 | 0x8_0000 | 0x2_0000);
    assert_eq!(u32_at(&dds, 12), 256);
    assert_eq!(u32_at(&dds, 16), 512);
    assert_eq!(u32_at(&dds, 20), 65_536);
    assert_eq!(u32_at(&dds, 24), 0);
    assert_eq!(u32_at(&dds, 28), 8);
    assert_eq!(u32_at(&dds, 108), 0x1000 | 0x8 | 0x40_0000);
    assert_eq!(u32_at(&dds, 112), 0);
    assert_eq!(u32_at(&dds, 132), 3);
    assert_eq!(dds.len(), 148 + subs.iter().map(Vec::len).sum::<usize>());

    let mut at = 148;
    for (index, sub) in subs.iter().enumerate() {
        assert_eq!(dds[at], sub[0], "subresource {index}");
        at += sub.len();
    }
}

#[test]
fn dds_cubemap_marks_all_faces() {
    let case = shape(128, 128, 6, 8, 72, 2, 18, 0);
    let l = layout(&case).unwrap();
    let mut subs = Vec::new();
    for _face in 0..6 {
        for mip in 0..8 {
            subs.push(vec![mip as u8; l.subresource_len(mip).unwrap()]);
        }
    }
    let dds = write_dds(&case, &subs).unwrap();
    assert_eq!(u32_at(&dds, 8), 0x1007 | 0x8_0000 | 0x2_0000);
    assert_eq!(u32_at(&dds, 20), 8192);
    assert_eq!(u32_at(&dds, 108), 0x1000 | 0x8 | 0x40_0000);
    assert_eq!(u32_at(&dds, 112), 0xFE00);
    assert_eq!(u32_at(&dds, 128), 72);
    assert_eq!(u32_at(&dds, 132), 3);
    assert_eq!(u32_at(&dds, 136), 4);
    assert_eq!(u32_at(&dds, 140), 1);
}

#[test]
fn dds_volume_reports_depth_and_3d_dimension() {
    let case = shape(256, 32, 8, 1, 54, 3, 2, 0);
    let l = layout(&case).unwrap();
    let subs = vec![vec![0x5A; l.subresource_len(0).unwrap()]];
    let dds = write_dds(&case, &subs).unwrap();
    assert_eq!(u32_at(&dds, 8), 0x1007 | 0x8 | 0x80_0000);
    assert_eq!(u32_at(&dds, 12), 32);
    assert_eq!(u32_at(&dds, 16), 256);
    assert_eq!(u32_at(&dds, 20), 512);
    assert_eq!(u32_at(&dds, 24), 8);
    assert_eq!(u32_at(&dds, 28), 1);
    assert_eq!(u32_at(&dds, 108), 0x1000 | 0x8);
    assert_eq!(u32_at(&dds, 112), 0x20_0000);
    assert_eq!(u32_at(&dds, 128), 54);
    assert_eq!(u32_at(&dds, 132), 4);
    assert_eq!(u32_at(&dds, 136), 0);
    assert_eq!(u32_at(&dds, 140), 1);
    assert_eq!(dds.len(), 148 + 131_072);
}

#[test]
fn dds_uncompressed_2d_uses_row_pitch() {
    let case = shape(17, 13, 1, 5, 61, 1, 2, 0);
    let l = layout(&case).unwrap();
    let subs: Vec<Vec<u8>> = (0..5)
        .map(|mip| vec![mip as u8; l.subresource_len(mip).unwrap()])
        .collect();
    let dds = write_dds(&case, &subs).unwrap();
    assert_eq!(u32_at(&dds, 8), 0x1007 | 0x8 | 0x2_0000);
    assert_eq!(u32_at(&dds, 12), 13);
    assert_eq!(u32_at(&dds, 16), 17);
    assert_eq!(u32_at(&dds, 20), 17);
    assert_eq!(u32_at(&dds, 24), 0);
    assert_eq!(u32_at(&dds, 28), 5);
    assert_eq!(u32_at(&dds, 128), 61);
    assert_eq!(u32_at(&dds, 132), 3);
    assert_eq!(dds.len(), 148 + subs.iter().map(Vec::len).sum::<usize>());
}

#[test]
fn dds_rejects_bad_subresource_sets() {
    let case = shape(128, 128, 6, 8, 72, 2, 18, 0);
    let l = layout(&case).unwrap();
    let good = vec![0u8; l.subresource_len(0).unwrap()];
    assert_eq!(
        write_dds(&case, &[]).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let five = vec![good.clone(); 5];
    assert_eq!(
        write_dds(&case, &five).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let seven = vec![good.clone(); 7];
    assert_eq!(
        write_dds(&case, &seven).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let mut wrong = good.clone();
    wrong.push(0);
    let wrong_all = vec![wrong; 48];
    assert_eq!(
        write_dds(&case, &wrong_all).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );

    // 49 vectors imply a partial second cube face, which is not a subresource.
    let partial = vec![good; 49];
    assert_eq!(
        write_dds(&case, &partial).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}
