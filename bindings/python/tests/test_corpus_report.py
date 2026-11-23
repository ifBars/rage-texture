"""Prevent incomplete coverage or a bad reference from becoming a parity pass."""
import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[3] / "scripts" / "summarize_corpus.py"
SPEC = importlib.util.spec_from_file_location("corpus_report", SCRIPT)
report = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(report)


def fixtures():
    counts = dict(expected=1, attempted=1, pass_=1, fail=0)
    counts["pass"] = counts.pop("pass_")
    rows = [
        dict(kind="run", phase="both", cache_budget=0, stores_total=1),
        dict(kind="store_start", store="synthetic", path="/owned/synthetic.ttpf", chunks=1, textures=1),
        dict(kind="chunk", store="synthetic", chunk=0, codec=8, status="pass", quality={}),
        dict(kind="texture", store="synthetic", texture=0, dxgi=28, layout="texture2d", status="pass", consumer="pass"),
        dict(kind="store_done", store="synthetic", status="pass", chunks=counts,
             textures=counts.copy(), source_changed=False, cache_policy_ok=True,
             consumer_failures=0, stats=dict(cache_hits=0, cache_bytes=0, peak_cache_bytes=0),
             chunk_aggregate=dict(chunks=1, sha256="ab" * 32, bytes=64, failed_indices=[]),
             dds_aggregate=dict(textures=1, bytes=212, failed_textures=[], consumer_failed_textures=[])),
        dict(kind="run_done"),
    ]
    reference = dict(stores_total=1, stores_done=1, stores_failed=0, cache_budget_bytes=0, stores=[
        dict(path="/owned/synthetic.ttpf", status="pass", source_changed=False,
             chunks_expected=1, chunks_attempted=1, chunks_pass=1, chunks_fail=0,
             failure_indices=[], aggregate_sha256="ab" * 32, successful_bytes=64,
             unfitted_blocks=0, invalid_blocks=0, codec_counts={"32": 1}),
    ])
    return rows, reference


class CorpusReportTests(unittest.TestCase):
    def test_source_names_keep_platform_case_semantics(self):
        self.assertNotEqual(report.source_key("/owned/A.ttpf"), report.source_key("/owned/a.ttpf"))
        self.assertEqual(report.source_key("D:\\Owned\\A.ttpf"), report.source_key("d:/owned/a.ttpf"))

    def read(self, rows):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "synthetic.jsonl"
            path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
            return report.read_native(path)

    def test_complete_matching_run_passes(self):
        rows, baseline = fixtures()
        result = report.compare(self.read(rows), baseline)
        self.assertTrue(result["compatibility_passed"])
        self.assertEqual((result["chunks"], result["textures"], result["matching_store_digests"]), (1, 1, 1))

    def test_missing_or_duplicated_chunk_is_not_coverage(self):
        rows, _ = fixtures()
        for damaged in (rows[:2] + rows[3:], rows[:3] + [copy.deepcopy(rows[2])] + rows[3:]):
            with self.subTest(rows=len(damaged)), self.assertRaises(ValueError):
                self.read(damaged)

    def test_missing_completion_is_not_a_pass(self):
        rows, _ = fixtures()
        with self.assertRaises(ValueError):
            self.read(rows[:-1])

    def test_reference_digest_must_match(self):
        rows, baseline = fixtures()
        baseline["stores"][0]["aggregate_sha256"] = "cd" * 32
        result = report.compare(self.read(rows), baseline)
        self.assertFalse(result["compatibility_passed"])

    def test_zero_residency_does_not_hide_peak_cache_use(self):
        rows, _ = fixtures()
        rows[4]["stats"]["peak_cache_bytes"] = 64
        with self.assertRaises(ValueError):
            self.read(rows)

    def test_diagnostics_must_match_even_with_equal_bytes(self):
        rows, baseline = fixtures()
        baseline["stores"][0]["unfitted_blocks"] = 1
        result = report.compare(self.read(rows), baseline)
        self.assertFalse(result["compatibility_passed"])


if __name__ == "__main__":
    unittest.main()
