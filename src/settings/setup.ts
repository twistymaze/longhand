// The setup items that are still outstanding (R2, R3, R5, KTD3, KTD4). The
// General setup list, its readiness row and the sidebar marks read this one
// function, so they cannot disagree.

import type { Readiness, Settings } from "../bindings";
import { readOnlyReason } from "./shared";

export type SetupItemId = "microphone" | "shortcut" | "storage" | "system_audio" | "models" | "provider";

export type SetupItem = {
  id: SetupItemId;
  /// The item stops a recording. The other items only delay saving or
  /// processing.
  blocksRecording: boolean;
};

/// The outstanding items, in the order that the General list shows them:
/// the recording blockers first, then the other items in sidebar order.
/// `systemAudio` is the Computer audio feature switch.
export function outstandingItems(
  readiness: Readiness,
  settings: Settings,
  { systemAudio }: { systemAudio: boolean },
): SetupItem[] {
  const items: SetupItem[] = [];
  if (!readiness.microphone_authorized) items.push({ id: "microphone", blocksRecording: true });
  // A cleared shortcut is a choice, not a problem (R13 of the shortcut work).
  if (readiness.shortcut.chord && !readiness.shortcut.registered) items.push({ id: "shortcut", blocksRecording: true });
  if (readiness.destination.kind !== "ready" || readOnlyReason(settings) !== null) {
    items.push({ id: "storage", blocksRecording: false });
  }
  // The microphone alone is enough to record. Computer audio access is
  // needed only when new notes start with Computer audio on.
  if (systemAudio && settings.system_audio_default && readiness.system_audio !== "allowed") {
    items.push({ id: "system_audio", blocksRecording: false });
  }
  if (readiness.missing_models.length > 0) items.push({ id: "models", blocksRecording: false });
  if (!readiness.endpoint_configured) items.push({ id: "provider", blocksRecording: false });
  return items;
}
