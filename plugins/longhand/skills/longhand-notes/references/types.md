# Note types

A note has one note type. General is the built-in type: a note of General is directly in the storage folder. Each other type is a type folder in the storage folder, with the name of the type. The position of the note folder decides the type.

The shared settings also have a list of the types (`note_types` in `settings.json`), in the order that Settings shows them. The effective type list is that list plus each type folder that holds a note. Also see `settings.md` for `default_note_type` and `note_type_prompts`.

## Type names

A type name:

- is not empty, and is not "General" in any case,
- does not start or end with a dot,
- has no control characters and none of `" * : < > ? / \ |`,
- is not a name that Windows uses for its own files (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`),
- has at most 120 characters,
- is not the name of a note folder.

Two type names that differ only in case or in spaces at the ends are one type.

## Change the type of a note

1. Make sure that the note is not busy, and make a snapshot.
2. Move the note folder into the type folder. Make the type folder first if it is not there. To change a note to General, move the note folder into the storage folder.
3. Keep the name of the note folder. If the type folder has a folder with that name, stop and tell the user.
4. Compare. `check.py` reads the move as a move of the same note.

## Add a type

1. Make a snapshot.
2. Add the name to `note_types` in `settings.json`.
3. Compare.

Longhand makes the type folder when the first note gets the type. You can also make it with the first move.

## Rename a type

1. Make sure that no note of the type is busy, and make a snapshot.
2. Rename the type folder.
3. In `settings.json`, change the name in `note_types`, the key in `note_type_prompts`, and `default_note_type` when it names the type.
4. Compare.

## Remove a type

1. Make sure that no note of the type is busy, and make a snapshot.
2. Move each note of the type to another type or to General. Then delete the empty type folder.
3. In `settings.json`, remove the name from `note_types` and its key from `note_type_prompts`. If `default_note_type` names the type, set it to `null` (General).
4. Compare.
