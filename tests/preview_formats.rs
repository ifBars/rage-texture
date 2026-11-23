//! Exact synthetic expectations for typed PNG previews.
use rage_texture::preview::{rgba8, snorm, supports};
use std::io;

fn bc4(indexes: [u8; 16]) -> [u8; 8] {
    let mut indices = 0u64;
    for (pixel, index) in indexes.into_iter().enumerate() {
        indices |= u64::from(index) << (3 * pixel);
    }
    let mut block = [0u8; 8];
    block[2..].copy_from_slice(&indices.to_le_bytes()[..6]);
    block
}

fn bc5(x: [u8; 8], y: [u8; 8]) -> [u8; 16] {
    let mut block = [0u8; 16];
    block[..8].copy_from_slice(&x);
    block[8..].copy_from_slice(&y);
    block
}

fn pixel(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let at = (y * width + x) * 4;
    rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn snorm_preview_mapping_is_explicit() {
    assert_eq!(snorm(f32::NEG_INFINITY), 0);
    assert_eq!(snorm(-1.0), 0);
    assert_eq!(snorm(-0.5), 64);
    assert_eq!(snorm(-0.0), 128);
    assert_eq!(snorm(0.0), 128);
    assert_eq!(snorm(0.5), 191);
    assert_eq!(snorm(1.0), 255);
    assert_eq!(snorm(f32::INFINITY), 255);
    assert_eq!(snorm(f32::NAN), 0);
}

#[test]
fn bc4_snorm_six_value_mode_and_signed_mapping() {
    // red_0 = +127 (+1), red_1 = -127 (-1): six interpolated values.
    let mut block = bc4([0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0]);
    block[0] = 0x7f;
    block[1] = 0x81;
    let rgba = rgba8(81, 4, 4, &block).unwrap();
    let expected = [
        [255, 255, 255, 255],
        [0, 0, 0, 255],
        [219, 219, 219, 255],
        [182, 182, 182, 255],
        [146, 146, 146, 255],
        [109, 109, 109, 255],
        [73, 73, 73, 255],
        [36, 36, 36, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    for (i, expected) in expected.into_iter().enumerate() {
        assert_eq!(pixel(&rgba, 4, i % 4, i / 4), expected, "pixel {i}");
    }
    // Pixels whose 3-bit index remains zero are the +1 endpoint.
    assert_eq!(pixel(&rgba, 4, 0, 2), [255, 255, 255, 255]);
}

#[test]
fn bc4_snorm_clamps_minus_128_before_selecting_four_values() {
    // red_0 = -127, red_1 = -128. -128 clamps to -127, so the endpoints are
    // equal and the four-value table applies. An unclamped comparison would
    // select six values and make index 7 opaque black instead of white.
    let mut block = bc4([6, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    block[0] = 0x81;
    block[1] = 0x80;
    let rgba = rgba8(81, 4, 4, &block).unwrap();
    assert_eq!(pixel(&rgba, 4, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&rgba, 4, 1, 0), [255, 255, 255, 255]);
    assert_eq!(pixel(&rgba, 4, 2, 0), [0, 0, 0, 255]);
}

#[test]
fn bc5_snorm_preserves_x_and_y_channels_and_crops_edges() {
    // X endpoint 0 (index 1) maps to the midpoint; Y endpoint +127 maps to
    // positive one. The first eight bytes feed red, the second eight green.
    let indices: [u8; 16] = [1; 16];
    let mut x = bc4(indices);
    x[0] = 0;
    x[1] = 0;
    let mut y = bc4(indices);
    y[0] = 0x7f;
    y[1] = 0x7f;
    let block = bc5(x, y);
    // A 2x2 texture still stores one whole 4x4 block; the padded texels are
    // dropped instead of leaking into the preview.
    let rgba = rgba8(84, 2, 2, &block).unwrap();
    assert_eq!(rgba.len(), 2 * 2 * 4);
    assert_eq!(pixel(&rgba, 2, 0, 0), [128, 255, 0, 255]);
    assert_eq!(pixel(&rgba, 2, 1, 0), [128, 255, 0, 255]);
    assert_eq!(pixel(&rgba, 2, 0, 1), [128, 255, 0, 255]);
}

#[test]
fn bc5_snorm_uses_the_same_six_value_table_per_channel() {
    let mut x = bc4([2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2]);
    x[0] = 0x7f;
    x[1] = 0x81;
    let y = bc4([0; 16]);
    let rgba = rgba8(84, 4, 4, &bc5(x, y)).unwrap();
    assert_eq!(pixel(&rgba, 4, 0, 0), [219, 128, 0, 255]);
}

#[test]
fn signed_block_lengths_are_checked() {
    assert_eq!(
        rgba8(81, 4, 4, &[0; 7]).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        rgba8(84, 4, 4, &[0; 15]).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn r10g10b10a2_unorm_is_typed() {
    let texel = (1023u32 | (512 << 10) | (1 << 20) | (2 << 30)).to_le_bytes();
    assert_eq!(rgba8(24, 1, 1, &texel).unwrap(), [255, 128, 0, 170]);
    assert!(supports(24));
}

#[test]
fn r11g11b10_float_uses_shared_preview_clamp() {
    // R = 1.0 (exponent 15, fraction 0), G = 0.5, B = 0.25.
    let code = (15u32 << 6) | ((14u32 << 6) << 11) | ((13u32 << 5) << 22);
    assert_eq!(
        rgba8(26, 1, 1, &code.to_le_bytes()).unwrap(),
        [255, 128, 64, 255]
    );
    // Infinity clamps to white and a NaN fraction maps to zero.
    let inf = (31u32 << 6).to_le_bytes();
    assert_eq!(rgba8(26, 1, 1, &inf).unwrap()[0], 255);
    let nan = ((31u32 << 6) | 1).to_le_bytes();
    assert_eq!(rgba8(26, 1, 1, &nan).unwrap()[0], 0);
}

#[test]
fn bgrx8_ignores_the_padding_byte() {
    assert_eq!(
        rgba8(88, 1, 1, &[10, 20, 30, 99]).unwrap(),
        [30, 20, 10, 255]
    );
    assert_eq!(
        rgba8(93, 1, 1, &[10, 20, 30, 0]).unwrap(),
        [30, 20, 10, 255]
    );
}

#[test]
fn snorm_uncompressed_variants_are_typed() {
    let rgba16 = [0x00, 0x80, 0x00, 0xc0, 0x00, 0x00, 0xff, 0x7f];
    assert_eq!(rgba8(13, 1, 1, &rgba16).unwrap(), [0, 64, 128, 255]);
    assert_eq!(
        rgba8(31, 1, 1, &[0x80, 0xc0, 0x00, 0x7f]).unwrap(),
        [0, 63, 128, 255]
    );
    assert_eq!(
        rgba8(37, 1, 1, &[0xff, 0x7f, 0x00, 0x80]).unwrap(),
        [255, 0, 0, 255]
    );
    assert_eq!(rgba8(51, 1, 1, &[0x7f, 0x80]).unwrap(), [255, 0, 0, 255]);
    assert_eq!(
        rgba8(58, 1, 1, &0x8000u16.to_le_bytes()).unwrap(),
        [0, 0, 0, 255]
    );
    assert_eq!(rgba8(63, 1, 1, &[0x7f]).unwrap(), [255, 255, 255, 255]);
}

#[test]
fn float_uncompressed_variants_are_typed() {
    let rgba32 = [
        &0.5f32.to_le_bytes()[..],
        &(-1.0f32).to_le_bytes()[..],
        &2.0f32.to_le_bytes()[..],
        &f32::NAN.to_le_bytes()[..],
    ]
    .concat();
    assert_eq!(rgba8(2, 1, 1, &rgba32).unwrap(), [128, 0, 255, 0]);
    let rg32 = [&0.25f32.to_le_bytes()[..], &1.0f32.to_le_bytes()[..]].concat();
    assert_eq!(rgba8(16, 1, 1, &rg32).unwrap(), [64, 255, 0, 255]);
    assert_eq!(
        rgba8(41, 1, 1, &0.5f32.to_le_bytes()).unwrap(),
        [128, 128, 128, 255]
    );
    assert_eq!(
        rgba8(34, 1, 1, &[0x00, 0x3c, 0x00, 0xc0]).unwrap(),
        [255, 0, 0, 255]
    );
}

#[test]
fn typeless_formats_never_inherit_a_unorm_preview() {
    for format in [19, 23, 27, 33, 39, 44, 48, 53, 60, 90, 92] {
        assert!(!supports(format), "DXGI {format} claimed");
        assert_eq!(
            rgba8(format, 4, 4, &[0; 128]).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
