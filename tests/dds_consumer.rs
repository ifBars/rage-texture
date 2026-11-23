//! Container-level DDS interoperability checks with `ddsfile`.
//!
//! The validator lives in `examples/support/dds_consumer.rs`. These tests
//! exercise every format observed in the retail stores, all four surface
//! families, and negative controls for truncation and wrong headers.
//!
//! The synthetic payloads are generated through the SDK writer so that the
//! test exercises the real output path. Pixel decoding is checked separately
//! by `scripts/verify_dds_consumers.py` with DirectXTex; a pass here means the
//! container is coherent, not that the pixels are correct.

#[path = "../examples/support/dds_consumer.rs"]
mod dds_consumer;

use rage_texture::{
    dds,
    layout::{self, SurfaceKind, TextureShape},
};

/// Formats observed in the retail stores (see `FORMAT_GAPS.md`).
const REAL_FORMATS: [u32; 20] = [
    10, 11, 28, 29, 54, 61, 65, 71, 72, 74, 77, 78, 80, 83, 87, 91, 95, 96, 98, 99,
];

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

fn subresources(shape: &TextureShape, mips: u32) -> Vec<Vec<u8>> {
    let surface = layout::layout(shape).unwrap();
    let count = surface.subresource_count().unwrap();
    let volume = surface.kind == SurfaceKind::Volume3D;
    (0..count)
        .map(|i| {
            let mip = if volume { i } else { i % mips as usize };
            let bytes = surface.subresource_len(mip as u32).unwrap();
            (0..bytes).map(|j| ((i * 31 + j * 7) % 256) as u8).collect()
        })
        .collect()
}

fn write_and_validate(shape: &TextureShape, mips: u32) -> Vec<u8> {
    let parts = subresources(shape, mips);
    let dds = dds::write_dds(shape, &parts).unwrap();
    dds_consumer::validate(&dds, shape).unwrap();
    dds
}

#[test]
fn two_dimensional_full_chains_for_all_real_formats() {
    for dxgi in REAL_FORMATS {
        // Non-square mips catch stride arithmetic that divides both axes by 4.
        let shape = shape(64, 32, 1, 4, dxgi, 1, 2, 0);
        let bytes = write_and_validate(&shape, 4);
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            4,
            "dxgi {dxgi}"
        );
    }
}

#[test]
fn arrays_for_all_real_formats() {
    for dxgi in REAL_FORMATS {
        let shape = shape(64, 32, 3, 3, dxgi, 1, 1, 0);
        let bytes = write_and_validate(&shape, 3);
        assert_eq!(
            u32::from_le_bytes(bytes[140..144].try_into().unwrap()),
            3,
            "dxgi {dxgi}"
        );
    }
}

#[test]
fn cubes_for_all_real_formats() {
    for dxgi in REAL_FORMATS {
        // Cubemap faces are square; mode 18 is non_tail 0.
        let shape = shape(32, 32, 6, 4, dxgi, 2, 18, 0);
        let bytes = write_and_validate(&shape, 4);
        assert_eq!(
            u32::from_le_bytes(bytes[136..140].try_into().unwrap()),
            0x4,
            "dxgi {dxgi}"
        );
        assert_eq!(
            u32::from_le_bytes(bytes[140..144].try_into().unwrap()),
            1,
            "dxgi {dxgi}"
        );
    }
}

#[test]
fn volumes_for_all_real_uncompressed_formats() {
    let mut checked = 0;
    for dxgi in REAL_FORMATS {
        let info = layout::format_info(dxgi).unwrap();
        if info.block != 1 {
            // Explicit failure instead of a disguised 2D array.
            let bad = shape(32, 32, 8, 1, dxgi, 3, 2, 0);
            let parts = vec![vec![0u8; 4096]];
            let error = dds::write_dds(&bad, &parts).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::Unsupported, "dxgi {dxgi}");
            continue;
        }
        let shape = shape(64, 32, 8, 1, dxgi, 3, 2, 0);
        let bytes = write_and_validate(&shape, 1);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            8,
            "dxgi {dxgi}"
        );
        assert_eq!(
            u32::from_le_bytes(bytes[132..136].try_into().unwrap()),
            4,
            "dxgi {dxgi}"
        );
        checked += 1;
    }
    assert_eq!(checked, 9);
}

