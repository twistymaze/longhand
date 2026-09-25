import { useState } from "react";
import { commands, type DestinationStatus, type Settings } from "../bindings";
import { Button, ErrorText, Group, Pane, Row, Status, type Tone, folderName, formatBytes } from "./ui";

export function describeDestination(status: DestinationStatus): { tone: Tone; text: string } {
  switch (status.kind) {
    case "unset":
      return { tone: "caution", text: "No folder chosen" };
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
  const current = settings.current_destination;
  const described = describeDestination(status);

  const choose = async () => {
    const result = await commands.chooseDestination();
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      if (result.data) onSettings(result.data);
    }
  };
  const use = async (folder: string) => {
    const result = await commands.setCurrentDestination(folder);
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      onSettings(result.data);
    }
  };

  const earlier = settings.destination_roots.filter((root) => root !== current);

  return (
    <Pane title="Storage">
      <Group
        header="Notes are saved to"
        footer="You can change this at any time, including while recording — a note is saved wherever this points when it finishes processing."
      >
        <Row
          label={current ? folderName(current) : "No folder chosen"}
          detail={current ?? "Choose a folder before the shortcut can start a recording."}
        >
          <Button prominent={!current} onClick={choose}>
            {current ? "Change…" : "Choose Folder…"}
          </Button>
        </Row>
        <Row label={<Status tone={described.tone}>{described.text}</Status>} />
        {error && <ErrorText>{error}</ErrorText>}
      </Group>

      {earlier.length > 0 && (
        <Group
          header="Earlier folders"
          footer="Every folder you have chosen is remembered, so notes already saved there stay findable."
        >
          {earlier.map((root) => (
            <Row key={root} label={folderName(root)} detail={root}>
              <Button onClick={() => use(root)}>Use</Button>
            </Row>
          ))}
        </Group>
      )}
    </Pane>
  );
}
