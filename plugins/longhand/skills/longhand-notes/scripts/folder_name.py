# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Print the folder name that Longhand gives a note with a title.

    python3 folder_name.py NOTE_FOLDER "New title"
    python3 folder_name.py --started 2026-10-01T09:00:00-05:00 "New title"

The first form reads the start time of the note from its `longhand.json`,
and prints the first free name next to the note folder: the name itself,
or the name with `-2`, `-3` and so on when another folder has the name.
The note's own folder does not count as taken. The second form prints the
name for a start time, with no check of a folder.

The name is `YYYY-MM-DD HHMM <title>` in the note's own clock. Characters
that macOS or Windows refuse, control characters, spaces and dots at the
ends, and the characters after the first 120 go. A title with nothing left
gives `YYYY-MM-DD HHMM Longhand session`.

The first form stops with exit code 2 when the storage folder has no
library marker (`.longhand/library`). The script writes nothing.
"""

import argparse
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import _longhand as lh  # noqa: E402


def storage_root_of(folder):
    """The storage folder of a note folder: its parent, or the parent of
    its type folder."""
    parent = os.path.dirname(folder)
    for candidate_root in (parent, os.path.dirname(parent)):
        if os.path.isfile(os.path.join(candidate_root, lh.LONGHAND_FOLDER, lh.LIBRARY_MARKER)):
            return candidate_root
    return lh.storage_root(parent)


def free_name(folder, name):
    parent = os.path.dirname(folder)
    own = os.path.basename(folder)
    taken = {entry.lower() for entry in os.listdir(parent) if entry != own}
    for attempt in range(1, lh.MAX_NAMES + 1):
        option = lh.candidate(name, attempt)
        if option.lower() not in taken or option == own:
            return option
    raise lh.StopError("no free name for %s after %d tries" % (name, lh.MAX_NAMES))


def main(argv=None):
    parser = argparse.ArgumentParser(description="Print the folder name that Longhand gives a note.")
    parser.add_argument("note", nargs="?", help="the note folder")
    parser.add_argument("title", help="the title")
    parser.add_argument("--started", help="the start time of the note, in RFC 3339")
    args = parser.parse_args(argv)

    def run():
        if args.started is not None:
            if args.note is not None:
                raise lh.StopError("give a note folder or --started, not both")
            started = lh.parse_time(args.started)
            if started is None:
                raise lh.StopError("%r is not an RFC 3339 time with an offset" % args.started)
            sys.stdout.write(lh.folder_name(started, args.title) + "\n")
            return 0
        if args.note is None:
            raise lh.StopError("give a note folder or --started")
        folder = os.path.abspath(args.note.rstrip("/\\"))
        storage_root_of(folder)
        record, record_state = lh.read_record(folder)
        if record_state != "ok":
            raise lh.StopError("%s: %s" % (folder, record_state))
        name = lh.folder_name(lh.parse_time(record["started"]), args.title)
        sys.stdout.write(free_name(folder, name) + "\n")
        return 0

    return lh.run(run)


if __name__ == "__main__":
    sys.exit(main())
