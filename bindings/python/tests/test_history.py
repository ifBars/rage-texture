"""History policy/snapshot tests for the optional native Python SDK.

The embedded stream is synthetic, reproducible with examples/history_fixture.rs
using bcpack-rs's test encoder. It contains no game data.
"""
import base64
import pathlib
import struct
import tempfile
import unittest
from rage_texture import Store, UnsupportedError, StaleError, BoundsError
from test_binding import make_texture_fixture

PACKED = base64.b64decode("MB8fHx8fHx8CAwMDAwMDAwEHAQEBAQEBAWdIpUhIpUgA7JnfmZnfmQIAoACgoACgAh8fHx8CAwMDAwcHBwdnZ2dn7Ozs7AAAAAACAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgA=")

def document(red=True, provisional=False):
    values=[31,0,0,31,0,0] if red else [0,63,0,0,63,0]
    return {"bc1":{"replace":True,"provisional" if provisional else "confirmed":{"255":values}}}

def make_history_fixture(path):
    make_texture_fixture(path)
    head=bytearray(path.read_bytes()[:4096])
    physical=(4096+len(PACKED)+255)//256*256
    padding=physical-4096-len(PACKED)
    struct.pack_into('<Q',head,24,16|(1<<26)|(1<<30)|(padding<<52))
    struct.pack_into('<Q',head,32,physical//256|(2<<30))
    path.write_bytes(head+PACKED+bytes(padding))

class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.path=pathlib.Path(self.temp.name)/'source.ttpf'
        make_history_fixture(self.path)
    def tearDown(self):self.temp.cleanup()

    def test_confirmed_dictionary_policy_quality_and_zero_cache(self):
        config=document()
        with Store(self.path,history=config,cache_bytes=0) as store:
            texture=store.textures()[0]
            result=store.texture_dds(texture)
            self.assertEqual(result.data[148:150],bytes.fromhex('00f8'))
            self.assertEqual(result.quality[0].kind,'confirmed_history')
            self.assertTrue(result.quality[0].is_confirmed)
            self.assertEqual(result.quality[0].provisional_dependencies,0)
            self.assertEqual(result.quality[0].history_digest,store.history_digest)
            self.assertEqual((store.stats().cache_hits,store.stats().cache_bytes),(0,0))
            config['bc1']['confirmed']['255']=[0,63,0,0,63,0]
            self.assertEqual(store.texture_dds(texture).data,result.data) # owned snapshot
            store.replace_history(config)
            with self.assertRaises(StaleError):store.texture_dds(texture)
            self.assertEqual(store.texture_dds(store.textures()[0]).data[148:150],bytes.fromhex('e007'))

    def test_provisional_is_explicit_unknown_is_always_refused(self):
        with Store(self.path,decode_policy='confirmed',history=document(provisional=True)) as store:
            with self.assertRaises(UnsupportedError):store.read_range_result(4096,4096)
        with Store(self.path,decode_policy='provisional',history=document(provisional=True)) as store:
            result=store.read_range_result(4096,4096)
            self.assertEqual(result.quality[0].kind,'provisional_history')
            self.assertFalse(result.quality[0].is_confirmed)
            self.assertGreater(result.quality[0].provisional_dependencies,0)
        for policy in ('confirmed','provisional'):
            with Store(self.path,decode_policy=policy,history={'bc1':{'replace':True}}) as store:
                with self.assertRaises(UnsupportedError):store.read_range(4096,1)

    def test_replacement_clears_cache_and_failed_replacement_keeps_snapshot(self):
        with Store(self.path,history=document(),cache_bytes=4096) as store:
            texture=store.textures()[0]
            before=store.texture_dds(texture).data
            store.texture_dds(texture)
            self.assertGreater(store.stats().cache_hits,0)
            digest=store.history_digest
            with self.assertRaises(BoundsError):store.replace_history({'bc1':[]})
            self.assertEqual(store.history_digest,digest)
            self.assertEqual(store.texture_dds(texture).data,before)
            store.replace_history(document(False))
            self.assertEqual(store.stats().cache_bytes,0)
            with self.assertRaises(StaleError):store.texture_dds(texture)

    def test_partial_chain_bounds_and_policy_options(self):
        with Store(self.path,history=document()) as store:
            texture=store.textures()[0]
            self.assertEqual(store.texture_dds(texture,mips=1).data[:4],b'DDS ')
            for count in (0,2):
                with self.assertRaises(BoundsError):store.texture_dds(texture,mips=count)
        for options in ({'decode_policy':'typo'},{'decode_policy':'lossless','history':document()},
                        {'allow_fitted':True,'decode_policy':'confirmed'}):
            with self.assertRaises(BoundsError):Store(self.path,**options)

if __name__=='__main__':unittest.main()
