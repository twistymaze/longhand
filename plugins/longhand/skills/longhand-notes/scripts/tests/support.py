"""Shared helpers of the script tests."""

import datetime
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = os.path.abspath(os.path.join(SCRIPTS, "..", "..", "..", "..", ".."))
TESTDATA = os.path.join(REPO, "testdata")
STORAGE_FOLDER = os.path.join(TESTDATA, "storage-folder")

if SCRIPTS not in sys.path:
    sys.path.insert(0, SCRIPTS)

BUDGET = "2026-10-01 0900 Budget review"
HIRING = "Meetings/2026-10-02 1400 Hiring sync"
DESIGN = "2026-10-03 1000 Design review"
VENDOR = "2026-10-04 1100 Vendor call"
BROKEN = "2026-10-05 1500 Broken frontmatter"
STANDUP = "Meetings/2026-10-06 0800 Standup"
IDEA = "2026-10-07 1300 Quick idea"
NEWER = "2026-10-08 1600 Newer layout"
RETRO = "2026-10-09 0930 Retro"
BUSY_MARKER = ".longhand-in-use-run-on-another-computer"


def all_files(root):
    """Each file in `root` by its path with `/`, with its bytes."""
    out = {}
    for current, _, files in os.walk(root):
        for name in files:
            path = os.path.join(current, name)
            relative = os.path.relpath(path, root).replace(os.sep, "/")
            with open(path, "rb") as handle:
                out[relative] = handle.read()
    return out


class StorageFolderTest(unittest.TestCase):
    """A test with a copy of the test storage folder. The copy has new file
    times, and the in-use marker of the busy note is fresh."""

    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = os.path.join(self._temp.name, "storage")
        # Snapshots go to a temporary folder of their own, outside the
        # storage folder.
        self.tmp = os.path.join(self._temp.name, "tmp")
        os.makedirs(self.tmp)
        for relative in all_files(STORAGE_FOLDER):
            source = os.path.join(STORAGE_FOLDER, *relative.split("/"))
            target = os.path.join(self.root, *relative.split("/"))
            if not os.path.isdir(os.path.dirname(target)):
                os.makedirs(os.path.dirname(target))
            # copyfile keeps no file times: freshness depends on the time.
            shutil.copyfile(source, target)
        self.refresh_marker()

    def refresh_marker(self):
        marker = self.path(DESIGN, BUSY_MARKER)
        now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
        with open(marker, "w", encoding="utf-8", newline="") as handle:
            json.dump({"run_id": "run-on-another-computer", "refreshed_at": now.isoformat()}, handle)

    def path(self, *parts):
        out = self.root
        for part in parts:
            out = os.path.join(out, *part.split("/"))
        return out

    def read(self, *parts):
        with open(self.path(*parts), "r", encoding="utf-8", newline="") as handle:
            return handle.read()

    def write(self, text, *parts):
        with open(self.path(*parts), "w", encoding="utf-8", newline="") as handle:
            handle.write(text)

    def run_script(self, script, *args):
        """Run a script as an agent runs it. Returns (exit code, stdout,
        stderr). The test also checks that the storage folder has no new
        file after the run."""
        before = set(all_files(self.root))
        env = dict(os.environ)
        for name in ("TMPDIR", "TEMP", "TMP"):
            env[name] = self.tmp
        env["PYTHONIOENCODING"] = "utf-8"
        result = subprocess.run(
            [sys.executable, os.path.join(SCRIPTS, script)] + list(args),
            cwd=self.root,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        after = set(all_files(self.root))
        self.assertEqual(sorted(after - before), [], "%s wrote into the storage folder" % script)
        self.assertFalse(
            os.path.isdir(os.path.join(SCRIPTS, "__pycache__")), "%s wrote bytecode next to the scripts" % script
        )
        return result.returncode, result.stdout.decode("utf-8"), result.stderr.decode("utf-8")

    def snapshot(self):
        code, out, err = self.run_script("check.py", "snapshot")
        self.assertEqual(code, 0, out + err)
        line = [line for line in out.splitlines() if line.startswith("Snapshot: ")][0]
        path = line[len("Snapshot: "):]
        self.assertTrue(path.startswith(self.tmp), path)
        return path

    def compare(self, snapshot):
        return self.run_script("check.py", "compare", snapshot)
