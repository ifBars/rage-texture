"""Adapter contract tests using the Python SDK's existing synthetic fixture."""
import copy
import pathlib
import sys
import tempfile
import time
import types
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'bindings/python/tests'))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from test_binding import make_texture_fixture
from native_dds import NativeDdsBackend
from rage_texture import ClosedError, StaleError, UnsupportedError


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.temp.name)/'source.ttpf'
        make_texture_fixture(self.path)
        self.pool = types.SimpleNamespace(path=str(self.path))
        self.analysis = dict(layout={}, available=[True], descriptor=dict(
            index=0, width=64, height=64, depth=1, mips=1, dimension=1,
            mode=2, non_tail=1, base=4096, meta=0, extra=0, format=71,
            asset_key_at=123))

    def tearDown(self):
        self.temp.cleanup()

    def test_complete_bytes_provenance_and_no_cache(self):
        with NativeDdsBackend(self.path) as backend:
            result = backend.export(self.pool, self.analysis)
            self.assertIsInstance(result.data, bytes)
            self.assertEqual(result.data[:4], b'DDS ')
            self.assertTrue(all(q.is_lossless for q in result.quality))
            self.assertEqual(backend.texture_dds(self.pool, self.analysis), result.data)
            stats = backend.store.stats()
            self.assertEqual((stats.decodes, stats.cache_hits, stats.cache_bytes), (2, 0, 0))
        with self.assertRaises(ClosedError):
            backend.texture_dds(self.pool, self.analysis)

    def test_disagreeing_analysis_and_pool_are_refused(self):
        with NativeDdsBackend(self.path) as backend:
            changed = copy.deepcopy(self.analysis)
            changed['descriptor']['width'] = 32
            with self.assertRaisesRegex(ValueError, 'disagree'):
                backend.export(self.pool, changed)
            changed = copy.deepcopy(self.analysis)
            changed['descriptor']['index'] = -1
            with self.assertRaisesRegex(ValueError, 'index'):
                backend.export(self.pool, changed)
            other = types.SimpleNamespace(path=self.path.with_name('other.ttpf'))
            with self.assertRaisesRegex(ValueError, 'different pool'):
                backend.export(other, self.analysis)

    def test_policy_conflicts_and_blocked_mips_are_explicit(self):
        with NativeDdsBackend(self.path) as backend:
            with self.assertRaises(ValueError):
                backend.export(self.pool,self.analysis,mips=0)
            with self.assertRaises(UnsupportedError):
                backend.export(self.pool,self.analysis,allow_provisional=True)
            self.assertEqual(backend.export(self.pool,self.analysis,mips=1,history={}).data[:4],b'DDS ')
            for changes in (dict(blocked='unsupported layout'), dict(available=[]),
                            dict(available=[False])):
                analysis = dict(self.analysis, **changes)
                with self.assertRaises(UnsupportedError):
                    backend.export(self.pool, analysis)

    def test_updated_input_never_uses_old_catalog_silently(self):
        with NativeDdsBackend(self.path) as backend:
            backend.export(self.pool, self.analysis)
            time.sleep(0.03)
            data = bytearray(self.path.read_bytes())
            data[-1] = 1
            self.path.write_bytes(data)
            with self.assertRaises(StaleError):
                backend.export(self.pool, self.analysis)


if __name__ == '__main__':
    unittest.main()
