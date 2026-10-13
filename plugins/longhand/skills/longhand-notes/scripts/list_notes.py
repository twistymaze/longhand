# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""List the notes of a Longhand storage folder.

    python3 list_notes.py [--root FOLDER] [--type TYPE] [--tag TAG]
                          [--since YYYY-MM-DD] [--until YYYY-MM-DD]
                          [--title TEXT] [--busy | --not-busy] [--json]

Each line gives the folder of a note, its start time, its type, its title,
its tags and its state. A note is busy when a computer records or
processes it, when its recording is not saved yet, or when a move to
another type waits. Do not edit a busy note.

`--tag` compares tag keys: case and the spaces around `::` do not count.
`--tag "Person::"` finds each tag with the scope Person. `--type General`
finds the notes that are not in a type folder.

The script stops with exit code 2 when the folder has no library marker
(`.longhand/library`). It writes nothing.
"""

import argparse
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import _longhand as lh  # noqa: E402


def state(note):
    """The state of a note as one text."""
    parts = lh.unusable_reasons(note)
    if not parts:
        return "ready"
    prefix = "busy: " if note["busy"] else "not readable: "
    return prefix + "; ".join(parts)


def tag_matches(tags, wanted):
    key = lh.tag_key(wanted)
    if key.endswith("::"):
        return any(lh.tag_key(tag).startswith(key) for tag in tags)
    return any(lh.tag_key(tag) == key for tag in tags)


def date_of(note):
    started = lh.parse_time(note["started"])
    return started.strftime("%Y-%m-%d") if started else None


def selected(note, args):
    if args.type is not None:
        note_type = note["type"] or lh.GENERAL
        if not lh.same_type_name(note_type, args.type):
            return False
    if args.tag is not None and not tag_matches(note["tags"], args.tag):
        return False
    day = date_of(note)
    if args.since is not None and (day is None or day < args.since):
        return False
    if args.until is not None and (day is None or day > args.until):
        return False
    if args.title is not None and args.title.lower() not in (note["title"] or "").lower():
        return False
    if args.busy and not note["busy"]:
        return False
    if args.not_busy and note["busy"]:
        return False
    return True


def main(argv=None):
    parser = argparse.ArgumentParser(description="List the notes of a Longhand storage folder.")
    parser.add_argument("--root", default=".", help="the storage folder (default: the current folder)")
    parser.add_argument("--type", help="only notes of this type (General for no type folder)")
    parser.add_argument("--tag", help="only notes with this tag, or with this scope when it ends in ::")
    parser.add_argument("--since", help="only notes that started on this day (YYYY-MM-DD) or later")
    parser.add_argument("--until", help="only notes that started on this day (YYYY-MM-DD) or earlier")
    parser.add_argument("--title", help="only notes whose title contains this text")
    busy = parser.add_mutually_exclusive_group()
    busy.add_argument("--busy", action="store_true", help="only busy notes")
    busy.add_argument("--not-busy", action="store_true", help="only notes that are not busy")
    parser.add_argument("--json", action="store_true", help="print JSON")
    args = parser.parse_args(argv)

    def run():
        root = lh.storage_root(args.root)
        notes = [note for note in lh.notes(root) if selected(note, args)]
        if args.json:
            out = [
                {
                    "path": note["path"],
                    "session_id": note["id"],
                    "started": note["started"],
                    "type": note["type"] or lh.GENERAL,
                    "title": note["title"],
                    "tags": note["tags"],
                    "busy": note["busy"],
                    "state": state(note),
                }
                for note in notes
            ]
            sys.stdout.write(json.dumps(out, indent=1, ensure_ascii=False) + "\n")
            return 0
        lines = []
        for note in notes:
            lines.append(
                "%s | %s | %s | %s | %s | %s"
                % (
                    note["path"],
                    note["started"] or "-",
                    note["type"] or lh.GENERAL,
                    note["title"] or "-",
                    ", ".join(note["tags"]) or "-",
                    state(note),
                )
            )
        lines.append("%d notes" % len(notes))
        lh.print_lines(lines)
        return 0

    return lh.run(run)


if __name__ == "__main__":
    sys.exit(main())
