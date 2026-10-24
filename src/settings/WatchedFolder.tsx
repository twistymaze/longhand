// Settings > Storage > Watched folder (R4–R6, R9, R10): a folder on this
// computer where each new audio file becomes a note. The user chooses it
// with the system folder dialog. The pane reads the folder first: a folder
// that Longhand refuses gives the reason, and a folder that holds audio
// files that are not imported gets one question (F3). Normal watching
// shows no message. A folder that stays unavailable shows its state here.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type ExistingFiles, type Settings, type WatchedFolderPreview, type WatchedFolderStatus } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { Button, ErrorText, Group, Row, Status, folderName, formatBytes } from "./ui";

type Answer = ExistingFiles | "cancel";

const audioFiles = (n: number) => `${n} audio ${n === 1 ? "file" : "files"}`;

/// The question about the audio files that are in the folder (R6). Only
/// New Files has the focus: an old archive can start hours of processing.
/// A storage folder change with Leave Them asks it too (U7).
export function ExistingFilesDialog({
  folder,
  preview,
  onAnswer,
}: {
  folder: string;
  preview: WatchedFolderPreview;
  onAnswer: (answer: Answer) => void;
}) {
  // Order in the row: Cancel, Import Them, Only New Files.
  return (
    <ConfirmDialog
      title={`Import the recordings in “${folderName(folder)}”?`}
      message={
        <>
          <p>
            This folder has {audioFiles(preview.count)} ({formatBytes(preview.bytes)}) that Longhand has not imported.
          </p>
          <p>Import Them makes a note from each file. Only New Files makes notes only from files added from now on.</p>
        </>
      }
      action="Only New Files"
      destructive={false}
      secondary={{ label: "Import Them", onClick: () => onAnswer("import") }}
      focus="confirm"
      width="w-[22rem]"
      onConfirm={() => onAnswer("skip")}
      onCancel={() => onAnswer("cancel")}
    />
  );
}

/// The status of the watched folder: read when the pane opens, then from
/// the events. A read that started before an event is older than the event.
function useWatchedStatus() {
  const [status, setStatus] = useState<WatchedFolderStatus | null>(null);
  const eventCount = useRef(0);
  useEffect(() => {
    let live = true;
    const count = eventCount.current;
    commands.watchedFolderStatus().then(
      (result) => {
        if (live && count === eventCount.current && result.status === "ok") setStatus(result.data);
      },
      () => {},
    );
    const changed = events.watchedFolderChanged.listen(({ payload }) => {
      eventCount.current += 1;
      setStatus(payload.status);
    });
    return () => {
      live = false;
      void changed.then((f) => f());
    };
  }, []);
  return status;
}

/// `turnedOff` is the reason when a storage folder change turned the
/// watched folder off (R9, U7). It shows while the setting stays off: the
/// user must choose another folder.
export function WatchedFolder({
  settings,
  turnedOff = null,
  onSettings,
}: {
  settings: Settings;
  turnedOff?: string | null;
  onSettings: (s: Settings) => void;
}) {
  const folder = settings.watched_folder ?? null;
  const status = useWatchedStatus();
  const [reading, setReading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState<{
    folder: string;
    preview: WatchedFolderPreview;
    answer: (a: Answer) => void;
  } | null>(null);

  const set = useCallback(
    async (next: string | null, existing: ExistingFiles) => {
      const result = await commands.setWatchedFolder(next, existing);
      if (result.status === "error") setError(result.error);
      else {
        setError(null);
        onSettings(result.data);
      }
    },
    [onSettings],
  );

  // Pick, read the folder, ask, then set (F3).
  const choose = async () => {
    setBusy(true);
    try {
      const picked = await commands.pickWatchedFolder();
      if (picked.status === "error") return setError(picked.error);
      const next = picked.data;
      if (next === null) return;
      setReading(true);
      const preview = await commands.watchedFolderPreview(next).finally(() => setReading(false));
      if (preview.status === "error") return setError(preview.error);
      let existing: ExistingFiles = "import";
      if (preview.data.count > 0) {
        const answer = await new Promise<Answer>((resolve) =>
          setAsking({ folder: next, preview: preview.data, answer: resolve }),
        );
        setAsking(null);
        if (answer === "cancel") return;
        existing = answer;
      }
      await set(next, existing);
    } finally {
      setBusy(false);
    }
  };

  const turnOff = async () => {
    setBusy(true);
    try {
      await set(null, "import");
    } finally {
      setBusy(false);
    }
  };

  const unavailable = folder !== null && status?.kind === "unavailable" && status.path === folder;

  return (
    <Group
      header="Watched folder"
      footer="Each new m4a, mp3 or wav file in this folder and its subfolders becomes a note. Longhand never changes or deletes the files. This setting is for this computer only."
    >
      {folder === null ? (
        <Row label="Watched folder" detail="Off">
          <Button disabled={busy} onClick={() => void choose()}>
            Choose Folder…
          </Button>
        </Row>
      ) : (
        <Row label={folderName(folder)} detail={folder}>
          <Button disabled={busy} onClick={() => void choose()}>
            Change…
          </Button>
          <Button disabled={busy} onClick={() => void turnOff()}>
            Turn Off
          </Button>
        </Row>
      )}
      {folder === null && turnedOff && (
        <Row
          label={
            <span role="status">
              <Status tone="caution">Turned off</Status>
            </span>
          }
          detail={turnedOff}
        />
      )}
      {reading && (
        <Row
          label={
            <span role="status">
              <Status tone="neutral">Reading the folder…</Status>
            </span>
          }
        />
      )}
      {unavailable && !reading && (
        <Row
          label={
            <span role="status">
              <Status tone="caution">Folder not available</Status>
            </span>
          }
          detail="Longhand tries again each minute. Connect its disk, or choose another folder."
        />
      )}
      {error && <ErrorText>{error}</ErrorText>}
      {asking && <ExistingFilesDialog folder={asking.folder} preview={asking.preview} onAnswer={asking.answer} />}
    </Group>
  );
}
