"""check.py snapshot and compare on a copy of the test storage folder."""

import json
import os
import shutil

from tests import support
from tests.support import BROKEN, BUDGET, DESIGN, HIRING, IDEA, NEWER, RETRO, STANDUP, VENDOR

PASS, PROBLEM, NOT_CHECKED = 0, 1, 2

BUDGET_SESSION = "---\ntitle: Budget review\nlonghand_tags:\n- Finance\n- Team::Planning\n---\n"


def section(out, title):
    """The lines of one section of the compare output, as one text."""
    lines, inside = [], False
    for line in out.splitlines():
        if not line.startswith(" "):
            inside = line == title + ":"
            continue
        if inside:
            lines.append(line)
    return "\n".join(lines)


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
        self.assertIn("layout 3", out)
        self.assertIn("Update the Longhand plugin", out)

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
        self.assertIn("one note id", out)

    def test_a_new_file_in_a_note_folder_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("scratch\n", BUDGET, "draft.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("draft.md is a new file", out)

    def test_a_new_file_at_the_root_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("scratch\n", "draft.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("draft.md is a new file that Longhand does not know. Remove it.", out)

    def test_a_new_file_in_a_type_folder_is_a_problem_and_a_new_folder_is_not_checked(self):
        snapshot = self.snapshot()
        self.write("scratch\n", "Meetings", "draft.md")
        os.makedirs(self.path("Meetings", "scratch"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        problems, not_checked = section(out, "Problems"), section(out, "Not checked")
        self.assertIn("Meetings/draft.md is a new file", problems)
        self.assertNotIn("Meetings/scratch", problems)
        self.assertIn("Meetings/scratch", not_checked)

    def test_a_new_hidden_file_in_the_longhand_folder_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("x", ".longhand", ".notes")
        # A temporary file of a Longhand write is not a problem.
        self.write("x", ".longhand", ".settings.json.0123456789abcdef.tmp")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn(".longhand/.notes is a new file", out)
        self.assertNotIn(".tmp", out)

    def test_agent_files_finder_files_and_write_probes_at_the_root_pass(self):
        snapshot = self.snapshot()
        self.write("# Rules\n", "AGENTS.md")
        self.write("@AGENTS.md\n", "CLAUDE.md")
        self.write("", ".DS_Store")
        self.write("", "Meetings", ".DS_Store")
        self.write("", ".longhand-write-probe-1-2-3")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("Changes: none", out)

    def test_a_type_folder_that_becomes_empty_passes(self):
        os.makedirs(self.path("Calls"))
        shutil.move(self.path(BUDGET), self.path("Calls", BUDGET))
        snapshot = self.snapshot()
        shutil.move(self.path("Calls", BUDGET), self.path(BUDGET))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)

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


class LonghandAndSyncWorkTests(support.StorageFolderTest):
    """Folders that Longhand or a sync makes while the agent edits are never
    a problem, and the check never tells the agent to remove a folder."""

    def test_a_longhand_work_folder_is_a_warning_not_a_problem(self):
        snapshot = self.snapshot()
        os.makedirs(self.path(".longhand-staging-abc", "audio"))
        self.write("x", ".longhand-staging-abc", "audio", "segment-0001.wav")
        os.makedirs(self.path("Meetings", ".longhand-rename-abc"))
        os.makedirs(self.path(".longhand-move-abc"))
        os.makedirs(self.path(".longhand-probe-1"))
        os.makedirs(self.path(".longhand-move-probe"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        warnings = section(out, "Warnings")
        for path in (".longhand-staging-abc", "Meetings/.longhand-rename-abc", ".longhand-move-abc"):
            self.assertIn("%s is a folder that Longhand uses" % path, warnings)
        self.assertIn("Do not touch it", warnings)
        self.assertNotIn("probe", out)
        self.assertNotIn("Remove it", out)

    def test_a_note_in_a_longhand_rename_folder_is_not_checked(self):
        snapshot = self.snapshot()
        shutil.move(self.path(IDEA), self.path(".longhand-rename-abc"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertNotIn("is gone", out)
        self.assertIn("Compare again later", section(out, "Not checked"))

    def test_a_note_in_a_longhand_rename_folder_in_a_type_folder_is_not_checked(self):
        snapshot = self.snapshot()
        shutil.move(self.path(HIRING), self.path("Meetings", ".longhand-rename-abc"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertNotIn("is gone", out)
        self.assertIn("Meetings/.longhand-rename-abc is a folder that Longhand uses", section(out, "Warnings"))

    def test_folders_that_a_sync_still_delivers_are_not_checked(self):
        snapshot = self.snapshot()
        os.makedirs(self.path("2026-10-15 0900 Standup"))
        self.write("x", "2026-10-15 0900 Standup", ".syncthing.session.md.tmp")
        os.makedirs(self.path("Clients", "2026-10-15 0900 Call"))
        os.makedirs(self.path(".sync-state"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        not_checked = section(out, "Not checked")
        for path in ("2026-10-15 0900 Standup", "Clients", ".sync-state"):
            self.assertIn(path + " ", not_checked)
        self.assertNotIn("Clients/", out)
        self.assertIn("sync", not_checked)
        self.assertNotIn("Remove it.", out)

    def test_a_new_note_that_a_sync_has_not_finished_is_not_checked(self):
        snapshot = self.snapshot()
        os.makedirs(self.path("2026-10-15 0900 Standup"))
        self.write("---\ntitle: Standup\n---\n", "2026-10-15 0900 Standup", "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertIn("2026-10-15 0900 Standup", section(out, "Not checked"))
        self.assertNotIn("Problems:", out)

    def test_finder_files_in_a_note_folder_do_not_count(self):
        self.write("old", HIRING, ".DS_Store")
        snapshot = self.snapshot()
        self.write("", BUDGET, ".DS_Store")
        self.write("new", HIRING, ".DS_Store")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertIn("Changes: none", out)

    def test_a_snapshot_of_the_earlier_format_stops(self):
        snapshot = self.snapshot()
        with open(snapshot, "r", encoding="utf-8") as handle:
            state = json.load(handle)
        state["format"] = 2
        with open(snapshot, "w", encoding="utf-8") as handle:
            json.dump(state, handle)
        code, _, err = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED)
        self.assertIn("not a snapshot of this script", err)


class ByteOrderMarkTests(support.StorageFolderTest):
    def test_a_session_md_with_a_byte_order_mark_is_a_problem(self):
        snapshot = self.snapshot()
        self.write("\ufeff" + BUDGET_SESSION, BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("byte order mark", section(out, "Problems"))

    def test_a_moved_note_with_a_byte_order_mark_from_before_is_a_problem(self):
        self.write("﻿" + BUDGET_SESSION, BUDGET, "session.md")
        snapshot = self.snapshot()
        os.rename(self.path(BUDGET), self.path(BUDGET + "-2"))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("byte order mark", section(out, "Problems"))

    def test_a_prompt_with_a_byte_order_mark_passes(self):
        snapshot = self.snapshot()
        text = self.read(".longhand/prompts", "Action items.md").replace("owner.", "owner and date.")
        self.write("\ufeff" + text, ".longhand/prompts", "Action items.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)


class DuplicateIdTests(support.StorageFolderTest):
    """Two folders with one note id from before the snapshot."""

    COPY = "2026-10-07 1300 Quick idea-2"

    def setUp(self):
        super().setUp()
        shutil.copytree(self.path(IDEA), self.path(self.COPY), copy_function=shutil.copyfile)

    def test_an_edit_of_another_note_passes(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("- Finance\n", "- Finance\n- Travel\n"), BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertNotIn("one note id", out)
        self.assertNotIn("moved", out)

    def test_an_edit_of_the_copy_is_checked_against_the_copy(self):
        snapshot = self.snapshot()
        text = self.read(self.COPY, "session.md")
        self.write(text.replace("- Ideas\n", "- ' '\n"), self.COPY, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("%s: tag" % self.COPY, section(out, "Problems"))
        self.assertNotIn("moved", out)

    def test_a_deleted_copy_is_gone(self):
        snapshot = self.snapshot()
        shutil.rmtree(self.path(self.COPY))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PROBLEM, out)
        self.assertIn("%s: the note folder is gone" % self.COPY, out)

    def test_a_rename_of_the_first_folder_is_one_move(self):
        snapshot = self.snapshot()
        renamed = "2026-10-07 1300 Quick idea-3"
        os.rename(self.path(IDEA), self.path(renamed))
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertEqual(out.count("moved:"), 1, out)
        self.assertIn("moved: %s -> %s" % (IDEA, renamed), out)
        self.assertNotIn("one note id", out)


class RecordLayoutTests(support.StorageFolderTest):
    """Notes whose longhand.json has layout 1 (`session_id`) or layout 2
    (`note_id`)."""

    def set_record(self, note, layout, id_key):
        record = json.loads(self.read(note, "longhand.json"))
        note_id = record.pop("note_id", None) or record.pop("session_id")
        record.pop("session_id", None)
        record["layout"] = layout
        record[id_key] = note_id
        self.write(json.dumps(record, indent=2, sort_keys=True) + "\n", note, "longhand.json")

    def edit_tags(self):
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("- Finance\n", "- Finance\n- Travel\n"), BUDGET, "session.md")
        return self.compare(snapshot)

    def test_a_tag_edit_of_a_layout_1_note_passes_and_the_note_still_reads(self):
        self.set_record(BUDGET, 1, "session_id")
        code, out, _ = self.edit_tags()
        self.assertEqual(code, PASS, out)
        self.assertIn("changed: %s: session.md" % BUDGET, out)
        code, out, err = self.run_script("list_notes.py", "--json", "--title", "Budget")
        self.assertEqual(code, 0, out + err)
        [note] = json.loads(out)
        self.assertEqual(note["note_id"], "note-budget-review")
        self.assertEqual(note["tags"], ["Finance", "Travel", "Team::Planning"])
        self.assertEqual(note["state"], "ready")

    def test_a_tag_edit_of_a_layout_2_note_passes(self):
        self.set_record(BUDGET, 2, "note_id")
        code, out, _ = self.edit_tags()
        self.assertEqual(code, PASS, out)

    def test_an_id_key_of_the_other_layout_is_a_key_that_the_skill_does_not_know(self):
        for layout, id_key in ((1, "note_id"), (2, "session_id")):
            with self.subTest(layout=layout):
                self.write(BUDGET_SESSION, BUDGET, "session.md")
                self.set_record(BUDGET, layout, id_key)
                code, out, _ = self.edit_tags()
                self.assertEqual(code, NOT_CHECKED, out)
                self.assertIn("keys that this skill does not know: %s" % id_key, out)

    def test_a_session_md_with_session_id_is_still_not_checked(self):
        self.set_record(BUDGET, 1, "session_id")
        snapshot = self.snapshot()
        self.write(BUDGET_SESSION.replace("---\n", "---\nsession_id: note-budget-review\n", 1), BUDGET, "session.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, NOT_CHECKED, out)
        self.assertIn("session_id", out)


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

    def test_a_prompt_with_no_text_is_a_problem(self):
        head = "---\nid: 0c1d2e3f-4a5b-4c6d-8e7f-901a2b3c4d5f\nname: \"Empty\"\n---\n"
        for body in ("", "\n  \t\n", "\n<!-- longhand: title instructions -->\nUse the date.\n"):
            with self.subTest(body=body):
                snapshot = self.snapshot()
                self.write(head + body, self.PROMPTS, "Empty.md")
                code, out, _ = self.compare(snapshot)
                self.assertEqual(code, PROBLEM, out)
                self.assertIn("Empty.md: the prompt has no text", out)
                os.remove(self.path(self.PROMPTS, "Empty.md"))

    def test_a_prompt_of_separator_characters_has_text(self):
        # Longhand's trim keeps U+001C to U+001F, as Rust's white space
        # does not hold them.
        snapshot = self.snapshot()
        self.write("---\nid: 0c1d2e3f-4a5b-4c6d-8e7f-901a2b3c4d5f\nname: \"Odd\"\n---\n\x1c\n",
                   self.PROMPTS, "Odd.md")
        code, out, _ = self.compare(snapshot)
        self.assertEqual(code, PASS, out)
        self.assertNotIn("has no text", out)

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
