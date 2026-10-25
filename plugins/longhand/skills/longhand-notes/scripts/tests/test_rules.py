"""The rules in _longhand.py, folder_name.py and new_id.py."""

import json
import os
import re
import subprocess
import sys
import unittest

from tests import support

import _longhand as lh
import new_id

SKILL = os.path.dirname(support.SCRIPTS)
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")


def run(script, *args):
    result = subprocess.run(
        [sys.executable, os.path.join(support.SCRIPTS, script)] + list(args),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=dict(os.environ, PYTHONIOENCODING="utf-8"),
    )
    return result.returncode, result.stdout.decode("utf-8"), result.stderr.decode("utf-8")


class FolderNameTests(unittest.TestCase):
    def cases(self):
        with open(os.path.join(support.TESTDATA, "folder-names.json"), "r", encoding="utf-8") as handle:
            return json.load(handle)["cases"]

    def test_the_shared_cases(self):
        cases = self.cases()
        self.assertGreater(len(cases), 10)
        for case in cases:
            with self.subTest(case=case["case"]):
                started = lh.parse_time(case["started"])
                self.assertEqual(lh.folder_name(started, case["title"]), case["name"])

    def test_the_script_prints_the_name_for_a_start_time(self):
        for case in self.cases():
            title = case["title"]
            if title is None or title.startswith("-") or any(ord(c) < 0x20 for c in title):
                continue
            with self.subTest(case=case["case"]):
                code, out, err = run("folder_name.py", "--started", case["started"], title)
                self.assertEqual(code, 0, err)
                self.assertEqual(out.rstrip("\r\n"), case["name"])


class FolderNameInFolderTests(support.StorageFolderTest):
    def test_the_script_gives_a_free_name_next_to_the_note(self):
        os.makedirs(self.path("2026-10-01 0900 Taken"))
        code, out, err = self.run_script("folder_name.py", self.path(support.BUDGET), "Taken")
        self.assertEqual(code, 0, err)
        self.assertEqual(out.strip(), "2026-10-01 0900 Taken-2")

    def test_the_own_folder_of_the_note_is_not_taken(self):
        code, out, err = self.run_script("folder_name.py", self.path(support.BUDGET), "Budget review")
        self.assertEqual(code, 0, err)
        self.assertEqual(out.strip(), support.BUDGET)

    def test_a_note_in_a_type_folder(self):
        code, out, err = self.run_script("folder_name.py", self.path(support.HIRING), "Hiring: next steps")
        self.assertEqual(code, 0, err)
        self.assertEqual(out.strip(), "2026-10-02 1400 Hiring next steps")


class NewIdTests(unittest.TestCase):
    def test_an_id_is_a_random_uuid(self):
        ids = {new_id.new_id() for _ in range(50)}
        self.assertEqual(len(ids), 50)
        for value in ids:
            self.assertRegex(value, UUID)

    def test_the_script_prints_one_id(self):
        code, out, _ = run("new_id.py")
        self.assertEqual(code, 0)
        self.assertRegex(out.strip(), UUID)


class FrontmatterTests(unittest.TestCase):
    def read(self, text):
        return lh.read_session_md(text)

    def test_the_forms_that_longhand_and_agents_write(self):
        cases = [
            ("---\ntitle: Budget review\nlonghand_tags:\n- Finance\n- Team::Planning\n---\n",
             ("Budget review", ["Finance", "Team::Planning"])),
            ("---\ntitle: Hiring sync\nlonghand_tags:\n  - 'Team :: Ops'\n  - \"it's\"\n---\nBody\n",
             ("Hiring sync", ["Team :: Ops", "it's"])),
            ("---\ntitle: \"A \\\"quoted\\\" title\"\nlonghand_tags: [a, 'b, c', \"d\"]\n---\n",
             ('A "quoted" title', ["a", "b, c", "d"])),
            ("---\r\ntitle: 'It''s here' # a comment\r\nlonghand_tags: []\r\n---\r\n", ("It's here", [])),
            ("---\n---\n", (None, [])),
            ("---\ntitle:\nother: value\n---", (None, [])),
            ("---\ntitle: Release 2.0 plan\nrating: 3\n---\n", ("Release 2.0 plan", [])),
        ]
        for text, expected in cases:
            with self.subTest(text=text):
                self.assertEqual(self.read(text), expected)

    def test_forms_that_the_reader_does_not_read_are_unreadable(self):
        cases = [
            ("no frontmatter\n", 1),
            ("---\ntitle: open\n", 1),
            ("---\ntitle: Broken frontmatter\nlonghand_tags: [Finance, Team\n---\n", 3),
            ("---\ntitle: >\n  folded\n---\n", 2),
            ("---\ntitle: a: b\n---\n", 2),
            ("---\ntitle: one\ntitle: two\n---\n", 3),
            ("---\nlonghand_tags:\n- [a, b]\n---\n", 3),
            ("---\ntitle: \"open\n---\n", 2),
            ("---\ntitle: true\n---\n", None),
            ("---\nlonghand_tags:\n- 12\n---\n", None),
            ("---\n  indented: value\n---\n", 2),
            ("---\nsession_id: abc\n---\n", None),
            # SessionMd::parse in crates/session-core does not remove a byte order mark.
            ("\ufeff---\ntitle: Plain\n---\n", None),
        ]
        for text, line in cases:
            with self.subTest(text=text):
                with self.assertRaises(lh.Unreadable) as raised:
                    self.read(text)
                self.assertEqual(raised.exception.line, line)