#[test]
fn partial_leading_chains_validate_against_the_written_shape() {
    let full = shape(64, 32, 1, 4, 71, 1, 2, 0);
    let partial = shape(64, 32, 1, 2, 71, 1, 2, 0);
    let parts = subresources(&full, 4);
    let dds = dds::write_dds(&partial, &parts[..2]).unwrap();
    dds_consumer::validate(&dds, &partial).unwrap();
    // The same file must not pass as the four-mip shape.
    assert!(dds_consumer::validate(&dds, &full).is_err());
}

fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn valid_two_mip_rgba8() -> (TextureShape, Vec<u8>) {
    let shape = shape(16, 16, 1, 2, 28, 1, 2, 0);
    let parts = subresources(&shape, 2);
    let bytes = dds::write_dds(&shape, &parts).unwrap();
    dds_consumer::validate(&bytes, &shape).unwrap();
    (shape, bytes)
}

#[test]
fn negative_controls_reject_bad_containers() {
    let (shape, good) = valid_two_mip_rgba8();

    let mut truncated = good.clone();
    truncated.pop();
    assert!(dds_consumer::validate(&truncated, &shape).is_err());

    let mut trailing = good.clone();
    trailing.push(0);
    assert!(dds_consumer::validate(&trailing, &shape).is_err());

    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(dds_consumer::validate(&magic, &shape).is_err());

    let mut no_dx10 = good.clone();
    no_dx10[84..88].copy_from_slice(b"DXT1");
    assert!(dds_consumer::validate(&no_dx10, &shape).is_err());

    let mut wrong_format = good.clone();
    set_u32(&mut wrong_format, 128, 29);
    assert!(dds_consumer::validate(&wrong_format, &shape).is_err());

    let mut wrong_width = good.clone();
    set_u32(&mut wrong_width, 16, 17);
    assert!(dds_consumer::validate(&wrong_width, &shape).is_err());

    let mut wrong_height = good.clone();
    set_u32(&mut wrong_height, 12, 17);
    assert!(dds_consumer::validate(&wrong_height, &shape).is_err());

    let mut wrong_mips = good.clone();
    set_u32(
        &mut wrong_mips,
        8,
        u32::from_le_bytes(good[8..12].try_into().unwrap()) | 0x2_0000,
    );
    set_u32(&mut wrong_mips, 28, 3);
    assert!(dds_consumer::validate(&wrong_mips, &shape).is_err());

    let mut wrong_array = good.clone();
    set_u32(&mut wrong_array, 140, 2);
    assert!(dds_consumer::validate(&wrong_array, &shape).is_err());

    let mut wrong_pitch = good.clone();
    set_u32(&mut wrong_pitch, 20, 16 * 5);
    assert!(dds_consumer::validate(&wrong_pitch, &shape).is_err());

    assert!(dds_consumer::validate(&[], &shape).is_err());
}

#[test]
fn negative_controls_reject_wrong_cube_and_volume_flags() {
    let cube = shape(16, 16, 6, 2, 71, 2, 18, 0);
    let cube_dds = write_and_validate(&cube, 2);
    dds_consumer::validate(&cube_dds, &cube).unwrap();

    let mut not_cube = cube_dds.clone();
    set_u32(&mut not_cube, 136, 0);
    assert!(dds_consumer::validate(&not_cube, &cube).is_err());

    let mut missing_face = cube_dds.clone();
    let caps2 = u32::from_le_bytes(cube_dds[112..116].try_into().unwrap());
    set_u32(&mut missing_face, 112, caps2 & !0x400);
    assert!(dds_consumer::validate(&missing_face, &cube).is_err());

    let volume = shape(32, 32, 8, 1, 54, 3, 2, 0);
    let volume_dds = write_and_validate(&volume, 1);
    dds_consumer::validate(&volume_dds, &volume).unwrap();

    let mut wrong_depth = volume_dds.clone();
    set_u32(&mut wrong_depth, 24, 9);
    assert!(dds_consumer::validate(&wrong_depth, &volume).is_err());

    let mut wrong_dimension = volume_dds.clone();
    set_u32(&mut wrong_dimension, 132, 3);
    assert!(dds_consumer::validate(&wrong_dimension, &volume).is_err());

    let mut no_volume_flag = volume_dds.clone();
    let caps2 = u32::from_le_bytes(volume_dds[112..116].try_into().unwrap());
    set_u32(&mut no_volume_flag, 112, caps2 & !0x20_0000);
    assert!(dds_consumer::validate(&no_volume_flag, &volume).is_err());
}
