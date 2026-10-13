// The library notices of Home, in the order of KTD14 of the safe agent
// edits plan: the Convert notice with the result of the last conversion,
// then the notes that Longhand can't read. Each shows only when the user
// must act: the normal path shows nothing.

import { type ReactNode, useState } from "react";
import { commands, type ConversionNotice, type LibraryNotices as Notices } from "../bindings";
import { CautionIcon } from "../settings/icons";
import { Button, notes } from "../settings/ui";

export function LibraryNotices({ notices, onChanged }: { notices: Notices | null; onChanged: () => void }) {
  if (!notices) return null;
  return (
    <>
      {notices.conversion && <ConvertNotice notice={notices.conversion} onChanged={onChanged} />}
      <ProblemNotice problems={notices.problems} />
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