class TagAndTypeTests(unittest.TestCase):
    def test_tag_keys(self):
        self.assertEqual(lh.tag_key(" Team :: Ops "), "team::ops")
        self.assertEqual(lh.tag_key("Person::Nathan"), lh.tag_key("person :: nathan"))
        self.assertNotEqual(lh.tag_key("Person::Nathan"), lh.tag_key("Person::Nat"))

    def test_tag_checks(self):
        self.assertIsNone(lh.check_tag("Person::Nathan"))
        self.assertIsNotNone(lh.check_tag("  "))
        self.assertIsNotNone(lh.check_tag("a\tb"))

    def test_type_name_checks(self):
        self.assertIsNone(lh.check_type_name("One on One"))
        for name in ("", "general", ".hidden", "a/b", "a:b", "ends.", "CON", "x" * 121, "a\nb"):
            with self.subTest(name=name):
                self.assertIsNotNone(lh.check_type_name(name))


class PromptRuleTests(unittest.TestCase):
    def test_safe_file_names(self):
        cases = [
            ("Weekly review", "Weekly review.md"),
            ("1:1 notes", "1-1 notes.md"),
            ("  .hidden. ", "hidden.md"),
            ("~$lock", "~-lock.md"),
            ("CON", "_CON.md"),
            ("???", "---.md"),
            ("...", "Prompt.md"),
        ]
        for name, expected in cases:
            with self.subTest(name=name):
                self.assertEqual(lh.safe_file_name(name), expected)

    def test_a_prompt_file_parses_as_longhand_reads_it(self):
        parsed = lh.parse_prompt(
            "---\nid: abc\nname: \"Action items\"\nextra: kept\n---\n\nList the items.\n"
            "<!-- longhand: title instructions -->\nUse the main item.\n"
        )
        self.assertEqual(parsed["id"], "abc")
        self.assertEqual(parsed["name"], "Action items")
        self.assertEqual(parsed["text"], "List the items.")
        self.assertEqual(parsed["title_text"], "Use the main item.")
        self.assertFalse(parsed["open"])
        self.assertTrue(lh.parse_prompt("---\nid: abc\n")["open"])
        plain = lh.parse_prompt("Write a short summary.\n")
        self.assertIsNone(plain["id"])
        self.assertEqual(plain["text"], "Write a short summary.")


class FilesReferenceTests(unittest.TestCase):
    """references/files.md names each protected and editable pattern of
    _longhand.py, so the text and the checks agree."""

    def test_files_md_names_each_pattern(self):
        with open(os.path.join(SKILL, "references", "files.md"), "r", encoding="utf-8") as handle:
            text = handle.read()
        patterns = (
            lh.PROTECTED_NOTE_PATTERNS
            + lh.PROTECTED_STORAGE_PATTERNS
            + lh.LONGHAND_ONLY_STORAGE_PATTERNS
            + lh.EDITABLE_NOTE_PATTERNS
            + lh.EDITABLE_STORAGE_PATTERNS
        )
        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertIn("`%s`" % pattern, text)

    def test_the_skill_names_each_reference_file(self):
        with open(os.path.join(SKILL, "SKILL.md"), "r", encoding="utf-8") as handle:
            text = handle.read()
        for name in sorted(os.listdir(os.path.join(SKILL, "references"))):
            with self.subTest(name=name):
                self.assertIn("references/%s" % name, text)


if __name__ == "__main__":
    unittest.main()
