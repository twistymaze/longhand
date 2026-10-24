"""list_notes.py on a copy of the test storage folder."""

import json

from tests import support
from tests.support import BUDGET, DESIGN, HIRING, IDEA, STANDUP, VENDOR


class ListNotesTests(support.StorageFolderTest):
    def listed(self, *args):
        code, out, err = self.run_script("list_notes.py", "--json", *args)
        self.assertEqual(code, 0, out + err)
        return json.loads(out)

    def test_every_note_folder_is_listed_with_its_type(self):
        notes = {note["path"]: note for note in self.listed()}
        self.assertEqual(
            sorted(notes),
            sorted([
                BUDGET, HIRING, DESIGN, VENDOR, support.BROKEN, STANDUP, IDEA, support.NEWER, support.RETRO,
            ]),
        )
        self.assertEqual(notes[HIRING]["type"], "Meetings")
        self.assertEqual(notes[BUDGET]["type"], "General")
        self.assertEqual(notes[BUDGET]["note_id"], "note-budget-review")
        self.assertEqual(notes[BUDGET]["tags"], ["Finance", "Team::Planning"])
        self.assertEqual(notes[BUDGET]["state"], "ready")
        # An imported note has an `import` record, and it reads as any
        # other note.
        self.assertEqual(notes[HIRING]["state"], "ready")

    def test_a_layout_1_record_with_session_id_is_listed_with_its_note_id(self):
        record = json.loads(self.read(BUDGET, "longhand.json"))
        record.pop("note_id", None)
        record["layout"] = 1
        record["session_id"] = "note-budget-review"
        self.write(json.dumps(record, indent=2, sort_keys=True) + "\n", BUDGET, "longhand.json")
        notes = {note["path"]: note for note in self.listed()}
        self.assertEqual(notes[BUDGET]["note_id"], "note-budget-review")
        self.assertNotIn("session_id", notes[BUDGET])
        self.assertEqual(notes[BUDGET]["state"], "ready")

    def test_a_session_md_with_a_byte_order_mark_is_not_ready(self):
        self.write("\ufeff" + self.read(BUDGET, "session.md"), BUDGET, "session.md")
        notes = {note["path"]: note for note in self.listed()}
        self.assertNotEqual(notes[BUDGET]["state"], "ready")

    def test_busy_notes_have_a_busy_flag(self):
        busy = sorted(note["path"] for note in self.listed("--busy"))
        self.assertEqual(busy, sorted([DESIGN, STANDUP, VENDOR]))
        not_busy = {note["path"] for note in self.listed("--not-busy")}
        self.assertFalse(not_busy & set(busy))

    def test_a_stale_marker_does_not_make_a_note_busy(self):
        stale = {"run_id": "run-on-another-computer", "refreshed_at": "2026-10-03T10:05:00-05:00"}
        self.write(json.dumps(stale), DESIGN, support.BUSY_MARKER)
        busy = sorted(note["path"] for note in self.listed("--busy"))
        self.assertEqual(busy, sorted([STANDUP, VENDOR]))

    def test_the_tag_filter_compares_tag_keys(self):
        self.assertEqual([n["path"] for n in self.listed("--tag", "team :: ops")], [HIRING])
        self.assertEqual(
            sorted(n["path"] for n in self.listed("--tag", "Team::")),
            sorted([BUDGET, HIRING, support.RETRO]),
        )

    def test_the_type_title_and_date_filters(self):
        self.assertEqual(sorted(n["path"] for n in self.listed("--type", "meetings")), sorted([HIRING, STANDUP]))
        self.assertNotIn(HIRING, [n["path"] for n in self.listed("--type", "General")])
        self.assertEqual([n["path"] for n in self.listed("--title", "BUDGET")], [BUDGET])
        self.assertEqual(
            sorted(n["path"] for n in self.listed("--since", "2026-10-06", "--until", "2026-10-07")),
            sorted([STANDUP, IDEA]),
        )

    def test_the_text_output_has_one_line_for_each_note(self):
        code, out, _ = self.run_script("list_notes.py", "--type", "Meetings")
        self.assertEqual(code, 0)
        lines = out.splitlines()
        self.assertEqual(lines[-1], "2 notes")
        self.assertTrue(lines[0].startswith(HIRING + " | 2026-10-02T14:00:00-05:00 | Meetings | Hiring sync"))
