use rage_texture::{
    DecodePolicy, Store, dds,
    layout::{self, TextureShape},
};

#[test]
fn partial_chains_preserve_source_placement_slice_order_and_zero_cache() {
    for depth in [1, 3, 6] {
        let shape = TextureShape {
            width: 128,
            height: 64,
            depth,
            mips: 5,
            dxgi: 71,
            dimension: if depth == 6 { 2 } else { 1 },
            mode: if depth == 6 {
                18
            } else if depth > 1 {
                1
            } else {
                2
            },
            non_tail: if depth == 6 { 0 } else { 1 },
        };
        // Arrays use raw descriptor mode 1 (the SW_64KB_D tile family).
        let surface = layout::layout(&shape).unwrap();
        let size = surface.total_bytes as usize;
        let mut data = vec![0u8; 4096 + size];
        data[..4].copy_from_slice(b"RDS5");
        data[4..8].copy_from_slice(&3u32.to_le_bytes());
        data[8..16].copy_from_slice(&((4096 + size as u64) | (22u64 << 56)).to_le_bytes());
        data[24..32].copy_from_slice(&(16u64 | (1 << 30)).to_le_bytes());
        data[32..40].copy_from_slice(
            &(((4096 + size) as u64 / 256) | (((4096 + size) as u64 / 4096) << 30)).to_le_bytes(),
        );
        data[256..260].copy_from_slice(b"ETPF");
        data[260..264].copy_from_slice(&634u32.to_le_bytes());
        data[264..268].copy_from_slice(&1u32.to_le_bytes());
        data[268..272].copy_from_slice(&24u32.to_le_bytes());
        let a =
            (shape.width - 1) | ((shape.height - 1) << 14) | ((shape.mips - shape.non_tail) << 28);
        let b = shape.non_tail
            | (shape.dxgi << 3)
            | ((depth - 1) << 10)
            | (shape.dimension << 21)
            | (shape.mode << 23);
        data[272..276].copy_from_slice(&a.to_le_bytes());
        data[276..280].copy_from_slice(&b.to_le_bytes());
        data[280..284].copy_from_slice(&1u32.to_le_bytes());
        for (i, value) in data[4096..].iter_mut().enumerate() {
            *value = ((i * 17 + i / 251) % 256) as u8;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.ttpf");
        std::fs::write(&path, data).unwrap();
        let mut store = Store::open(&path, 0, DecodePolicy::LosslessOnly).unwrap();
        let t = store.textures().unwrap().remove(0);
        for count in [1, 3, 5] {
            let actual = store.texture_dds_mips(&t, count).unwrap();
            let mut subresources = Vec::new();
            for slice in 0..depth {
                for mip in 0..count {
                    subresources.push(store.texture_mip(&t, mip, slice).unwrap().0);
                }
            }
            let expected = dds::write_dds(&shape, &subresources).unwrap();
            assert_eq!(actual.dds, expected, "depth={depth},mips={count}");
            assert_eq!(
                u32::from_le_bytes(actual.dds[28..32].try_into().unwrap()),
                count
            );
            assert_eq!(
                (store.stats().cache_hits, store.stats().cache_bytes),
                (0, 0)
            );
        }
        assert!(store.texture_dds_mips(&t, 0).is_err());
        assert!(store.texture_dds_mips(&t, 6).is_err());
    }
}
