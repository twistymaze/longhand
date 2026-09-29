import { useState } from "react";
import { commands, type DestinationStatus, type Result, type Settings } from "../bindings";
import { Button, ErrorText, Group, Pane, Row, Status, type Tone, folderName, formatBytes } from "./ui";

export function describeDestination(status: DestinationStatus): { tone: Tone; text: string } {
  switch (status.kind) {
    case "ready":
      return {
        tone: "positive",
        text: status.free_bytes === null ? "Ready" : `Ready · ${formatBytes(status.free_bytes)} free`,
      };
    case "missing":
      return { tone: "negative", text: "Folder not found — it may be on a disconnected drive" };
    case "not_a_directory":
      return { tone: "negative", text: "This path is a file, not a folder" };
    case "not_writable":
      return { tone: "negative", text: `Longhand can't write here: ${status.message}` };
    case "low_space":
      return { tone: "caution", text: `Only ${formatBytes(status.free_bytes)} free` };
  }
}

/// Pick a folder and make it the storage folder. The notes in the old
/// folder stay there, and the library stops listing them. `null` data when
/// the user closes the dialog. (U9 adds the Move / Leave them choice.)
export async function changeToPickedFolder(): Promise<Result<Settings | null, string>> {
  const picked = await commands.pickStorageFolder();
  if (picked.status === "error") return picked;
  if (picked.data === null) return { status: "ok", data: null };
  return commands.changeStorageFolder(picked.data, "leave");
}

export function DestinationPicker({
  settings,
  status,
  onSettings,
}: {
  settings: Settings;
  status: DestinationStatus;
  onSettings: (s: Settings) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const current = settings.storage_folder;
  const described = describeDestination(status);

  const choose = async () => {
    const result = await changeToPickedFolder();
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      if (result.data) onSettings(result.data);
    }
  };

  return (
    <Pane title="Storage">
      <Group
        header="Notes are saved to"
        footer="When you change the folder, the notes in the old folder stay there, and Longhand no longer lists them."
      >
        <Row label={folderName(current)} detail={current}>
          <Button onClick={choose}>Change…</Button>
        </Row>
        <Row label={<Status tone={described.tone}>{described.text}</Status>} />
        {error && <ErrorText>{error}</ErrorText>}
      </Group>
    </Pane>
  );
}
