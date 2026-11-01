# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""The shared rules of the longhand-notes scripts.

This module holds Longhand's file names, the protected and editable file
patterns, the naming rules of note folders, and the readers of the files in
a storage folder. It uses only the Python standard library. It reads
`longhand.json` as JSON and reads frontmatter with a small reader for the
YAML forms that Longhand and agents write. It never writes into the storage
folder.

The values here follow the Longhand code. The tests and the Rust drift test
(`crates/library/tests/skill_drift.rs`) fail when they do not agree.
"""

import datetime
import fnmatch
import hashlib
import json
import os
import re
import sys
import unicodedata
import uuid

sys.dont_write_bytecode = True

# A title can hold characters that the console encoding of Windows does not
# have. Such a character prints as an escape, not as an error.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(errors="backslashreplace")
    except (AttributeError, ValueError):
        # A replaced stream can have no reconfigure method. It keeps its
        # own error handling.
        pass

# ---------------------------------------------------------------- file names

LONGHAND_FOLDER = ".longhand"
LIBRARY_MARKER = "library"
SETTINGS_FILE = "settings.json"
BACKGROUND_FILE = "background.md"
PROMPTS_FOLDER = "prompts"
IMPORTS_FOLDER = "imports"
NOTE_RECORD_FILE = "longhand.json"
SESSION_FILE = "session.md"
NOTES_FILE = "notes.md"
TRANSCRIPT_FILE = "transcript.md"
SUMMARY_FILE = "summary.md"
RECOGNITION_FILE = "recognition.jsonl"
# The marker that Longhand writes when it removes the transcript of a note.
# The note then has no `transcript.md` and no `recognition.jsonl`.
TRANSCRIPT_REMOVED_MARKER = "transcript.removed"
AUDIO_FOLDER = "audio"
IN_USE_PREFIX = ".longhand-in-use-"

# The keys that each layout of `longhand.json` shares.
_SHARED_RECORD_KEYS = frozenset(
    [
        "asr_model",
        "found_speakers",
        "layout",
        "pending_type",
        "recognition_artifact",
        "recognition_audio",
        "recognition_snapshot",
        "result",
        "segments",
        "speaker_count",
        "started",
        "summary",
        "summary_fingerprint",
        "transcript_fingerprint",
        "written_title",
    ]
)

# The layouts of `longhand.json` that these scripts know, each with the key
# that holds the note id. Layout 1 holds the note id in `session_id`, and
# layout 2 holds it in `note_id`.
RECORD_ID_KEYS = {1: "session_id", 2: "note_id"}

# The keys that only one layout has, other than the note id. Layout 2 has
# the optional key `import`: the source file of an imported note.
_LAYOUT_RECORD_KEYS = {1: frozenset(), 2: frozenset(["import"])}

# The keys of a `longhand.json` in each known layout. A file with another
# key comes from a Longhand that these scripts do not know, so a check
# reports it as not checked.
RECORD_KEYS = {
    layout: _SHARED_RECORD_KEYS | {id_key} | _LAYOUT_RECORD_KEYS[layout]
    for layout, id_key in RECORD_ID_KEYS.items()
}

# The keys of `.longhand/settings.json` that these scripts know, with the
# schema version.
SETTINGS_SCHEMA_VERSION = 1
SETTINGS_KEYS = frozenset(
    [
        "active_summary_prompt",
        "asr_model",
        "audio_removal",
        "check_updates_automatically",
        "default_note_type",
        "default_speaker_count",
        "live_transcript",
        "new_note_records",
        "note_type_prompts",
        "note_types",
        "recognition_language",
        "schema_version",
        "show_recording_widget",
        "speaker_processing_mode",
        "speaker_source",
        "transcript_removal",
    ]
)

# The files that an agent must never edit, as patterns. A pattern without
# `/` matches a file name in a note folder. A pattern that starts with
# `.longhand/` matches a path in the storage folder. `references/files.md`
# names each of these patterns, and a test checks that.
PROTECTED_NOTE_PATTERNS = (
    NOTE_RECORD_FILE,
    RECOGNITION_FILE,
    TRANSCRIPT_REMOVED_MARKER,
    AUDIO_FOLDER + "/*",
    IN_USE_PREFIX + "*",
)
PROTECTED_STORAGE_PATTERNS = (LONGHAND_FOLDER + "/" + LIBRARY_MARKER,)

# The files that only Longhand writes, as patterns in the storage folder.
# Longhand writes them at any time, also while an agent edits. An agent
# must never edit them. A check does not compare them, so a write of
# Longhand is not a change of the agent. `references/files.md` names each
# of these patterns.
LONGHAND_ONLY_STORAGE_PATTERNS = (LONGHAND_FOLDER + "/" + IMPORTS_FOLDER + "/*",)

# The files that an agent can edit, with the steps in the references.
EDITABLE_NOTE_PATTERNS = (SESSION_FILE, NOTES_FILE, TRANSCRIPT_FILE, SUMMARY_FILE)
EDITABLE_STORAGE_PATTERNS = (
    LONGHAND_FOLDER + "/" + SETTINGS_FILE,
    LONGHAND_FOLDER + "/" + BACKGROUND_FILE,
    LONGHAND_FOLDER + "/" + PROMPTS_FOLDER + "/*.md",
)

# ---------------------------------------------------------------- limits

# An in-use marker is fresh for this long after its last refresh.
FRESH_FOR = datetime.timedelta(minutes=30)
# A marker time up to this far in the future is fresh.
CLOCK_MARGIN = datetime.timedelta(minutes=5)

PROMPT_TEXT_LIMIT = 4000
TITLE_TEXT_LIMIT = 1000
BACKGROUND_TEXT_LIMIT = 4000
TITLE_MARKER = "<!-- longhand: title instructions -->"
BYTE_ORDER_MARK = "\ufeff"
# The largest prompt file that Longhand reads, in bytes.
PROMPT_MAX_FILE_BYTES = PROMPT_TEXT_LIMIT * 4 + TITLE_TEXT_LIMIT * 4 + len(TITLE_MARKER) + 2 + 4096
BACKGROUND_MAX_FILE_BYTES = BACKGROUND_TEXT_LIMIT * 4

AUDIO_REMOVAL_MIN_DAYS = 1
AUDIO_REMOVAL_MAX_DAYS = 3650
TRANSCRIPT_REMOVAL_MIN_DAYS = AUDIO_REMOVAL_MIN_DAYS
TRANSCRIPT_REMOVAL_MAX_DAYS = AUDIO_REMOVAL_MAX_DAYS
SPEAKER_SOURCES = ("off", "pyannote", "sortformer", "nemotron")
PROCESSING_MODES = ("during_recording", "after_stop")

BUILT_IN_PROMPT_IDS = ("topic-summary", "play-by-play")
# The namespace of the prompt ids that Longhand makes from file names.
PROMPT_ID_NAMESPACE = uuid.UUID(int=0x6C6F6E6768615E6480726F6D70747331)

# ---------------------------------------------------------------- naming

MAX_TITLE_CHARS = 120
MAX_NAMES = 100
GENERAL = "General"
NO_TITLE_NAME = "Longhand session"
FOLDER_FORBIDDEN = '"*:<>?/\\|'
RESERVED_NAMES = (
    "CON", "PRN", "AUX", "NUL",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
)
RESERVED_FILES = ("desktop.ini", "thumbs.db", ".lock")

# The characters that Rust's `char::is_whitespace` accepts (the Unicode
# White_Space property). Python's `str.isspace` accepts other characters
# too, so the naming rules use this set.
WHITE_SPACE = frozenset(
    chr(code)
    for code in [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x85, 0xA0, 0x1680]
    + list(range(0x2000, 0x200B))
    + [0x2028, 0x2029, 0x202F, 0x205F, 0x3000]
)


def is_control(char):
    """Whether `char` is a control character (Unicode category Cc)."""
    return unicodedata.category(char) == "Cc"


def is_reserved(name):
    stem = name.split(".")[0]
    return any(stem.upper() == r for r in RESERVED_NAMES) or any(
        name.lower() == r for r in RESERVED_FILES
    )


def sanitize_title(title):
    """The title as a safe part of a folder name, or None when nothing is
    left. This follows `sanitize_title` in `crates/session-save/src/naming.rs`."""
    kept = "".join(
        " " if c in WHITE_SPACE else c
        for c in title
        if c in WHITE_SPACE or not (is_control(c) or c in FOLDER_FORBIDDEN)
    )
    collapsed = " ".join(word for word in kept.split(" ") if word)
    cut = collapsed.strip(" .")[:MAX_TITLE_CHARS]
    title = cut.strip(" .")
    if not title:
        return None
    if is_reserved(title):
        at = title.find(".")
        if at < 0:
            at = len(title)
        return title[:at] + "_" + title[at:]
    return title


def folder_name(started, title):
    """`YYYY-MM-DD HHMM <title>` in the note's own clock. `started` is a
    datetime with its offset."""
    safe = sanitize_title(title) if title is not None else None
    return "%s %s" % (started.strftime("%Y-%m-%d %H%M"), safe or NO_TITLE_NAME)


def candidate(name, attempt):
    """The name itself, then `name-2`, `name-3`, and so on."""
    return name if attempt <= 1 else "%s-%d" % (name, attempt)


def same_type_name(a, b):
    return a.strip().lower() == b.strip().lower()


def check_type_name(name):
    """None when `name` can be a type folder name, or the reason."""
    name = name.strip()
    if not name:
        return "a type name cannot be empty"
    if same_type_name(name, GENERAL):
        return "General is the built-in type and has no type folder"
    if name.startswith("."):
        return "a type name cannot start with a dot"
    if any(is_control(c) for c in name):
        return "a type name cannot contain control characters"
    for c in name:
        if c in FOLDER_FORBIDDEN:
            return 'a type name cannot contain "%s"' % c
    if name.endswith("."):
        return "a type name cannot end with a dot"
    if is_reserved(name):
        return "Windows uses this name for its own files"
    if len(name) > MAX_TITLE_CHARS:
        return "a type name can have at most %d characters" % MAX_TITLE_CHARS
    return None


# ---------------------------------------------------------------- tags


def tag_key(text):
    """The key of a tag: two tags with one key are one tag."""
    return "::".join(part.strip() for part in text.split("::")).lower()


def check_tag(text):
    """None when `text` is a valid tag, or the reason."""
    text = text.strip()
    if not text:
        return "a tag cannot be empty"
    if any(is_control(c) for c in text):
        return "a tag cannot contain a line break, a tab or another control character"
    return None


# ---------------------------------------------------------------- time

_FRACTION = re.compile(r"(\.\d{1,6})\d*")


def parse_time(text):
    """A datetime from an RFC 3339 text, or None. Python 3.9 reads at most
    six digits of a fraction and no `Z`."""
    if not isinstance(text, str):
        return None
    text = text.strip()
    if text.endswith("Z") or text.endswith("z"):
        text = text[:-1] + "+00:00"
    text = _FRACTION.sub(lambda m: m.group(1), text, count=1)
    try:
        value = datetime.datetime.fromisoformat(text)
    except ValueError:
        return None
    if value.tzinfo is None:
        return None
    return value


def now():
    return datetime.datetime.now(datetime.timezone.utc)


# ---------------------------------------------------------------- errors


class StopError(Exception):
    """A condition that stops a script before it does any work."""


class Unreadable(Exception):
    """A file that the reader cannot read. `line` is the line in the file,
    or None."""

    def __init__(self, message, line=None):
        Exception.__init__(self, message)
        self.message = message
        self.line = line

    def describe(self, file_name):
        if self.line is None:
            return "%s: %s" % (file_name, self.message)
        return "%s, line %d: %s" % (file_name, self.line, self.message)


# ---------------------------------------------------------------- storage folder


def storage_root(path):
    """The absolute storage folder at `path`. Stops when the folder has no
    library marker (`.longhand/library`)."""
    root = os.path.abspath(path)
    marker = os.path.join(root, LONGHAND_FOLDER, LIBRARY_MARKER)
    if not os.path.isfile(marker):
        raise StopError(
            "%s is not a Longhand storage folder: it has no %s/%s file. "
            "Start the agent in the storage folder, or give its path with --root."
            % (root, LONGHAND_FOLDER, LIBRARY_MARKER)
        )
    return root


def is_hidden(name):
    return name.startswith(".")


def is_note_folder(path):
    return os.path.isfile(os.path.join(path, SESSION_FILE)) or os.path.isfile(
        os.path.join(path, NOTE_RECORD_FILE)
    )


def note_folders(root):
    """Each note folder of the storage folder as (relative path with `/`,
    note type or None). A note folder is directly in the storage folder
    (General) or in a type folder. Hidden folders are not read."""
    out = []
    for name in sorted(os.listdir(root)):
        path = os.path.join(root, name)
        if is_hidden(name) or not os.path.isdir(path):
            continue
        if is_note_folder(path):
            out.append((name, None))
            continue
        for child in sorted(os.listdir(path)):
            child_path = os.path.join(path, child)
            if is_hidden(child) or not os.path.isdir(child_path):
                continue
            if is_note_folder(child_path):
                out.append((name + "/" + child, name))
    return out


def type_folders(root):
    """The names of the folders in the storage folder that hold notes."""
    return sorted({note_type for _, note_type in note_folders(root) if note_type})


def native(root, relative):
    return os.path.join(root, *relative.split("/"))


def read_bytes(path):
    with open(path, "rb") as handle:
        return handle.read()


def read_text(path):
    """The text of a file as UTF-8, with its line ends as they are."""
    with open(path, "r", encoding="utf-8", newline="") as handle:
        return handle.read()


def digest(path, relative):
    """A value that changes when the file changes. `relative` is the path
    in the note folder. Audio files are not opened, because a sync service
    can keep them online only."""
    if relative.split("/")[0] == AUDIO_FOLDER:
        info = os.stat(path)
        return "stat:%d:%d" % (info.st_size, info.st_mtime_ns)
    return hashlib.sha256(read_bytes(path)).hexdigest()


def folder_files(folder):
    """Each file in `folder` and its folders, by its path relative to
    `folder` with `/`."""
    out = []
    for current, dirs, files in os.walk(folder):
        dirs.sort()
        relative_dir = os.path.relpath(current, folder)
        for name in sorted(files):
            if relative_dir == ".":
                out.append(name)
            else:
                out.append(relative_dir.replace(os.sep, "/") + "/" + name)
    return out


def matches(relative, patterns):
    return any(fnmatch.fnmatchcase(relative, pattern) for pattern in patterns)


# ---------------------------------------------------------------- longhand.json


def read_json_object(path, file_name):
    """The JSON object in the file at `path`: (object or None, state). The
    state is "ok", or a text that tells why the file cannot be used."""
    try:
        text = read_text(path)
    except (OSError, UnicodeDecodeError) as error:
        return None, "%s cannot be read: %s" % (file_name, error)
    try:
        value = json.loads(text)
    except ValueError as error:
        line = getattr(error, "lineno", None)
        return None, Unreadable("does not parse as JSON", line).describe(file_name)
    if not isinstance(value, dict):
        return None, "%s is not a JSON object" % file_name
    return value, "ok"


def read_record(folder):
    """The records of a note folder: (record or None, state). The state is
    "ok", or a text that tells why the records cannot be used."""
    path = os.path.join(folder, NOTE_RECORD_FILE)
    if not os.path.isfile(path):
        return None, "no %s yet: the sync has not delivered it" % NOTE_RECORD_FILE
    record, state = read_json_object(path, NOTE_RECORD_FILE)
    if record is None:
        return None, state
    layout = record.get("layout")
    if not isinstance(layout, int) or isinstance(layout, bool):
        return None, "%s has no layout number" % NOTE_RECORD_FILE
    if layout not in RECORD_KEYS:
        return record, (
            "%s has layout %d, and this skill knows layouts %s. Update the Longhand plugin."
            % (NOTE_RECORD_FILE, layout, " and ".join(str(known) for known in sorted(RECORD_KEYS)))
        )
    unknown = sorted(set(record) - RECORD_KEYS[layout])
    if unknown:
        return record, (
            "%s has keys that this skill does not know: %s. Update the Longhand plugin."
            % (NOTE_RECORD_FILE, ", ".join(unknown))
        )
    if record_id(record) is None or parse_time(record.get("started")) is None:
        return record, "%s has no valid note id or started time" % NOTE_RECORD_FILE
    return record, "ok"


def record_id(record):
    """The note id in a `longhand.json`, or None. A known layout gives the
    key. For a layout that this skill does not know, the id is in the key
    of a known layout, so that a check can find the note after a move."""
    layout = record.get("layout")
    if isinstance(layout, int) and not isinstance(layout, bool) and layout in RECORD_ID_KEYS:
        keys = [RECORD_ID_KEYS[layout]]
    else:
        keys = [RECORD_ID_KEYS[known] for known in sorted(RECORD_ID_KEYS, reverse=True)]
    for key in keys:
        if isinstance(record.get(key), str):
            return record[key]
    return None


# ---------------------------------------------------------------- frontmatter

_KEY = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*)[ \t]*:(?:[ \t]+(.*)|)$")
_ITEM = re.compile(r"^([ \t]*)-(?:[ \t]+(.*)|)$")
_NOT_TEXT = re.compile(
    r"^(?:~|null|Null|NULL|true|True|TRUE|false|False|FALSE"
    r"|[-+]?(?:\d[\d_]*)(?:\.\d*)?(?:[eE][-+]?\d+)?"
    r"|[-+]?\.\d+(?:[eE][-+]?\d+)?"
    r"|0x[0-9a-fA-F_]+|0o[0-7_]+|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$"
)
_INDICATORS = "[]{}&*!|>%@`#,"


class Scalar(object):
    """One frontmatter value as the reader found it."""

    def __init__(self, text, quoted):
        self.text = text
        self.quoted = quoted

    def is_text(self):
        """A plain value that YAML reads as a number, a boolean or null is
        not text."""
        return self.quoted or not _NOT_TEXT.match(self.text)


def _strip_comment(value):
    at = value.find(" #")
    if at < 0:
        at = value.find("\t#")
    return value if at < 0 else value[:at]


def _scalar(value, line):
    """One YAML scalar on one line: plain, single-quoted or double-quoted."""
    value = value.strip()
    if value.startswith('"'):
        end = _closing_double_quote(value)
        if end < 0:
            raise Unreadable("a double-quoted value has no closing quote", line)
        rest = _strip_comment(" " + value[end + 1:]).strip()
        if rest:
            raise Unreadable("text after a quoted value", line)
        try:
            text = json.loads(value[: end + 1])
        except ValueError:
            raise Unreadable("a double-quoted value that this reader cannot read", line)
        return Scalar(text, True)
    if value.startswith("'"):
        at = 1
        out = []
        while True:
            end = value.find("'", at)
            if end < 0:
                raise Unreadable("a single-quoted value has no closing quote", line)
            out.append(value[at:end])
            if value[end + 1:end + 2] == "'":
                out.append("'")
                at = end + 2
                continue
            rest = _strip_comment(" " + value[end + 1:]).strip()
            if rest:
                raise Unreadable("text after a quoted value", line)
            return Scalar("".join(out), True)
    value = _strip_comment(value).strip()
    if value and (value[0] in _INDICATORS or value.startswith("- ") or value == "-"):
        raise Unreadable("a value form that this reader cannot read", line)
    if value.startswith("? ") or ": " in value or value.endswith(":"):
        raise Unreadable("a value with ': ' in it must be in quotes", line)
    return Scalar(value, False)


def _closing_double_quote(value):
    at = 1
    while at < len(value):
        if value[at] == "\\":
            at += 2
            continue
        if value[at] == '"':
            return at
        at += 1
    return -1


def _flow_list(value, line):
    """A YAML flow list of scalars on one line, such as `[a, "b c"]`."""
    value = _strip_comment(value).strip()
    if not value.endswith("]"):
        raise Unreadable("a list in [ ] has no closing bracket", line)
    inner = value[1:-1]
    items = []
    current = []
    quote = None
    at = 0
    while at < len(inner):
        c = inner[at]
        if quote:
            current.append(c)
            if quote == '"' and c == "\\":
                current.append(inner[at + 1:at + 2])
                at += 2
                continue
            if c == quote:
                if quote == "'" and inner[at + 1:at + 2] == "'":
                    current.append("'")
                    at += 2
                    continue
                quote = None
        elif c in "\"'" and not "".join(current).strip():
            quote = c
            current.append(c)
        elif c == ",":
            items.append("".join(current))
            current = []
        elif c in "[]{}":
            raise Unreadable("a list in [ ] inside a list", line)
        else:
            current.append(c)
        at += 1
    if quote:
        raise Unreadable("a quoted value has no closing quote", line)
    tail = "".join(current)
    if tail.strip() or items:
        items.append(tail)
    out = []
    for item in items:
        if not item.strip():
            if len(items) == 1:
                break
            raise Unreadable("an empty item in a list in [ ]", line)
        out.append(_scalar(item, line))
    return out


def frontmatter_bounds(text):
    """(start, end, body start) of the frontmatter in `text`, or None. This
    follows `frontmatter_bounds` in `crates/session-core/src/folder.rs`."""
    if text.startswith("---\n"):
        start = 4
    elif text.startswith("---\r\n"):
        start = 5
    else:
        return None
    at = start
    for line in split_inclusive(text[start:]):
        if line.rstrip("\r\n") == "---":
            return start, at, at + len(line)
        at += len(line)
    return None


def split_inclusive(text):
    """The lines of `text`, each with its LF, as Rust's
    `split_inclusive('\\n')` gives them. Only LF ends a line."""
    lines = text.split("\n")
    out = [line + "\n" for line in lines[:-1]]
    if lines[-1]:
        out.append(lines[-1])
    return out


def read_frontmatter(text):
    """The keys of a YAML frontmatter: a dict of key to Scalar, or to a list
    of Scalar. Raises Unreadable for a form that this reader cannot read,
    with the line in the file."""
    bounds = frontmatter_bounds(text)
    if bounds is None:
        raise Unreadable("the file has no frontmatter between two --- lines", 1)
    start, end, _ = bounds
    lines = [line.rstrip("\r") for line in text[start:end].split("\n")]
    fields = {}
    index = 0
    while index < len(lines):
        line_no = index + 2
        line = lines[index]
        index += 1
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if line[0] in " \t":
            raise Unreadable("an indented line that this reader cannot read", line_no)
        match = _KEY.match(line)
        if not match:
            raise Unreadable("a line that is not 'key: value'", line_no)
        key, value = match.group(1), match.group(2)
        if key in fields:
            raise Unreadable("the key %s is in the frontmatter two times" % key, line_no)
        value = "" if value is None else value
        if _strip_comment(" " + value).strip() == "":
            items = []
            while index < len(lines):
                item_line = lines[index]
                if not item_line.strip() or item_line.strip().startswith("#"):
                    index += 1
                    continue
                item = _ITEM.match(item_line)
                if not item:
                    if item_line[0] in " \t":
                        raise Unreadable("an indented line that this reader cannot read", index + 2)
                    break
                item_value = item.group(2) or ""
                if not _strip_comment(" " + item_value).strip():
                    raise Unreadable("an empty list item", index + 2)
                if item_value.strip().startswith("- "):
                    raise Unreadable("a list inside a list", index + 2)
                items.append(_scalar(item_value, index + 2))
                index += 1
            fields[key] = items if items else None
        elif value.strip().startswith("["):
            fields[key] = _flow_list(value, line_no)
        else:
            fields[key] = _scalar(value, line_no)
    return fields


def read_session_md(text):
    """(title or None, tags) of a `session.md`. Raises Unreadable."""
    if text.startswith(BYTE_ORDER_MARK):
        # `SessionMd::parse` in crates/session-core does not remove it.
        raise Unreadable("starts with a byte order mark, which Longhand cannot read. Save it without one.", None)
    fields = read_frontmatter(text)
    if "session_id" in fields:
        raise Unreadable(
            "the frontmatter holds session_id, which this skill does not know. "
            "Open Longhand and follow its notice.",
            None,
        )
    title = fields.get("title")
    if isinstance(title, list):
        raise Unreadable("title is a list", None)
    if title is not None and not title.is_text():
        raise Unreadable("title %s is not text: put it in quotes" % title.text, None)
    tags_value = fields.get("longhand_tags")
    if tags_value is None:
        tags = []
    elif isinstance(tags_value, list):
        tags = tags_value
    else:
        tags = [tags_value]
    for tag in tags:
        if not tag.is_text():
            raise Unreadable("tag %s is not text: put it in quotes" % tag.text, None)
    return (title.text if title is not None else None), [tag.text for tag in tags]


# ---------------------------------------------------------------- in-use markers


def fresh_markers(folder, at=None):
    """The in-use markers in `folder` that are fresh at `at`."""
    at = at or now()
    out = []
    try:
        names = sorted(os.listdir(folder))
    except OSError:
        return out
    for name in names:
        if not name.startswith(IN_USE_PREFIX) or len(name) == len(IN_USE_PREFIX):
            continue
        path = os.path.join(folder, name)
        try:
            refreshed = parse_time(json.loads(read_text(path)).get("refreshed_at"))
        except (OSError, ValueError, AttributeError, UnicodeDecodeError):
            refreshed = None
        if refreshed is not None and refreshed <= at + CLOCK_MARGIN:
            if at - refreshed < FRESH_FOR:
                out.append(name)
            continue
        try:
            modified = datetime.datetime.fromtimestamp(
                os.stat(path).st_mtime, datetime.timezone.utc
            )
        except OSError:
            continue
        if at - modified < FRESH_FOR:
            out.append(name)
    return out


# ---------------------------------------------------------------- notes


def read_note(root, relative, note_type, at=None):
    """What a check and a list need to know about one note folder."""
    folder = native(root, relative)
    record, record_state = read_record(folder)
    note = {
        "path": relative,
        "type": note_type,
        "id": None,
        "started": None,
        "record": record_state,
        "title": None,
        "tags": [],
        "session_md": "ok",
        "session_md_bom": False,
        "busy": [],
    }
    if record is not None:
        note["id"] = record_id(record)
    if record is not None and isinstance(record.get("started"), str):
        note["started"] = record["started"]
    session_path = os.path.join(folder, SESSION_FILE)
    if os.path.isfile(session_path):
        try:
            text = read_text(session_path)
            note["session_md_bom"] = text.startswith(BYTE_ORDER_MARK)
            note["title"], note["tags"] = read_session_md(text)
        except Unreadable as error:
            note["session_md"] = error.describe(SESSION_FILE)
        except (OSError, UnicodeDecodeError) as error:
            note["session_md"] = "%s cannot be read: %s" % (SESSION_FILE, error)
    else:
        note["session_md"] = "the folder has no %s" % SESSION_FILE
    markers = fresh_markers(folder, at)
    if markers:
        note["busy"].append("a computer records or processes it (in-use marker %s)" % markers[0])
    if record is not None:
        if record.get("result") == "pending":
            note["busy"].append("its recording is not saved yet (result: pending)")
        if record.get("pending_type"):
            note["busy"].append("a move to another type waits (pending_type)")
    return note


def unusable_reasons(note):
    """Why an agent must not edit `note` now. An empty list when it can."""
    reasons = list(note["busy"])
    if note["record"] != "ok":
        reasons.append(note["record"])
    if note["session_md"] != "ok":
        reasons.append(note["session_md"])
    return reasons


def notes(root, at=None):
    return [read_note(root, relative, note_type, at) for relative, note_type in note_folders(root)]


# ---------------------------------------------------------------- prompts


def file_name_id(file_name):
    """The id that Longhand gives a prompt file with no id."""
    return str(uuid.uuid5(PROMPT_ID_NAMESPACE, "file:" + file_name))


def _split_body(body):
    start = 0
    for line in split_inclusive(body):
        if line.strip() == TITLE_MARKER:
            return _strip_final_newline(body[:start]), _strip_final_newline(body[start + len(line):])
        start += len(line)
    return _strip_final_newline(body), ""


def _strip_final_newline(text):
    if text.endswith("\r\n"):
        return text[:-2]
    if text.endswith("\n"):
        return text[:-1]
    return text


def _front_matter_value(value):
    value = value.strip()
    if value.startswith('"'):
        try:
            text = json.loads(value)
            if isinstance(text, str):
                return text
        except ValueError:
            # A value in quotes that is not valid JSON is plain text.
            pass
    return value


def parse_prompt(text):
    """The parts of a prompt file, as `parse_prompt_file` in
    `crates/shared-settings/src/prompts.rs` reads them: a dict with id,
    name, text, title_text and open."""
    if text.startswith(BYTE_ORDER_MARK):
        text = text[1:]
    parsed = {"id": None, "name": None, "text": "", "title_text": "", "open": False}
    lines = split_inclusive(text)
    if not lines or lines[0].rstrip() != "---":
        parsed["text"], parsed["title_text"] = _split_body(text)
        return parsed
    consumed = len(lines[0])
    for line in lines[1:]:
        consumed += len(line)
        bare = line.rstrip("\r\n")
        if bare.rstrip() == "---":
            body = text[consumed:]
            if body.startswith("\r\n"):
                body = body[2:]
            elif body.startswith("\n"):
                body = body[1:]
            parsed["text"], parsed["title_text"] = _split_body(body)
            return parsed
        key, sep, value = bare.partition(":")
        if not sep:
            continue
        if key.strip() == "id":
            parsed["id"] = _front_matter_value(value) or None
        elif key.strip() == "name":
            name = _front_matter_value(value)
            parsed["name"] = name if name.strip() else None
    parsed["open"] = True
    return parsed


def prompt_file_names(root):
    """The file names of the prompt files: `.md` files whose names do not
    start with `.` or `~$`."""
    folder = os.path.join(root, LONGHAND_FOLDER, PROMPTS_FOLDER)
    if not os.path.isdir(folder):
        return []
    return sorted(
        name
        for name in os.listdir(folder)
        if name.lower().endswith(".md")
        and not name.startswith(".")
        and not name.startswith("~$")
        and os.path.isfile(os.path.join(folder, name))
    )


def read_prompts(root):
    """Each prompt file by its file name: id (the id in the file, or the
    id that Longhand makes from the file name), id_in_file, name, size,
    open, over_limit, empty and a hash of its texts."""
    folder = os.path.join(root, LONGHAND_FOLDER, PROMPTS_FOLDER)
    out = {}
    for name in prompt_file_names(root):
        path = os.path.join(folder, name)
        size = os.path.getsize(path)
        info = {"size": size, "too_large": size > PROMPT_MAX_FILE_BYTES}
        try:
            parsed = parse_prompt(read_text(path))
        except (OSError, UnicodeDecodeError) as error:
            info.update({"unreadable": str(error), "id": file_name_id(name), "id_in_file": None,
                         "name": None, "open": False, "over_limit": False, "empty": False, "texts": None})
            out[name] = info
            continue
        info.update(
            {
                "unreadable": None,
                "id_in_file": parsed["id"],
                "id": parsed["id"] or file_name_id(name),
                "name": parsed["name"],
                "open": parsed["open"],
                "over_limit": info["too_large"]
                or len(parsed["text"]) > PROMPT_TEXT_LIMIT
                or len(parsed["title_text"]) > TITLE_TEXT_LIMIT,
                # `read_entry` in prompts.rs does not use a prompt whose
                # trimmed text is empty.
                "empty": not parsed["open"]
                and not info["too_large"]
                and all(char in WHITE_SPACE for char in parsed["text"]),
                "texts": hashlib.sha256(
                    (parsed["text"] + "\0" + parsed["title_text"]).encode("utf-8")
                ).hexdigest(),
            }
        )
        out[name] = info
    return out


_PROMPT_REFUSED = '<>:"/\\|?*'
_PROMPT_RESERVED = RESERVED_NAMES[:4] + ("CONIN$", "CONOUT$") + RESERVED_NAMES[4:]
_MAX_STEM_BYTES = 197


def safe_file_name(name):
    """The file name of a prompt named `name`, as `safe_file_name` in
    `crates/shared-settings/src/prompts.rs` makes it."""
    replaced = "".join("-" if is_control(c) or c in _PROMPT_REFUSED else c for c in name.strip())
    stem = replaced.lstrip(". ")
    if stem.startswith("~$"):
        stem = "~-" + stem[2:]
    encoded = stem.encode("utf-8")[:_MAX_STEM_BYTES]
    stem = encoded.decode("utf-8", "ignore").rstrip(". ")
    if not stem:
        stem = "Prompt"
    first = stem.split(".")[0].rstrip()
    if any(first.upper() == reserved for reserved in _PROMPT_RESERVED):
        stem = "_" + stem
    return stem + ".md"


# ---------------------------------------------------------------- output


def print_lines(lines, stream=None):
    stream = stream or sys.stdout
    for line in lines:
        stream.write(line + "\n")


def run(main):
    """Run `main`. A StopError gives exit code 2: the script did no work."""
    try:
        return main()
    except StopError as error:
        sys.stderr.write("Stopped: %s\n" % error)
        return 2
