"""Synthetic tests for the optional rage-texture Python bindings.

No real game data is used. The fixtures are hand-built RDS5/ETPF containers.
"""

import concurrent.futures
import struct
import tempfile
import time
import unittest
from pathlib import Path

from rage_texture import (
    BoundsError,
    BusyError,
    ClosedError,
    CorruptError,
    Error,
    StaleError,
    Store,
    UnsupportedError,
)


def u32_at(data: bytes, at: int) -> int:
    return struct.unpack_from("<I", data, at)[0]


def make_texture_fixture(path: Path) -> None:
    """One 64x64 BC1 texture, one mip, one raw 4 KiB chunk.

    This mirrors the synthetic fixture used by the parent CLI tests.
    """
    data = bytearray(8192)
    data[0:4] = b"RDS5"
    struct.pack_into("<I", data, 4, 3)  # two table entries plus sentinel
    struct.pack_into("<Q", data, 8, 8192 | (22 << 56))
    # chunk 0: physical [4096, 8192), virtual [4096, 8192), raw codec 0
    struct.pack_into("<Q", data, 24, 16 | (1 << 30))
    struct.pack_into("<Q", data, 32, 32 | (2 << 30))
    # ETPF at 256: version 634, one record, 24-byte area (record + key)
    data[256:260] = b"ETPF"
    struct.pack_into("<I", data, 260, 634)
    struct.pack_into("<I", data, 264, 1)
    struct.pack_into("<I", data, 268, 24)
    # width/height 64, 1 mip, DXGI 71 (BC1), depth 1, dimension 1, mode 2
    struct.pack_into("<I", data, 272, 63 | (63 << 14))
    struct.pack_into("<I", data, 276, 1 | (71 << 3) | (1 << 21) | (2 << 23))
    struct.pack_into("<I", data, 280, 1)  # base page 1 => 4096
    struct.pack_into("<I", data, 284, 0)
    struct.pack_into("<Q", data, 288, 123)
    path.write_bytes(data)


def make_fitted_fixture(path: Path) -> None:
    """A single codec-1 chunk with intentionally unusable BCPack data."""
    data = bytearray(4352)
    data[0:4] = b"RDS5"
    struct.pack_into("<I", data, 4, 3)
    struct.pack_into("<Q", data, 8, 8192 | (22 << 56))
    struct.pack_into("<Q", data, 24, 16 | (1 << 30) | (1 << 26))  # codec 1
    struct.pack_into("<Q", data, 32, 17 | (2 << 30))
    path.write_bytes(data)


class BindingTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.path = self.dir / "source.ttpf"
        make_texture_fixture(self.path)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_textures_and_immutable_descriptor(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            textures = store.textures()
            self.assertEqual(len(textures), 1)
            texture = textures[0]
            self.assertEqual(texture.index, 0)
            self.assertEqual(texture.asset_key, 123)
            self.assertEqual((texture.width, texture.height), (64, 64))
            self.assertEqual(texture.depth, 1)
            self.assertEqual(texture.mips, 1)
            self.assertEqual(texture.dxgi, 71)
            self.assertEqual(texture.dimension, 1)
            self.assertEqual(texture.mode, 2)
            self.assertEqual(texture.base, 4096)
            self.assertIn("Texture(index=0", repr(texture))
            with self.assertRaises(AttributeError):
                texture.width = 1  # type: ignore[misc]

    def test_dds_header_quality_and_zero_cache(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            texture = store.textures()[0]
            export = store.texture_dds(texture)
            data = export.data
            self.assertIsInstance(data, bytes)
            self.assertEqual(data[:4], b"DDS ")
            self.assertEqual(u32_at(data, 4), 124)
            self.assertEqual(u32_at(data, 8), 0x1007 | 0x80000)
            self.assertEqual(u32_at(data, 12), 64)
            self.assertEqual(u32_at(data, 16), 64)
            self.assertEqual(u32_at(data, 20), (64 // 4) * (64 // 4) * 8)
            self.assertEqual(u32_at(data, 24), 0)
            self.assertEqual(u32_at(data, 28), 1)
            self.assertEqual(data[84:88], b"DX10")
            self.assertEqual(u32_at(data, 128), 71)
            self.assertEqual(u32_at(data, 132), 3)
            self.assertEqual(u32_at(data, 136), 0)
            self.assertEqual(u32_at(data, 140), 1)
            self.assertEqual(len(data), 148 + (64 // 4) * (64 // 4) * 8)

            self.assertEqual(export.input_bytes, 4096)
            self.assertGreaterEqual(export.read_decode_seconds, 0.0)
            self.assertGreaterEqual(export.detile_seconds, 0.0)
            self.assertGreaterEqual(export.dds_seconds, 0.0)
            self.assertGreaterEqual(export.total_seconds, 0.0)
            self.assertIn("Export(bytes=", repr(export))

            self.assertEqual(len(export.quality), 1)
            quality = export.quality[0]
            self.assertEqual(quality.kind, "lossless")
            self.assertIsNone(quality.codec)
            self.assertEqual(quality.unfitted_blocks, 0)
            self.assertEqual(quality.invalid_blocks, 0)
            self.assertTrue(quality.is_lossless)
            self.assertIn("Quality(kind=", repr(quality))

            stats = store.stats()
            self.assertEqual(stats.cache_hits, 0)
            self.assertEqual(stats.cache_bytes, 0)
            self.assertEqual(stats.peak_cache_bytes, 0)
            self.assertGreaterEqual(stats.decodes, 1)
            self.assertGreaterEqual(stats.cache_misses, 1)
            self.assertIn("Stats(hits=0", repr(stats))

    def test_mip_and_png_exports(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            texture = store.textures()[0]
            mip = store.texture_mip(texture)
            self.assertEqual(len(mip.data), (64 // 4) * (64 // 4) * 8)
            self.assertEqual(mip.input_bytes, 4096)
            self.assertEqual(mip.quality[0].kind, "lossless")

            png = store.texture_png(texture)
            self.assertTrue(png.data.startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertEqual(png.input_bytes, 4096)
            self.assertGreaterEqual(png.encode_seconds, 0.0)
            self.assertGreaterEqual(png.total_seconds, 0.0)

    def test_read_range_and_cache_zero_counts(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            self.assertEqual(store.read_range(4096, 0), b"")
            self.assertEqual(store.read_range(4096, 4096), bytes(4096))
            detailed = store.read_range_result(4096, 4096)
            self.assertEqual(detailed.data, bytes(4096))
            self.assertEqual(detailed.input_bytes, 4096)
            self.assertGreaterEqual(detailed.read_decode_seconds, 0.0)
            self.assertEqual(len(detailed.quality), 1)
            self.assertEqual(detailed.quality[0].kind, "lossless")
            self.assertEqual(store.read_range(4096, 4096), bytes(4096))
            stats = store.stats()
            self.assertEqual(stats.cache_hits, 0)
            self.assertEqual(stats.cache_bytes, 0)
            self.assertEqual(stats.decodes, 3)
            store.reset()
            reset = store.stats()
            self.assertEqual(
                (
                    reset.cache_hits,
                    reset.cache_misses,
                    reset.decodes,
                    reset.source_bytes,
                    reset.cache_bytes,
                    reset.peak_cache_bytes,
                    reset.evictions,
                ),
                (0, 0, 0, 0, 0, 0, 0),
            )

    def test_bounds_errors_are_explicit(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            with self.assertRaises(BoundsError):
                store.read_range(4096, 4097)
            with self.assertRaises(BoundsError):
                store.read_range(0, 1)
            with self.assertRaises(BoundsError):
                Store(self.path, cache_bytes=-1)
        with self.assertRaises(FileNotFoundError):
            Store(self.dir / "missing.ttpf")

        bad = self.dir / "bad.ttpf"
        bad.write_bytes(b"not an RDS5 container")
        with self.assertRaises(CorruptError):
            Store(bad)

    def test_close_and_context_manager(self) -> None:
        store = Store(self.path, cache_bytes=0)
        self.assertFalse(store.closed)
        store.close()
        self.assertTrue(store.closed)
        store.close()  # idempotent
        with self.assertRaises(ClosedError):
            store.textures()
        with self.assertRaises(ClosedError):
            store.stats()
        with self.assertRaises(ClosedError):
            with store:
                pass

        with Store(self.path, cache_bytes=0) as context:
            self.assertFalse(context.closed)
        self.assertTrue(context.closed)
        with self.assertRaises(ClosedError):
            context.read_range(4096, 1)

    def test_cross_store_descriptor_is_rejected(self) -> None:
        with Store(self.path, cache_bytes=0) as first:
            with Store(self.path, cache_bytes=0) as second:
                texture = first.textures()[0]
                with self.assertRaises(StaleError):
                    second.texture_dds(texture)

    def test_refresh_invalidates_generation(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            old = store.textures()[0]
            store.refresh()
            with self.assertRaises(StaleError):
                store.texture_dds(old)
            fresh = store.textures()[0]
            self.assertEqual(store.texture_dds(fresh).data[:4], b"DDS ")

    def test_source_change_is_stale_until_refresh(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            old = store.textures()[0]
            # Windows may deny path replacement while the core file handle is
            # open, so exercise the equivalent same-file/same-length update.
            time.sleep(0.05)  # coarse filesystem timestamp granularity
            data = bytearray(self.path.read_bytes())
            data[-1] ^= 0xFF
            self.path.write_bytes(data)
            with self.assertRaises(StaleError):
                store.textures()
            store.refresh()
            fresh = store.textures()[0]
            self.assertEqual(store.texture_dds(fresh).data[:4], b"DDS ")
            with self.assertRaises(StaleError):
                store.texture_dds(old)

    def test_fitted_codec_requires_explicit_opt_in(self) -> None:
        fitted = self.dir / "fitted.ttpf"
        make_fitted_fixture(fitted)
        with Store(fitted, cache_bytes=0, allow_fitted=False) as store:
            with self.assertRaises(UnsupportedError):
                store.read_range(4096, 1)

        with Store(fitted, cache_bytes=0, allow_fitted=True) as store:
            try:
                store.read_range(4096, 1)
            except CorruptError:
                pass  # expected for the deliberately unusable entropy payload

    def test_same_store_concurrent_calls_fail_or_serialize(self) -> None:
        with Store(self.path, cache_bytes=0) as store:
            def read() -> object:
                try:
                    return store.read_range(4096, 4096)
                except BusyError:
                    return "busy"
                except Error as exc:  # no non-busy binding error is acceptable
                    return exc

            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                results = list(pool.map(lambda _: read(), range(8)))
            self.assertTrue(any(result == bytes(4096) for result in results))
            for result in results:
                self.assertIn(result, (bytes(4096), "busy"))


if __name__ == "__main__":
    unittest.main()
