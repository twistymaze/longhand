"""Check that artifact refresh reads Handy schema 2 and keeps Longhand's policy."""

import copy
import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "refresh-catalog.py"
SPEC = importlib.util.spec_from_file_location("refresh_catalog", SCRIPT)
refresh = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(refresh)


ASSESSMENT = {
    "source": "https://github.com/cjpais/Handy/blob/" + "c" * 40 + "/src-tauri/src/catalog/catalog.json",
    "source_revision": "c" * 40,
    "speed_score": 79,
    "asr_accuracy_score": 90,
    "speaker_accuracy_score": None,
    "benchmark_context": None,
}


class RefreshTests(unittest.TestCase):
    def setUp(self):
        self.handy = {
            "id": "parakeet-unified-en-0.6b",
            "repository": "handy-computer/parakeet-unified-en-0.6b-gguf",
            "revision": "b" * 40,
            "license": "cc-by-4.0",
            "family": "parakeet",
            "languages": ["en"],
            "capabilities": {"streaming": True, "timestamps": True},
            "quant": "Q8_0",
            "role": "asr",
            "files": {
                "Q8_0": {"filename": "model-Q8_0.gguf", "size": 1, "sha256": "a" * 64},
                "Q5_K_M": {"filename": "model-Q5_K_M.gguf", "size": 2, "sha256": "d" * 64},
            },
        }
        self.checked_in = {
            "id": "parakeet-unified-en-0.6b-q80",
            "timestamps": "token",
            "recommended": True,
            "speaker_preferences": {
                "after_stop": ["nemotron", "pyannote"],
                "during_recording": ["nemotron"],
            },
            "assessment": copy.deepcopy(ASSESSMENT),
        }

    def entry(self):
        return refresh.entry(self.handy, [self.checked_in])

    def test_entry_combines_handy_artifact_and_checked_in_values(self):
        result = self.entry()
        self.assertEqual(result["id"], "parakeet-unified-en-0.6b-q80")
        self.assertEqual(result["name"], "Parakeet Unified EN 0.6B")
        self.assertEqual(result["revision"], "b" * 40)
        self.assertEqual(result["filename"], "model-Q8_0.gguf")
        self.assertEqual(result["size_bytes"], 1)
        self.assertEqual(result["sha256"], "a" * 64)
        self.assertEqual(
            result["url"],
            "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/"
            + "b" * 40
            + "/model-Q8_0.gguf",
        )
        self.assertEqual(result["timestamps"], "token")
        self.assertTrue(result["recommended"])
        self.assertEqual(result["speaker_preferences"], self.checked_in["speaker_preferences"])
        self.assertEqual(result["assessment"], ASSESSMENT)
        self.assertEqual(result["description"], "English, token-level timestamps, streaming.")

    def test_parakeet_unified_keeps_the_model_card_licence(self):
        licence = self.entry()["licence"]
        self.assertEqual(licence["name"], "NVIDIA Open Model License")
        self.assertEqual(
            licence["attribution"],
            "GGUF conversion by handy-computer of nvidia/parakeet-unified-en-0.6b"
            " (NVIDIA, NVIDIA Open Model License).",
        )

    def test_region_tags_become_primary_codes_in_order(self):
        tags = ["en-US", "en-GB", "es-US", "es-ES", "ar-AR", "nb-NO", "zh-CN", "de"]
        self.assertEqual(refresh.language_codes(tags), ["en", "es", "ar", "nb", "zh", "de"])

    def test_handy_without_timestamps_fails(self):
        self.handy["capabilities"]["timestamps"] = False
        with self.assertRaisesRegex(SystemExit, "timestamps"):
            self.entry()

    def test_checked_in_entry_without_timestamp_level_fails(self):
        self.checked_in["timestamps"] = "frame"
        with self.assertRaisesRegex(SystemExit, "timestamp level"):
            self.entry()

    def test_missing_default_quant_file_fails(self):
        del self.handy["files"]["Q8_0"]
        with self.assertRaisesRegex(SystemExit, "Q8_0"):
            self.entry()

    def test_file_in_a_folder_fails(self):
        self.handy["files"]["Q8_0"]["filename"] = "sub/model-Q8_0.gguf"
        with self.assertRaisesRegex(SystemExit, "plain"):
            self.entry()

    def test_other_repository_fails(self):
        self.handy["repository"] = "someone/parakeet-unified-en-0.6b-gguf"
        with self.assertRaisesRegex(SystemExit, "repository"):
            self.entry()

    def test_missing_assessment_fails(self):
        del self.checked_in["assessment"]
        with self.assertRaisesRegex(SystemExit, "assessment"):
            self.entry()

    def test_checked_in_entry_for_another_artifact_fails(self):
        self.checked_in["id"] = "parakeet-unified-en-0.6b-q5km"
        with self.assertRaisesRegex(SystemExit, "parakeet-unified-en-0.6b-q80"):
            self.entry()

    def test_missing_speaker_preferences_fails(self):
        del self.checked_in["speaker_preferences"]
        with self.assertRaisesRegex(SystemExit, "speaker_preferences"):
            self.entry()

    def test_other_schema_fails(self):
        with self.assertRaisesRegex(SystemExit, "schema 2"):
            refresh.models_by_id({"schema": 1, "models": []}, "catalog.json")


if __name__ == "__main__":
    unittest.main()
