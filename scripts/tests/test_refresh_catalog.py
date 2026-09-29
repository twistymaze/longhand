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
            "slug": "parakeet-unified-en-0.6b",
            "id": "handy-computer/parakeet-unified-en-0.6b-gguf",
            "capabilities": {"timestamps": "token", "streaming": True},
            "default_quant": "Q8_0",
            "files": [{"quant": "Q8_0", "filename": "model.gguf", "size_bytes": 1, "sha256": "a" * 64}],
            "languages": ["en"],
            "language_count": 1,
            "license": "other",
            "base_model": "nvidia/parakeet-unified-en-0.6b",
            "revision": "b" * 40,
        }
        self.policy = {
            "id": "parakeet-unified-en-0.6b-q80",
            "recommended": True,
            "speaker_preferences": {
                "after_stop": ["nemotron", "pyannote"],
                "during_recording": ["nemotron"],
            },
        }

    def test_refresh_retains_exact_artifact_policy(self):
        result = refresh.entry(self.handy, "Parakeet Unified", "c" * 40, [self.policy])
        self.assertEqual(result["speaker_preferences"], self.policy["speaker_preferences"])
        self.assertTrue(result["recommended"])
        self.assertEqual(result["revision"], self.handy["revision"])
        self.assertEqual(result["sha256"], "a" * 64)

    def test_refresh_rejects_a_different_artifact_policy(self):
        self.policy["id"] = "parakeet-unified-en-0.6b-q5km"
        with self.assertRaisesRegex(SystemExit, "parakeet-unified-en-0.6b-q80"):
            refresh.entry(self.handy, "Parakeet Unified", "c" * 40, [self.policy])

    def test_refresh_rejects_missing_speaker_policy(self):
        del self.policy["speaker_preferences"]
        with self.assertRaisesRegex(SystemExit, "speaker_preferences"):
            refresh.entry(self.handy, "Parakeet Unified", "c" * 40, [self.policy])


if __name__ == "__main__":
    unittest.main()
