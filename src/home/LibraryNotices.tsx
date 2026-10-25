// The library notices of Home, in the order of KTD14 of the safe agent
// edits plan: the Convert notice with the result of the last conversion,
// then the notes that Longhand can't read, then the notices of the audio
// import. Each shows only when the user must act: the normal path shows
// nothing.

import { type ReactNode, useState } from "react";
import { commands, type ConversionNotice, type LibraryNotices as Notices } from "../bindings";
import { CautionIcon } from "../settings/icons";
import { Button, folderName, notes } from "../settings/ui";

export function LibraryNotices({ notices, onChanged }: { notices: Notices | null; onChanged: () => void }) {
  if (!notices) return null;
  return (
    <>
      {notices.conversion && <ConvertNotice notice={notices.conversion} onChanged={onChanged} />}
      <ProblemNotice problems={notices.problems} />
      <ImportNotices imports={notices.imports} onChanged={onChanged} />
    </>
  );
}

/// The notices of the audio import. Each
/// shows only when the user must act or an import failed. An import that
/// waits for a download or a recording shows no notice (R21).
function ImportNotices({ imports, onChanged }: { imports: Notices["imports"]; onChanged: () => void }) {
  const dismiss = async (noteId: string) => {
    try {
      await commands.dismissImportFailure(noteId);
    } finally {
      onChanged();
    }
  };
  const waiting = imports.waiting_for_space.map((name) => `“${name}”`).join(", ");
  return (
    <>
      {imports.watched_folder !== null && (
        <CautionBanner label="Watched folder not available">
          <p className="min-w-0 flex-1">
            Longhand can’t open the watched folder “{folderName(imports.watched_folder)}”. Connect its disk, or
            choose another folder in Settings.
          </p>
          <Button onClick={() => void commands.openSettings("destination")}>Open Settings…</Button>
        </CautionBanner>
      )}
      {imports.waiting_for_space.length > 0 && (
        <CautionBanner label="Imports that wait for disk space">
          <p className="min-w-0 flex-1">
            {imports.waiting_for_space.length === 1
              ? `There is not enough disk space to import ${waiting}. Free some space, and the import continues.`
              : `There is not enough disk space to import ${waiting}. Free some space, and the imports continue.`}
          </p>
        </CautionBanner>
      )}
      {imports.failed.length > 0 && (
        <CautionBanner label="Audio files that could not be imported">
          <ul className="min-w-0 flex-1 space-y-1.5">
            {imports.failed.map((failure) => (
              <li key={failure.note_id} className="flex items-center gap-2.5">
                <p className="min-w-0 flex-1">
                  Longhand can’t read the audio in “{failure.file_name}”. The file may be damaged.
                </p>
                <Button label={`Dismiss “${failure.file_name}”`} onClick={() => void dismiss(failure.note_id)}>
                  Dismiss
                </Button>
              </li>
            ))}
          </ul>
        </CautionBanner>
      )}
    </>
  );
}

/// A caution banner of Home. With a `label`, it is a region with that
/// name, and its content aligns to the top of the banner.
export function CautionBanner({ children, label }: { children: ReactNode; label?: string }) {
  const Tag = label ? "section" : "div";
  return (
    <Tag
      aria-label={label}
      className={`mx-[24px] mt-1 mb-2 flex ${label ? "items-start" : "items-center"} gap-2.5 rounded-[10px] bg-caution/10 px-3 py-2 text-[0.85rem] shadow-[inset_0_0_0_0.5px_color-mix(in_srgb,var(--caution)_35%,transparent)]`}
    >
      <CautionIcon className={label ? "mt-[0.15rem] size-4 shrink-0 text-caution" : "size-4 text-caution"} />
      {children}
    </Tag>
  );
}

/// The notice for notes that an older Longhand saved (R11, F3). Only the
/// user starts the conversion (R12). Convert stays available after a run
/// that skipped or could not convert notes (R10).
function ConvertNotice({ notice, onChanged }: { notice: ConversionNotice; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const [stopped, setStopped] = useState(false);
  const running = busy || notice.running;
  const convert = async () => {
    setBusy(true);
    setStopped(false);
    try {
      const result = await commands.convertOldNotes();
      setStopped(result.status === "error");
    } catch {
      setStopped(true);
    } finally {
      setBusy(false);
      onChanged();
    }
  };
  const verb = notice.old_notes === 1 ? "is" : "are";
  return (
    <CautionBanner label="Notes in an older format">
      <div className="min-w-0 flex-1">
        <p>
          {notes(notice.old_notes)} {verb} in an older format. Convert them to change their
          summary, transcript or type.
        </p>
        {notice.skipped > 0 && (
          <p className="mt-1">
            {notes(notice.skipped)} could not be converted yet because {notice.skipped === 1 ? "it was" : "they were"} in use. Select
            Convert again later.
          </p>
        )}
        {notice.failed.length > 0 && (
          <p className="mt-1">
            These notes could not be converted: {notice.failed.map((note) => note.name).join(", ")}.
          </p>
        )}
        {stopped && <p className="mt-1">The conversion did not finish. Select Convert to try again.</p>}
      </div>
      <Button disabled={running} onClick={() => void convert()}>
        {running ? "Converting…" : "Convert"}
      </Button>
    </CautionBanner>
  );
}

/// One notice for the notes whose files Longhand can't read (R6). It
/// counts the notes and lists each with what to do.
function ProblemNotice({ problems }: { problems: Notices["problems"] }) {
  if (problems.length === 0) return null;
  return (
    <CautionBanner label="Notes that Longhand can’t read">
      <div className="min-w-0 flex-1">
        <p className="font-medium">Longhand can’t read {notes(problems.length)}.</p>
        <ul className="mt-1 space-y-0.5">
          {problems.map((note, i) => (
            <li key={i} className="min-w-0">
              <span className="font-medium">{note.name}</span>
              {" — "}
              <span className="text-label-secondary">{note.problem.reason}</span>
            </li>
          ))}
        </ul>
      </div>
    </CautionBanner>
  );
}
