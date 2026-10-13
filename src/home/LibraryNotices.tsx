// The library notices of Home, in the order of KTD14 of the safe agent
// edits plan: the Convert notice with the result of the last conversion,
// then the notes that Longhand can't read. Each shows only when the user
// must act: the normal path shows nothing.

import { useState } from "react";
import { commands, type ConversionNotice, type LibraryNotices as Notices } from "../bindings";
import { CautionIcon } from "../settings/icons";
import { Button } from "../settings/ui";

export function LibraryNotices({ notices, onChanged }: { notices: Notices | null; onChanged: () => void }) {
  if (!notices) return null;
  return (
    <>
      {notices.conversion && <ConvertNotice notice={notices.conversion} onChanged={onChanged} />}
      <ProblemNotice problems={notices.problems} />
    </>
  );
}

const box =
  "mx-[24px] mt-1 mb-2 flex gap-2.5 rounded-[10px] bg-caution/10 px-3 py-2 text-[0.85rem] shadow-[inset_0_0_0_0.5px_color-mix(in_srgb,var(--caution)_35%,transparent)]";

function notes(count: number): string {
  return count === 1 ? "1 note" : `${count} notes`;
}

/// The notice for notes that an older Longhand saved (R11, F3). Only the
/// user starts the conversion (R12). Convert stays available after a run
/// that skipped or could not convert notes (R10).
function ConvertNotice({ notice, onChanged }: { notice: ConversionNotice; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const running = busy || notice.running;
  const convert = async () => {
    setBusy(true);
    try {
      await commands.convertOldNotes();
    } finally {
      setBusy(false);
      onChanged();
    }
  };
  const verb = notice.old_notes === 1 ? "is" : "are";
  return (
    <section aria-label="Notes in an older format" className={`${box} items-start`}>
      <CautionIcon className="mt-[0.15rem] size-4 shrink-0 text-caution" />
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
      </div>
      <Button disabled={running} onClick={() => void convert()}>
        {running ? "Converting…" : "Convert"}
      </Button>
    </section>
  );
}

/// One notice for the notes whose files Longhand can't read (R6). It
/// counts the notes and lists each with what to do.
function ProblemNotice({ problems }: { problems: Notices["problems"] }) {
  if (problems.length === 0) return null;
  return (
    <section aria-label="Notes that Longhand can’t read" className={box}>
      <CautionIcon className="mt-[0.15rem] size-4 shrink-0 text-caution" />
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
    </section>
  );
}
