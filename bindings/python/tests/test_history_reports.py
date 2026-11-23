"""Regressions for the real-history validation scripts; synthetic JSON only."""
import importlib.util
import pathlib
import unittest


def report_modules():
    root = pathlib.Path(__file__).resolve().parents[3]
    for name in ('trace_history', 'verify_bptc_real'):
        spec = importlib.util.spec_from_file_location(name, root/'scripts'/f'{name}.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        yield module


class HistoryReportTests(unittest.TestCase):
    def test_combined_documents_use_codec_names_not_integer_ids(self):
        document = {'bc6h': {'endpointClass': {'5': 1}},
                    'bc7': {'historyEndpointBits': {'7': [8]*24}}}
        for module in report_modules():
            with self.subTest(module=module.__name__):
                self.assertEqual(module.section(document, 6), document['bc6h'])
                self.assertEqual(module.section(document, 7), document['bc7'])
                coverage = module.history_coverage(document)
                self.assertEqual(coverage['bc6h_endpoint_class'][5], 1)
                self.assertEqual(coverage['bc7_endpoint_bits'][7], [8]*24)

    def test_single_codec_documents_and_malformed_sections(self):
        single = {'endpointHistory': {'5': [[0]*4 for _ in range(3)]}}
        for module in report_modules():
            with self.subTest(module=module.__name__):
                self.assertEqual(module.section(single, 6), single)
                self.assertEqual(module.section(single, 7), {})
                with self.assertRaises(ValueError):module.section({'bc6h': []}, 6)


if __name__ == '__main__':
    unittest.main()
