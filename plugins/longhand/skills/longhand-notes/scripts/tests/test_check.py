"""check.py snapshot and compare on a copy of the test storage folder."""

import json
import os
import shutil

from tests import support
from tests.support import BROKEN, BUDGET, DESIGN, HIRING, IDEA, NEWER, RETRO, STANDUP, VENDOR

PASS, PROBLEM, NOT_CHECKED = 0, 1, 2

BUDGET_SESSION = "---\ntitle: Budget review\nlonghand_tags:\n- Finance\n- Team::Planning\n---\n"


class CompareTests(support.StorageFolderTest):
    def test_no_change_passes(self):
        snapshot = self.snapshot()
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("Changes: none", out)

    def test_a_tag_edit_passes_and_names_each_changed_note(self):
        snapshot = self.snapshot()
        self.assertEqual(self.read(BUDGET, "session.md"), BUDGET_SESSION)
        self.write(BUDGET_SESSION.replace("- Team::Planning\n", "- Team::Planning\n- Person::Nathan\n"),
                   BUDGET, "session.md")
        # A flow list in another YAML style, with a comment and a key that
        # Longhand does not know.
        self.write("---\n# Tags from the agent\ntitle: 'Quick idea'\nlonghand_tags: [Ideas, \"Person::Nathan\"]\n"
                   "rating: 3\n---\n", IDEA, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("changed: %s: session.md" % BUDGET, out)
        self.assertIn("changed: %s: session.md" % IDEA, out)

    def test_crlf_line_ends_pass(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("\n", "\r\n").replace("Finance", "Finance\r\n- Travel"),
                   BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)

    def test_edits_of_notes_transcript_and_summary_pass(self):
        snapshot = self.snapshot()
        for name in ("notes.md", "transcript.md", "summary.md"):
            self.write(self.read(BUDGET, name) + "\nAn edit.\n", BUDGET, name)
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("notes.md", out)
        self.assertIn("summary.md", out)
        self.assertIn("transcript.md", out)

    def test_an_edit_of_a_protected_file_is_a_problem(self):
        for name in ("longhand.json", "recognition.jsonl"):
            with self.subTest(name=name):
                snapshot = self.snapshot()
                original = self.read(BUDGET, name)
                self.write(original.replace("generation-1", "generation-2").replace('"found_speakers": 2',
                                                                                    '"found_speakers": 3'),
                           BUDGET, name)
                code, out, _ = self.compare(snapshot)
                self.assertEqual(code, PROBLEM, out)
                self.assertIn("%s: %s changed" % (BUDGET, name), out)
                self.write(original, BUDGET, name)

    def test_an_edit_of_audio_is_a_problem(self):
        snapshot = self.snapshot()
        with open(self.path(BUDGET, "audio/segment-01.opus"), "ab") as handle:
            handle.write(b"more")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("audio/segment-01.opus changed", out)

    def test_an_edit_of_a_busy_note_is_a_problem(self):
        for note in (DESIGN, STANDUP, VENDOR):
            with self.subTest(note=note):
                snapshot = self.snapshot()
                original = self.read(note, "notes.md")
                self.write(original + "- An edit\n", note, "notes.md")
                code, out, _ = self.compare(snapshot)
                self.assertEqual(code, PROBLEM, out)
                self.assertIn("%s: the note was changed while it was busy" % note, out)
                self.write(original, note, "notes.md")

    def test_a_note_that_becomes_busy_during_the_edit_is_a_problem(self):
        snapshot = self.snapshot()
        self.write(self.read(DESIGN, support.BUSY_MARKER), BUDGET, support.BUSY_MARKER)
        self.write(self.read(BUDGET, "notes.md") + "- An edit\n", BUDGET, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("in-use marker", out)

    def test_a_stale_in_use_marker_does_not_make_a_note_busy(self):
        stale = {"run_id": "old", "refreshed_at": "2026-10-03T10:05:00-05:00"}
        self.write(json.dumps(stale), BUDGET, ".longhand-in-use-old")
        snapshot = self.snapshot()
        self.write(self.read(BUDGET, "notes.md") + "- An edit\n", BUDGET, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)

    def test_a_frontmatter_that_cannot_be_read_is_not_checked(self):
        cases = [
            "---\ntitle: Budget review\nlonghand_tags: [Finance, Team\n---\n",
            "---\ntitle: |\n  Budget review\n---\n",
            "---\ntitle: Budget review\nlonghand_tags:\n  nested: value\n---\n",
            "title: Budget review\n",
            "---\ntitle: 2026\n---\n",
        ]
        for text in cases:
            with self.subTest(text=text):
                snapshot = self.snapshot()
                self.write(text, BUDGET, "session.md")
                code, out, _ = self.compare(snapshot)
                self.assertEqual(code, NOT_CHECKED, out)
                self.assertIn("Not checked:", out)
                self.assertIn("session.md", out)
                self.write(BUDGET_SESSION, BUDGET, "session.md")

    def test_an_edit_of_a_note_with_an_unknown_layout_is_not_checked(self):
        snapshot = self.snapshot()
        self.write(self.read(NEWER, "notes.md") + "- An edit\n", NEWER, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertIn("layout 2", out)

    def test_an_edit_of_a_note_with_an_unknown_record_key_is_not_checked(self):
        record = json.loads(self.read(BUDGET, "longhand.json"))
        record["future_record"] = True
        self.write(json.dumps(record, indent=2, sort_keys=True) + "\n", BUDGET, "longhand.json")
        snapshot = self.snapshot()
        self.write(self.read(BUDGET, "notes.md") + "- An edit\n", BUDGET, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertIn("future_record", out)

    def test_an_edit_of_a_note_that_waits_for_its_records_is_not_checked(self):
        snapshot = self.snapshot()
        self.write(self.read(RETRO, "notes.md") + "- An edit\n", RETRO, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)

    def test_unchanged_notes_that_cannot_be_read_do_not_fail_a_check(self):
        snapshot = self.snapshot()
        self.write(self.read(BUDGET, "notes.md") + "- An edit\n", BUDGET, "notes.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertNotIn(BROKEN, out)

    def test_a_title_change_with_a_folder_rename_reads_as_moved(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("title: Budget review", "title: 'Budget: Q4 review'"), BUDGET, "session.md")
        os.rename(self.path(BUDGET), self.path("2026-10-01 0900 Budget Q4 review"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("moved: %s -> 2026-10-01 0900 Budget Q4 review" % BUDGET, out)
        self.assertNotIn("gone", out)

    def test_a_title_change_without_the_folder_rename_is_a_problem(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("title: Budget review", "title: Budget plan"), BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("2026-10-01 0900 Budget plan", out)

    def test_a_rename_to_a_name_that_does_not_follow_the_title_is_a_problem(self):
        snapshot = self.snapshot()
        os.rename(self.path(BUDGET), self.path("Budget"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("does not follow the title", out)

    def test_a_type_move_passes(self):
        snapshot = self.snapshot()
        shutil.move(self.path(BUDGET), self.path("Meetings", BUDGET))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("moved: %s -> Meetings/%s" % (BUDGET, BUDGET), out)

    def test_a_move_to_a_type_folder_with_a_name_that_is_not_valid_is_a_problem(self):
        snapshot = self.snapshot()
        os.makedirs(self.path("General"))
        shutil.move(self.path(BUDGET), self.path("General", BUDGET))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("General", out)

    def test_a_deleted_note_is_a_problem(self):
        snapshot = self.snapshot()
        shutil.rmtree(self.path(IDEA))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("%s: the note folder is gone" % IDEA, out)

    def test_a_copied_note_folder_is_a_problem(self):
        snapshot = self.snapshot()
        shutil.copytree(self.path(IDEA), self.path("2026-10-07 1300 Quick idea-2"), copy_function=shutil.copyfile)
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("one session_id", out)

    def test_a_new_file_in_a_note_folder_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("scratch\n", BUDGET, "draft.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("draft.md is a new file", out)

    def test_a_tag_that_is_not_valid_is_a_problem(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("- Finance\n", "- ''\n"), BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("a tag cannot be empty", out)

    def test_two_tags_with_one_key_are_a_warning(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("- Finance\n", "- Finance\n- ' finance '\n"), BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("one tag key", out)

    def test_the_library_marker_is_protected(self):
        snapshot = self.snapshot()
        self.write("x", ".longhand/library")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn(".longhand/library changed", out)


class SettingsTests(support.StorageFolderTest):
    def settings(self):
        return json.loads(self.read(".longhand/settings.json"))

    def write_settings(self, value):
        self.write(json.dumps(value, indent=2, sort_keys=True) + "\n", ".longhand/settings.json")

    def test_a_valid_settings_edit_passes(self):
        snapshot = self.snapshot()
        value = self.settings()
        value["show_recording_widget"] = False
        value["audio_removal"] = "off"
        value["note_types"] = ["Meetings", "One on One"]
        value["default_note_type"] = "one on one"
        value["note_type_prompts"]["One on One"] = "play-by-play"
        self.write_settings(value)
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("show_recording_widget", out)

    def test_settings_values_that_are_not_valid_are_problems(self):
        cases = [
            ("show_recording_widget", "no"),
            ("audio_removal", 0),
            ("speaker_source", "nobody"),
            ("active_summary_prompt", "a-prompt-that-is-not-there"),
            ("note_types", ["Meetings", "meetings"]),
            ("note_types", ["General"]),
            ("default_note_type", "Nothing"),
            ("default_speaker_count", {"kind": "exact", "count": 0}),
            ("note_type_prompts", {"Meetings": "a-prompt-that-is-not-there"}),
        ]
        for key, bad in cases:
            with self.subTest(key=key, value=bad):
                original = self.read(".longhand/settings.json")
                snapshot = self.snapshot()
                value = self.settings()
                value[key] = bad
                self.write_settings(value)
                code, out, _ = self.compare(snapshot)
                self.assertEqual(code, PROBLEM, out)
                self.assertIn(key, out)
                self.write(original, ".longhand/settings.json")

    def test_a_settings_file_that_does_not_parse_is_a_problem(self):
        snapshot = self.snapshot()
        self.write(self.read(".longhand/settings.json")[:-10], ".longhand/settings.json")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("does not parse", out)

    def test_a_removed_key_is_a_problem(self):
        snapshot = self.snapshot()
        value = self.settings()
        del value["live_transcript"]
        self.write_settings(value)
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)

    def test_a_key_that_the_skill_does_not_know_is_not_checked(self):
        snapshot = self.snapshot()
        value = self.settings()
        value["future_setting"] = 1
        self.write_settings(value)
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertIn("future_setting", out)

    def test_a_background_edit_passes_and_a_long_background_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("I am Erv. I work on Longhand.\n", ".longhand/background.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.write("x" * 4001, ".longhand/background.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)


class PromptTests(support.StorageFolderTest):
    PROMPTS = ".longhand/prompts"

    def test_a_new_prompt_with_a_new_id_passes(self):
        snapshot = self.snapshot()
        text = self.read(self.PROMPTS, "Action items.md")
        text = text.replace("8b2e4f60-7a13-4d9c-b5e2-1c0d9f3a6e21", "5d0f8a52-1f45-4c1e-9a7e-0b1c2d3e4f50")
        text = text.replace('name: "Action items"', 'name: "Action items by person"')
        self.write(text, self.PROMPTS, "Action items by person.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("new: .longhand/prompts/Action items by person.md", out)

    def test_a_copy_that_keeps_the_id_is_a_problem(self):
        snapshot = self.snapshot()
        text = self.read(self.PROMPTS, "Action items.md").replace("owner.", "owner and date.")
        self.write(text, self.PROMPTS, "Action items copy.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("has the id of .longhand/prompts/Action items.md", out)

    def test_a_new_prompt_with_no_id_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("Write the summary as a list.\n", self.PROMPTS, "List.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("has no id", out)

    def test_a_prompt_text_over_the_limit_is_a_problem(self):
        snapshot = self.snapshot()
        text = self.read(self.PROMPTS, "Action items.md").replace("owner.", "owner. " + "x" * 4000)
        self.write(text, self.PROMPTS, "Action items.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("over 4000 characters", out)

    def test_a_rename_by_name_passes_and_tells_the_file_rename(self):
        snapshot = self.snapshot()
        text = self.read(self.PROMPTS, "Action items.md").replace('name: "Action items"', 'name: "To-dos"')
        self.write(text, self.PROMPTS, "Action items.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("renames the file to To-dos.md", out)

    def test_a_file_rename_with_the_same_id_passes(self):
        snapshot = self.snapshot()
        os.rename(self.path(self.PROMPTS, "Action items.md"), self.path(self.PROMPTS, "To-dos.md"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertNotIn("deleted", out)

    def test_a_delete_passes_and_tells_the_default_prompt(self):
        snapshot = self.snapshot()
        os.remove(self.path(self.PROMPTS, "Action items.md"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("deleted: .longhand/prompts/Action items.md", out)
        self.assertIn("default prompt", out)

    def test_an_id_for_a_prompt_with_no_id_passes(self):
        snapshot = self.snapshot()
        self.write("---\nid: 0c1d2e3f-4a5b-4c6d-8e7f-901a2b3c4d5e\nname: \"Plain prompt\"\n---\n\n"
                   "Write a short summary in three sentences.\n", self.PROMPTS, "Plain prompt.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)

    def test_a_setting_can_name_a_prompt_with_no_id(self):
        import _longhand as lh

        snapshot = self.snapshot()
        value = json.loads(self.read(".longhand/settings.json"))
        value["active_summary_prompt"] = lh.file_name_id("Plain prompt.md")
        self.write(json.dumps(value), ".longhand/settings.json")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)


class StopTests(support.StorageFolderTest):
    def test_the_scripts_stop_without_the_library_marker(self):
        os.remove(self.path(".longhand/library"))
        runs = [
            ("check.py", ["snapshot"]),
            ("check.py", ["compare", os.path.join(self.tmp, "none.json")]),
            ("list_notes.py", []),
            ("folder_name.py", [self.path(BUDGET), "A title"]),
        ]
        for script, args in runs:
            with self.subTest(script=script, args=args):
                code, out, err = self.run_script(script, *args)
                self.assertEqual(code, NOT_CHECKED, out + err)
                self.assertIn("Stopped:", err)
        self.assertEqual(os.listdir(self.tmp), [])

    def test_compare_stops_when_the_marker_goes_after_the_snapshot(self):
        snapshot = self.snapshot()
        os.remove(self.path(".longhand/library"))
        code, _, err = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED)
        self.assertIn(".longhand/library", err)

    def test_the_snapshot_is_outside_the_storage_folder(self):
        snapshot = self.snapshot()
        self.assertFalse(os.path.abspath(snapshot).startswith(os.path.abspath(self.root) + os.sep))
        self.assertTrue(os.path.isfile(snapshot))

    def test_a_snapshot_in_the_storage_folder_is_refused(self):
        self.tmp = os.path.join(self.root, "tmp-inside")
        os.makedirs(self.tmp)
        code, _, err = self.run_script("check.py", "snapshot")
        self.assertEqual(code, NOT_CHECKED)
        self.assertIn("outside the storage folder", err)
        self.assertEqual(os.listdir(self.tmp), [])
