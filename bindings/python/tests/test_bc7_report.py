"""A partial BC7 reference must fail before any game store is opened."""
import copy
import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[3] / "scripts" / "verify_bc7_presets.py"
SPEC = importlib.util.spec_from_file_location("bc7_report", SCRIPT)
report = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(report)


class Bc7ReferenceGateTests(unittest.TestCase):
    def setUp(self):
        self.manifest = [dict(path="nonexistent-synthetic.ttpf", chunks=[
            dict(index=7, start=4096, len=4096, old_sha256="ab" * 32)])]
        self.reference = [dict(path="nonexistent-synthetic.ttpf", index=7,
                               bytes=4096, skipped=0, sha256="ab" * 32)]

    def reject(self, manifest=None, reference=None):
        with self.assertRaises(ValueError):
            report.verify(self.manifest if manifest is None else manifest,
                          self.reference if reference is None else reference, False)

    def test_empty_or_missing_reference(self):
        self.reject(manifest=[])
        self.reject(reference=[])

    def test_duplicate_manifest(self):
        self.manifest.append(copy.deepcopy(self.manifest[0]))
        self.reject()

    def test_duplicate_reference(self):
        self.reference.append(copy.deepcopy(self.reference[0]))
        self.reject()

    def test_unknown_reference(self):
        self.reference[0]["index"] = 8
        self.reject()

    def test_skipped_or_wrong_length(self):
        self.reference[0]["skipped"] = 1
        self.reject()
        self.reference[0].update(skipped=0, bytes=4095)
        self.reject()


if __name__ == "__main__":
    unittest.main()
