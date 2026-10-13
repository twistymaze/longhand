// The state of the shared settings for the Settings panes (R16). While the
// storage folder is not available, Longhand uses the last shared settings
// that it read, and the panes show them read-only.

import type { DamagedFile, ReadOnlyCause, Settings } from "../bindings";

const causeText: Record<ReadOnlyCause, string> = {
  no_storage_folder: "No storage folder is set.",
  not_read_yet: "Longhand is still reading the storage folder.",
  folder_not_available: "The storage folder is not available.",
  not_readable: "Longhand can’t read the storage folder.",
  does_not_parse: "The settings in the storage folder are not complete. A sync can still be in progress.",
  no_settings_file: "The storage folder has no settings yet.",
  settings_file_damaged: "The settings file in the storage folder has an error.",
};

/// Why the shared settings are read-only now, as one sentence, or null
/// when this computer can change them.
export function readOnlyReason(settings: Settings): string | null {
  const availability = settings.shared.availability;
  return availability.kind === "read_only" ? causeText[availability.cause] : null;
}

/// Why the prompts and the Background can't change now, or null. While only
/// the settings file does not parse, they can, because they are in other
/// files (R22).
export function filesReadOnlyReason(settings: Settings): string | null {
  return settings.shared.files_writable ? null : readOnlyReason(settings);
}

/// The notice for a settings file that did not parse for about one minute
/// (R22): the file, the line, the settings that can't change, and what to
/// do.
export function damagedFileText(file: DamagedFile): string {
  const where = file.line === null ? file.path : `${file.path}, line ${file.line}`;
  return (
    `Longhand can’t read the settings file in the storage folder (${where}). ` +
    "Until the file is fixed, these settings can’t be changed: the default prompt, the note types, the default type, the prompt of each type, " +
    "the transcription choices, update checks, audio removal, the recording widget and New Note. " +
    "Fix the file, or restore it from another computer."
  );
}
