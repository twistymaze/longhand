// Settings > Storage (R3, R4, R6, R10, R16): the one storage folder, Show
// in Finder, Change, the Move of the notes to a new folder, and the removal
// of old audio. Change is not available while a Move runs or is paused
// (KTD11).

import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  events,
  type DestinationStatus,
  type MovePauseCause,
  type MoveState,
  type Settings,
} from "../bindings";
import { AudioRemoval } from "./AudioRemoval";
import { useFolderChange } from "./OldNotesDialog";
import { readOnlyReason } from "./shared";
import { Button, ErrorText, Group, Pane, ProgressBar, Row, Status, type Tone, folderName, formatBytes, notes } from "./ui";

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

// Read the platform on each render, so that tests can set the user agent.
const isMac = () => navigator.userAgent.includes("Mac");

/// The Move as the pane shows it. `folder` is the new folder, or null
/// until the pane reads it.
type Move = { folder: string | null; moved: number; total: number; state: MoveState; cause: MovePauseCause | null };

/// The open Move, from `move_status` when the pane opens and from the Move
/// events after that. `finished` is the result of the last Move that ended
/// while the pane was open.
function useMove() {
  const [move, setMoveState] = useState<Move | null>(null);
  const [finished, setFinished] = useState<{ moved: number; folder: string | null } | null>(null);
  // The events read the last Move that the pane shows.
  const current = useRef<Move | null>(null);
  const setMove = useCallback((next: Move | null) => {
    current.current = next;
    setMoveState(next);
  }, []);
  const read = useCallback(async () => {
    const status = await commands.moveStatus();
    // A finished Move that the events already reported stays finished.
    if (!status && !current.current) return;
    setMove(
      status && {
        folder: status.new_folder,
        moved: status.moved,
        total: status.total,
        state: status.state,
        cause: status.cause,
      },
    );
    if (status) setFinished(null);
  }, [setMove]);
  useEffect(() => {
    void read();
    const progress = events.moveProgress.listen(({ payload }) => {
      const m = current.current;
      if (payload.state === "finished") {
        setFinished({ moved: payload.moved, folder: m?.folder ?? null });
        setMove(null);
        return;
      }
      setFinished(null);
      setMove({
        folder: m?.folder ?? null,
        cause: payload.state === "paused" ? (m?.cause ?? null) : null,
        moved: payload.moved,
        total: payload.total,
        state: payload.state,
      });
      // A Move that started in another window: read its folders.
      if (!m) void read();
    });
    const paused = events.movePaused.listen(({ payload }) => {
      const m = current.current;
      if (m) setMove({ ...m, state: "paused", cause: payload.cause });
      else void read();
    });
    return () => {
      void progress.then((f) => f());
      void paused.then((f) => f());
    };
  }, [read]);
  return { move, finished, read };
}

function MoveGroup({ move, onRetry }: { move: Move; onRetry: () => void }) {
  const to = move.folder ? ` to “${folderName(move.folder)}”` : "";
  const count = `${move.moved} of ${notes(move.total)} moved`;
  const paused = move.state === "paused";
  return (
    <Group
      header="Move"
      footer="You can record and edit notes while they move. You can change the folder again when the move is done."
    >
      {paused && (
        <Row label={<Status tone="caution">Move paused</Status>} detail={move.cause?.message}>
          <Button prominent onClick={onRetry}>
            Retry
          </Button>
        </Row>
      )}
      <Row
        label={`Moving notes${to}`}
        detail={move.state === "waiting" ? "The move waits for a note that is in use." : undefined}
      >
        <span role="status" className="text-label-secondary tabular-nums">
          {count}
        </span>
        <ProgressBar fraction={move.total ? move.moved / move.total : 0} />
      </Row>
    </Group>
  );
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
  const { change, dialog } = useFolderChange();
  const { move, finished, read } = useMove();
  const [busy, setBusy] = useState(false);
  const current = settings.storage_folder;
  const described = describeDestination(status);
  const readOnly = readOnlyReason(settings);
  const available =
    status.kind !== "missing" &&
    status.kind !== "not_a_directory" &&
    !(settings.shared.availability.kind === "read_only" && settings.shared.availability.cause === "folder_not_available");

  const choose = async () => {
    setBusy(true);
    try {
      const result = await change();
      if (result.status === "error") setError(result.error);
      else {
        setError(null);
        if (result.data) {
          onSettings(result.data.settings);
          if (result.data.moved) await read();
        }
      }
    } finally {
      setBusy(false);
    }
  };

  const showFolder = async () => {
    const result = await commands.showStorageFolder();
    setError(result.status === "error" ? result.error : null);
  };

  const retry = async () => {
    const result = await commands.retryMove();
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      await read();
    }
  };

  return (
    <Pane title="Storage">
      <Group
        header="Notes are saved to"
        footer="Summary prompts and transcription choices are saved in this folder too. Every computer that uses this folder has the same settings."
      >
        <Row label={folderName(current)} detail={current}>
          <Button disabled={!available} onClick={() => void showFolder()}>
            {isMac() ? "Show in Finder" : "Show in Explorer"}
          </Button>
          <Button disabled={busy || move !== null} onClick={() => void choose()}>
            Change…
          </Button>
        </Row>
        {readOnly ? (
          <Row
            label={<Status tone="caution">{readOnly}</Status>}
            detail="Longhand uses the last settings it read. Summary prompts and transcription choices can’t be changed until the folder is available."
          />
        ) : (
          <Row label={<Status tone={described.tone}>{described.text}</Status>} />
        )}
        {finished && (
          <Row
            label={
              <Status tone="positive">
                Moved {notes(finished.moved)}
                {finished.folder ? ` to “${folderName(finished.folder)}”` : ""}.
              </Status>
            }
          />
        )}
        {error && <ErrorText>{error}</ErrorText>}
      </Group>
      {move && <MoveGroup move={move} onRetry={() => void retry()} />}
      <AudioRemoval settings={settings} onSettings={onSettings} />
      {dialog}
    </Pane>
  );
}
