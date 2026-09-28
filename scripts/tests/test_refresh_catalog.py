"""Check that artifact refresh keeps Longhand's recognition policy."""

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "refresh-catalog.py"
SPEC = importlib.util.spec_from_file_location("refresh_catalog", SCRIPT)
refresh = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(refresh)


class RefreshPolicyTests(unittest.TestCase):
    def setUp(self):
        self.handy = {
            "slug": "granite-speech-4.1-2b-plus",
            "id": "handy-computer/granite-speech-4.1-2b-plus-gguf",
            "capabilities": {"timestamps": "segment", "streaming": False},
            "default_quant": "Q5_K_M",
            "files": [{"quant": "Q5_K_M", "filename": "model.gguf", "size_bytes": 1, "sha256": "a" * 64}],
            "languages": ["en"],
            "language_count": 1,
            "license": "apache-2.0",
            "base_model": "ibm-granite/granite-speech-4.1-2b-plus",
            "revision": "b" * 40,
        }
        self.policy = {
            "id": "granite-speech-4.1-2b-plus-q5km",
            "recommended": True,
            "speaker_preferences": {
                "after_stop": ["nemotron", "pyannote"],
                "during_recording": ["nemotron"],
            },
        }

    def test_refresh_retains_exact_artifact_policy(self):
        result = refresh.entry(self.handy, "Granite", "c" * 40, [self.policy])
        self.assertEqual(result["speaker_preferences"], self.policy["speaker_preferences"])
        self.assertTrue(result["recommended"])
        self.assertEqual(result["revision"], self.handy["revision"])
        self.assertEqual(result["sha256"], "a" * 64)

    def test_refresh_rejects_a_different_artifact_policy(self):
        self.policy["id"] = "granite-speech-4.1-2b-plus-q80"
        with self.assertRaisesRegex(SystemExit, "granite-speech-4.1-2b-plus-q5km"):
            refresh.entry(self.handy, "Granite", "c" * 40, [self.policy])

    def test_refresh_rejects_missing_speaker_policy(self):
        del self.policy["speaker_preferences"]
        with self.assertRaisesRegex(SystemExit, "speaker_preferences"):
            refresh.entry(self.handy, "Granite", "c" * 40, [self.policy])


if __name__ == "__main__":
    unittest.main()
