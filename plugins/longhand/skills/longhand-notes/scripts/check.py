# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Prove what an edit in a Longhand storage folder changed.

    python3 check.py snapshot [--root FOLDER]
    python3 check.py compare SNAPSHOT [--root FOLDER]

`snapshot` records the state of the storage folder in a file in the
temporary folder of the computer, outside the storage folder, and prints
its path. `compare` reads the storage folder again, compares it with the
snapshot, and prints each change and each problem.

Exit codes of `compare`: 0 the check passed, 1 a problem was found, 2 a
part of the change was not checked. Exit code 2 is never a pass. Both
commands stop with exit code 2 when the folder has no library marker
(`.longhand/library`).

The script writes nothing into the storage folder.
"""

import argparse
import datetime
import json
import os
import re
import sys
import tempfile

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import _longhand as lh  # noqa: E402

SNAPSHOT_FORMAT = 3
SNAPSHOT_FOLDER = "longhand-notes-snapshots"

PASS, PROBLEM, NOT_CHECKED = 0, 1, 2


# ---------------------------------------------------------------- state


def read_settings(root):
    path = os.path.join(root, lh.LONGHAND_FOLDER, lh.SETTINGS_FILE)
    if not os.path.isfile(path):
        return {"state": "missing", "value": None}
    value, state = lh.read_json_object(path, lh.SETTINGS_FILE)
    return {"state": state, "value": value}


def longhand_files(root):
    """Each file in `.longhand`, by its path in the storage folder. Hidden
    files are temporary files of a write, and do not count."""
    folder = os.path.join(root, lh.LONGHAND_FOLDER)
    out = {}
    for relative in lh.folder_files(folder):
        if any(part.startswith(".") for part in relative.split("/")):
            continue
        out[lh.LONGHAND_FOLDER + "/" + relative] = lh.digest(os.path.join(folder, *relative.split("/")), relative)
    return out


# The files at the storage-folder root that are not Longhand files but are
# allowed there: the agent instructions of the user.
ROOT_FILES = ("AGENTS.md", "CLAUDE.md")
FINDER_FILE = ".DS_Store"
WRITE_PROBE_PREFIX = ".longhand-write-probe-"
# The temporary file of a Longhand write: `.<name>.<16 hex digits>.tmp`.
LONGHAND_TEMP = re.compile(r"^\..+\.[0-9a-f]{16}\.tmp$")
# The probes that Longhand makes in the storage folder and removes at once.
PROBE_PREFIX = ".longhand-probe-"
MOVE_PROBE = ".longhand-move-probe"
# The folders that Longhand makes at the root or in a type folder while it
# saves, renames or moves a note, and removes when it is done.
WORK_FOLDER_PREFIXES = (".longhand-staging-", ".longhand-rename-", ".longhand-move-")


def is_skipped(name):
    """Whether `name` is a file that the system or Longhand makes and
    removes again, so that it does not count."""
    return (
        name == FINDER_FILE
        or name == MOVE_PROBE
        or name.startswith(WRITE_PROBE_PREFIX)
        or name.startswith(PROBE_PREFIX)
        or bool(LONGHAND_TEMP.match(name))
    )


def is_work_folder(name):
    return name != MOVE_PROBE and name.startswith(WORK_FOLDER_PREFIXES)


def other_entries(root):
    """The files and folders that are not Longhand files, by their paths in
    the storage folder, each with True for a folder: names at the root that
    are not note folders, type folders, `.longhand` or the agent
    instructions; entries in a type folder that are not note folders; and
    hidden files in `.longhand`."""
    types = set(lh.type_folders(root))
    out = {}

    def add(relative, path):
        out[relative] = os.path.isdir(path)

    for name in sorted(os.listdir(root)):
        path = os.path.join(root, name)
        if is_skipped(name) or name == lh.LONGHAND_FOLDER or name in ROOT_FILES:
            continue
        if name in types:
            for child in sorted(os.listdir(path)):
                child_path = os.path.join(path, child)
                if is_skipped(child) or (not lh.is_hidden(child) and lh.is_note_folder(child_path)):
                    continue
                add(name + "/" + child, child_path)
            continue
        if not lh.is_hidden(name) and os.path.isdir(path) and lh.is_note_folder(path):
            continue
        add(name, path)
    folder = os.path.join(root, lh.LONGHAND_FOLDER)
    for relative in lh.folder_files(folder):
        parts = relative.split("/")
        if any(lh.is_hidden(part) for part in parts) and not is_skipped(parts[-1]):
            out[lh.LONGHAND_FOLDER + "/" + relative] = False
    return out


def read_state(root, at=None):
    """The state of the storage folder that a snapshot holds."""
    notes = []
    for note in lh.notes(root, at):
        folder = lh.native(root, note["path"])
        note["files"] = {
            name: lh.digest(os.path.join(folder, *name.split("/")), name) for name in lh.folder_files(folder)
        }
        notes.append(note)
    background = os.path.join(root, lh.LONGHAND_FOLDER, lh.BACKGROUND_FILE)
    return {
        "format": SNAPSHOT_FORMAT,
        "root": root,
        "taken": lh.now().isoformat(),
        "notes": notes,
        "longhand_files": longhand_files(root),
        "settings": read_settings(root),
        "prompts": lh.read_prompts(root),
        "background": background_state(background),
        "type_folders": lh.type_folders(root),
        "other_entries": other_entries(root),
    }


def background_state(path):
    if not os.path.isfile(path):
        return None
    size = os.path.getsize(path)
    if size > lh.BACKGROUND_MAX_FILE_BYTES:
        return {"size": size, "chars": None}
    try:
        return {"size": size, "chars": len(lh.read_text(path))}
    except (OSError, UnicodeDecodeError):
        return {"size": size, "chars": None}


def note_key(note):
    if note["id"] is not None:
        return "id:" + note["id"]
    return "path:" + note["path"]


# ---------------------------------------------------------------- snapshot


def snapshot_folder():
    return os.path.join(tempfile.gettempdir(), SNAPSHOT_FOLDER)


def inside(path, root):
    path = os.path.normcase(os.path.realpath(path))
    root = os.path.normcase(os.path.realpath(root))
    try:
        return os.path.commonpath([path, root]) == root
    except ValueError:
        return False


def take_snapshot(root):
    folder = snapshot_folder()
    if inside(folder, root):
        raise lh.StopError(
            "the temporary folder %s is in the storage folder. Set TMPDIR (TEMP on Windows) "
            "to a folder outside the storage folder." % folder
        )
    state = read_state(root)
    if not os.path.isdir(folder):
        os.makedirs(folder)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    handle = tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", prefix="snapshot-%s-" % stamp, suffix=".json", dir=folder, delete=False
    )
    with handle:
        json.dump(state, handle, indent=1, sort_keys=True)
    return handle.name, state


def cmd_snapshot(args):
    root = lh.storage_root(args.root)
    path, state = take_snapshot(root)
    lines = ["Snapshot: %s" % path, "Notes: %d" % len(state["notes"])]
    for note in state["notes"]:
        reasons = lh.unusable_reasons(note)
        if reasons:
            lines.append("  Do not edit now: %s (%s)" % (note["path"], "; ".join(reasons)))
    lh.print_lines(lines)
    return PASS


# ---------------------------------------------------------------- compare


class Report(object):
    def __init__(self):
        self.changes = []
        self.problems = []
        self.not_checked = []
        self.warnings = []

    def code(self):
        if self.problems:
            return PROBLEM
        if self.not_checked:
            return NOT_CHECKED
        return PASS

    def lines(self):
        out = []
        if self.changes:
            out.append("Changes:")
            out.extend("  " + line for line in self.changes)
        else:
            out.append("Changes: none")
        for title, items in (
            ("Problems", self.problems),
            ("Not checked", self.not_checked),
            ("Warnings", self.warnings),
        ):
            if items:
                out.append("%s:" % title)
                out.extend("  - " + item for item in items)
        out.append(
            "Result: %s"
            % {PASS: "pass", PROBLEM: "problem (exit 1)", NOT_CHECKED: "not checked (exit 2)"}[self.code()]
        )
        return out


def diff_files(before, after):
    added = sorted(set(after) - set(before))
    removed = sorted(set(before) - set(after))
    changed = sorted(name for name in set(before) & set(after) if before[name] != after[name])
    return added, removed, changed


def is_marker(name):
    return name.startswith(lh.IN_USE_PREFIX)


def compare_note(report, before, after):
    added, removed, changed = diff_files(before["files"], after["files"])
    moved = before["path"] != after["path"]
    if not (added or removed or changed or moved):
        return
    path = after["path"]
    touched = [
        name
        for name in added + removed + changed
        if not is_marker(name) and not is_skipped(name.split("/")[-1])
    ]
    if moved:
        report.changes.append("moved: %s -> %s" % (before["path"], path))
    if touched:
        report.changes.append("changed: %s: %s" % (path, ", ".join(touched)))
    if not touched and not moved:
        # Only an in-use marker or a system file changed: a computer started
        # or ended work, or the system wrote its own file.
        return

    for reason in before["busy"] + [r for r in after["busy"] if r not in before["busy"]]:
        report.problems.append("%s: the note was changed while it was busy: %s" % (path, reason))
    if before["record"] != "ok":
        report.not_checked.append("%s: %s" % (path, before["record"]))
        return

    for name in touched:
        if lh.matches(name, lh.PROTECTED_NOTE_PATTERNS):
            report.problems.append(
                "%s: %s changed. Agents must never edit it. If Longhand or a sync changed it, "
                "take a new snapshot." % (path, name)
            )
        elif name in removed and name in lh.EDITABLE_NOTE_PATTERNS:
            report.problems.append("%s: %s was deleted." % (path, name))
        elif name in added and not lh.matches(name, lh.EDITABLE_NOTE_PATTERNS):
            report.problems.append(
                "%s: %s is a new file. Do not add files to a note folder. "
                "If a sync made it, take a new snapshot." % (path, name)
            )

    if after["session_md"] != "ok":
        if lh.SESSION_FILE in touched or moved:
            # Longhand cannot read a session.md that starts with a byte
            # order mark, so that one is a problem of the edit.
            items = report.problems if after["session_md_bom"] else report.not_checked
            items.append("%s: %s" % (path, after["session_md"]))
        return

    if lh.SESSION_FILE in changed:
        for tag in after["tags"]:
            reason = lh.check_tag(tag)
            if reason:
                report.problems.append("%s: tag %r: %s" % (path, tag, reason))
        keys = [lh.tag_key(tag) for tag in after["tags"]]
        duplicates = sorted({key for key in keys if keys.count(key) > 1})
        if duplicates:
            report.warnings.append(
                "%s: two tags have one tag key (%s). Longhand counts them as one tag. "
                "Keep one of them." % (path, ", ".join(duplicates))
            )

    if after["type"] != before["type"] and after["type"] is not None:
        reason = lh.check_type_name(after["type"])
        if reason:
            report.problems.append("%s: the type folder %r is not valid: %s" % (path, after["type"], reason))

    title_changed = after["title"] != before["title"]
    before_name = before["path"].split("/")[-1]
    after_name = path.split("/")[-1]
    if title_changed or before_name != after_name:
        started = lh.parse_time(before["started"])
        expected = lh.folder_name(started, after["title"])
        allowed = [lh.candidate(expected, n) for n in range(1, lh.MAX_NAMES + 1)]
        if after_name not in allowed:
            report.problems.append(
                "%s: the folder name does not follow the title. Rename the folder to %r "
                "(folder_name.py gives the name)." % (path, expected)
            )


def notes_by_key(notes):
    """The notes of one state by their key, each list in path order."""
    out = {}
    for note in sorted(notes, key=lambda n: n["path"]):
        out.setdefault(note_key(note), []).append(note)
    return out


def compare_notes(report, before_state, after_state, longhand_works):
    """Compare the notes of the two states. One key can have more than one
    folder (a copied note): a folder that keeps its path is compared with
    itself, and the folders that are left pair in path order as moves.
    `longhand_works` tells whether a new Longhand work folder is present,
    in which a gone note can be."""
    before = notes_by_key(before_state["notes"])
    after = notes_by_key(after_state["notes"])
    for key in sorted(set(before) | set(after), key=lambda k: (before.get(k) or after[k])[0]["path"]):
        olds, news = before.get(key, []), after.get(key, [])
        old_paths = {n["path"] for n in olds}
        new_by_path = {n["path"]: n for n in news}
        for old in olds:
            if old["path"] in new_by_path:
                compare_note(report, old, new_by_path[old["path"]])
        left_olds = [n for n in olds if n["path"] not in new_by_path]
        left_news = [n for n in news if n["path"] not in old_paths]
        for old, note in zip(left_olds, left_news):
            compare_note(report, old, note)
        for old in left_olds[len(left_news):]:
            if longhand_works:
                report.not_checked.append(
                    "%s: the note folder is not in its place while Longhand saves, renames or moves "
                    "notes. Compare again later." % old["path"]
                )
            else:
                report.problems.append(
                    "%s: the note folder is gone. Do not delete or move a note out of the storage folder."
                    % old["path"]
                )
        extra = left_news[len(left_olds):]
        if not olds and extra:
            compare_new_note(report, extra[0])
            extra = extra[1:]
        for note in extra:
            first = next(n for n in news if n is not note)
            report.problems.append(
                "%s and %s have one note id. Do not copy a note folder." % (first["path"], note["path"])
            )


def compare_new_note(report, note):
    if note["record"] == "ok":
        report.changes.append("new note: %s (a sync can deliver a note from another computer)" % note["path"])
    else:
        report.not_checked.append(
            "%s: a new folder that is not a complete note (%s). A sync can still deliver the other "
            "files. Compare again later." % (note["path"], note["record"])
        )


def valid_prompt_ids(prompts):
    return set(lh.BUILT_IN_PROMPT_IDS) | {info["id"] for info in prompts.values()}


def effective_types(settings, after_state):
    types = list(after_state["type_folders"])
    value = settings.get("note_types")
    if isinstance(value, list):
        types.extend(t for t in value if isinstance(t, str))
    return types


def is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def check_setting(report, key, value, settings, after_state):
    """Problems of one changed key of settings.json."""
    where = "settings.json: %s" % key
    ids = valid_prompt_ids(after_state["prompts"])
    if key in ("live_transcript", "check_updates_automatically", "show_recording_widget", "new_note_records"):
        if not isinstance(value, bool):
            report.problems.append("%s must be true or false." % where)
    elif key == "active_summary_prompt":
        if not isinstance(value, str) or value not in ids:
            report.problems.append("%s must be the id of a prompt: %r is not." % (where, value))
    elif key == "audio_removal":
        if value != "off" and not (
            is_int(value) and lh.AUDIO_REMOVAL_MIN_DAYS <= value <= lh.AUDIO_REMOVAL_MAX_DAYS
        ):
            report.problems.append(
                '%s must be "off" or whole days from %d to %d.'
                % (where, lh.AUDIO_REMOVAL_MIN_DAYS, lh.AUDIO_REMOVAL_MAX_DAYS)
            )
    elif key == "speaker_source":
        if value not in lh.SPEAKER_SOURCES:
            report.problems.append("%s must be one of %s." % (where, ", ".join(lh.SPEAKER_SOURCES)))
    elif key == "speaker_processing_mode":
        if value not in lh.PROCESSING_MODES:
            report.problems.append("%s must be one of %s." % (where, ", ".join(lh.PROCESSING_MODES)))
    elif key == "recognition_language":
        if value is not None and not (isinstance(value, str) and value.strip()):
            report.problems.append("%s must be null or a language code." % where)
    elif key == "asr_model":
        if not isinstance(value, str):
            report.problems.append("%s must be the id of a transcription model." % where)
        else:
            report.warnings.append(
                "%s changed. This skill cannot check the model list. Check it in Longhand Settings." % where
            )
    elif key == "default_speaker_count":
        if not valid_speaker_count(value):
            report.problems.append(
                '%s must be null, {"kind": "exact", "count": N} or {"kind": "range", "min": N, "max": M}.'
                % where
            )
    elif key == "note_types":
        if not isinstance(value, list) or not all(isinstance(t, str) for t in value):
            report.problems.append("%s must be a list of type names." % where)
            return
        for index, name in enumerate(value):
            reason = lh.check_type_name(name)
            if reason:
                report.problems.append("%s: %r: %s." % (where, name, reason))
            if any(lh.same_type_name(name, other) for other in value[:index]):
                report.problems.append("%s: %r is in the list two times." % (where, name))
    elif key == "default_note_type":
        if value is None:
            return
        if not isinstance(value, str) or not any(
            lh.same_type_name(value, t) for t in effective_types(settings, after_state)
        ):
            report.problems.append("%s must be null (General) or a note type: %r is not." % (where, value))
    elif key == "note_type_prompts":
        if not isinstance(value, dict) or not all(isinstance(v, str) for v in value.values()):
            report.problems.append("%s must be an object of type names and prompt ids." % where)
            return
        types = effective_types(settings, after_state) + [lh.GENERAL]
        for name, prompt_id in sorted(value.items()):
            if prompt_id not in ids:
                report.problems.append("%s: %r names no prompt (%r)." % (where, name, prompt_id))
            if not any(lh.same_type_name(name, t) for t in types):
                report.warnings.append("%s: %r is not a note type, so Longhand does not use it." % (where, name))


def valid_speaker_count(value):
    if value is None:
        return True
    if not isinstance(value, dict):
        return False
    if value.get("kind") == "exact" and set(value) == {"kind", "count"}:
        return is_int(value["count"]) and value["count"] >= 1
    if value.get("kind") == "range" and set(value) == {"kind", "min", "max"}:
        return is_int(value["min"]) and is_int(value["max"]) and 1 <= value["min"] <= value["max"]
    return False


def compare_settings(report, before_state, after_state):
    path = lh.LONGHAND_FOLDER + "/" + lh.SETTINGS_FILE
    if before_state["longhand_files"].get(path) == after_state["longhand_files"].get(path):
        return
    before, after = before_state["settings"], after_state["settings"]
    if after["state"] == "missing":
        report.problems.append("%s was deleted." % path)
        return
    report.changes.append("changed: %s" % path)
    if after["state"] != "ok":
        report.problems.append("%s. Longhand does not use a file that does not parse." % after["state"])
        return
    if before["state"] != "ok":
        report.not_checked.append("%s: the snapshot could not read it (%s)." % (path, before["state"]))
        return
    old, new = before["value"], after["value"]
    if not is_int(new.get("schema_version")) or new.get("schema_version") != old.get("schema_version"):
        report.problems.append("%s: do not change schema_version." % path)
        return
    if new["schema_version"] > lh.SETTINGS_SCHEMA_VERSION:
        report.not_checked.append(
            "%s has schema_version %s, and this skill knows %d. Update the Longhand plugin."
            % (path, new["schema_version"], lh.SETTINGS_SCHEMA_VERSION)
        )
        return
    for key in sorted(set(old) - set(new)):
        report.problems.append("%s: the key %s was removed. Keep each key." % (path, key))
    unknown = sorted(set(new) - lh.SETTINGS_KEYS)
    if unknown:
        report.not_checked.append(
            "%s has keys that this skill does not know: %s. Update the Longhand plugin, "
            "or remove a key that the agent added." % (path, ", ".join(unknown))
        )
    changed = sorted(key for key in set(new) & lh.SETTINGS_KEYS if key not in old or old[key] != new[key])
    if changed:
        report.changes[-1] += " (%s)" % ", ".join(changed)
    for key in changed:
        check_setting(report, key, new[key], new, after_state)


def compare_prompts(report, before_state, after_state):
    before, after = before_state["prompts"], after_state["prompts"]
    folder = lh.LONGHAND_FOLDER + "/" + lh.PROMPTS_FOLDER + "/"
    settings = after_state["settings"]["value"] or {}
    for name in sorted(after):
        info = after[name]
        old = before.get(name)
        if old is not None and old == info:
            continue
        where = folder + name
        report.changes.append("%s: %s" % ("new" if old is None else "changed", where))
        if info["unreadable"]:
            report.problems.append("%s cannot be read: %s" % (where, info["unreadable"]))
            continue
        if info["open"]:
            report.problems.append("%s: the front matter has no closing --- line." % where)
        if info["empty"]:
            report.problems.append(
                "%s: the prompt has no text. Longhand does not use a prompt with no text." % where
            )
        if info["over_limit"]:
            report.problems.append(
                "%s: the prompt text is over %d characters or the title instructions are over %d "
                "characters." % (where, lh.PROMPT_TEXT_LIMIT, lh.TITLE_TEXT_LIMIT)
            )
        if info["id_in_file"] is None and old is None:
            report.problems.append("%s has no id. Write a new id from new_id.py into it." % where)
        if info["id_in_file"] in lh.BUILT_IN_PROMPT_IDS:
            report.problems.append("%s uses the id of a built-in prompt. Write a new id." % where)
        renamed = old is None and any(
            other not in after and other_info["id_in_file"] == info["id_in_file"]
            for other, other_info in before.items()
        )
        if info["id_in_file"] is not None and not renamed and (
            old is None or old["id_in_file"] != info["id_in_file"]
        ):
            holders = [
                other
                for other, other_info in after.items()
                if other != name
                and other_info["id_in_file"] == info["id_in_file"]
                and before.get(other, {}).get("id_in_file") == info["id_in_file"]
            ]
            if holders:
                report.problems.append(
                    "%s has the id of %s. A new prompt needs a new id from new_id.py."
                    % (where, ", ".join(folder + h for h in sorted(holders)))
                )
        if info["name"] is not None:
            renamed = lh.safe_file_name(info["name"])
            if renamed != name:
                report.changes.append("%s: Longhand renames the file to %s at its next write." % (where, renamed))
    for name in sorted(set(before) - set(after)):
        old = before[name]
        moved = [other for other, info in after.items() if info["id"] == old["id"] and other not in before]
        if moved:
            continue
        report.changes.append("deleted: %s%s" % (folder, name))
        names_it = settings.get("active_summary_prompt") == old["id"] or (
            isinstance(settings.get("note_type_prompts"), dict)
            and old["id"] in settings["note_type_prompts"].values()
        )
        if names_it:
            report.changes.append(
                "  The settings that name it use the default prompt now. Longhand removes the references later."
            )


def compare_longhand_files(report, before_state, after_state):
    before, after = before_state["longhand_files"], after_state["longhand_files"]
    added, removed, changed = diff_files(before, after)
    background = lh.LONGHAND_FOLDER + "/" + lh.BACKGROUND_FILE
    for path in added + removed + changed:
        if lh.matches(path, lh.PROTECTED_STORAGE_PATTERNS):
            report.problems.append("%s changed. Agents must never edit it." % path)
        elif path == background:
            if path in removed:
                report.problems.append("%s was deleted. To clear the Background, make the file empty." % path)
                continue
            report.changes.append("changed: %s" % path)
            state = after_state["background"]
            if state["chars"] is None or state["chars"] > lh.BACKGROUND_TEXT_LIMIT:
                report.problems.append("%s is over %d characters." % (path, lh.BACKGROUND_TEXT_LIMIT))
        elif lh.matches(path, lh.EDITABLE_STORAGE_PATTERNS):
            continue
        elif path in added:
            report.problems.append("%s is a new file that Longhand does not know. Remove it." % path)
        else:
            report.problems.append("%s changed. Agents must not edit it." % path)


def compare_other_entries(report, before_state, after_state):
    """Report each new file or folder that Longhand does not know. A new
    file is a problem. A new folder can be a Longhand work folder, a note
    that a sync has not delivered yet or the folder of another app, so the
    agent must not remove it. A type folder that has no note now is not
    new. Return whether a new Longhand work folder is present."""
    known = set(before_state["other_entries"]) | set(before_state["type_folders"])
    longhand_works = False
    for path, is_folder in sorted(after_state["other_entries"].items()):
        if path in known:
            continue
        if not is_folder:
            report.problems.append("%s is a new file that Longhand does not know. Remove it." % path)
        elif is_work_folder(path.split("/")[-1]):
            longhand_works = True
            report.warnings.append(
                "%s is a folder that Longhand uses while it saves, renames or moves a note. "
                "Do not touch it." % path
            )
        else:
            report.not_checked.append(
                "%s is a new folder that is not a note. It can be a note that a sync has not "
                "delivered yet, or the folder of another app. Compare again later. Remove it "
                "only if you made it." % path
            )
    return longhand_works


def compare(before_state, after_state):
    report = Report()
    longhand_works = compare_other_entries(report, before_state, after_state)
    compare_notes(report, before_state, after_state, longhand_works)
    compare_settings(report, before_state, after_state)
    compare_prompts(report, before_state, after_state)
    compare_longhand_files(report, before_state, after_state)
    return report


def cmd_compare(args):
    try:
        with open(args.snapshot, "r", encoding="utf-8") as handle:
            before = json.load(handle)
    except (OSError, ValueError) as error:
        raise lh.StopError("the snapshot %s cannot be read: %s" % (args.snapshot, error))
    if before.get("format") != SNAPSHOT_FORMAT:
        raise lh.StopError("the snapshot %s is not a snapshot of this script" % args.snapshot)
    root = lh.storage_root(args.root or before["root"])
    if os.path.normcase(root) != os.path.normcase(before["root"]):
        raise lh.StopError("the snapshot is of %s, not of %s" % (before["root"], root))
    report = compare(before, read_state(root))
    lh.print_lines(report.lines())
    return report.code()


def main(argv=None):
    parser = argparse.ArgumentParser(description="Prove what an edit in a Longhand storage folder changed.")
    commands = parser.add_subparsers(dest="command")
    snap = commands.add_parser("snapshot", help="record the state of the storage folder")
    snap.add_argument("--root", default=".", help="the storage folder (default: the current folder)")
    comp = commands.add_parser("compare", help="compare the storage folder with a snapshot")
    comp.add_argument("snapshot", help="the path that snapshot printed")
    comp.add_argument("--root", default=None, help="the storage folder (default: the folder of the snapshot)")
    args = parser.parse_args(argv)
    if args.command == "snapshot":
        return lh.run(lambda: cmd_snapshot(args))
    if args.command == "compare":
        return lh.run(lambda: cmd_compare(args))
    parser.print_help()
    return NOT_CHECKED


if __name__ == "__main__":
    sys.exit(main())
