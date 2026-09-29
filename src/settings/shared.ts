// The state of the shared settings for the Settings panes (R16). While the
// storage folder is not available, Longhand uses the last shared settings
// that it read, and the panes show them read-only.

import type { ReadOnlyCause, Settings } from "../bindings";

const causeText: Record<ReadOnlyCause, string> = {
  no_storage_folder: "No storage folder is set.",
  not_read_yet: "Longhand is still reading the storage folder.",
  folder_not_available: "The storage folder is not available.",
  not_readable: "Longhand can’t read the storage folder.",
  does_not_parse: "The settings in the storage folder are not complete. A sync can still be in progress.",
  no_settings_file: "The storage folder has no settings yet.",
};

/// Why the shared settings are read-only now, as one sentence, or null
/// when this computer can change them.
export function readOnlyReason(settings: Settings): string | null {
  const availability = settings.shared.availability;
  return availability.kind === "read_only" ? causeText[availability.cause] : null;
}
