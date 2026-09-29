// The question after the user picks a new storage folder (R4): Move the
// notes of the old folder, leave them there, or cancel the change. Move
// has the focus, so Return moves the notes and no note is left behind by
// accident.

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { commands, type OldNotes, type Result, type Settings } from "../bindings";
import { folderName } from "./ui";

type Answer = OldNotes | "cancel";

export function OldNotesDialog({ folder, onAnswer }: { folder: string; onAnswer: (answer: Answer) => void }) {
  const id = useId();
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const answer = useRef(onAnswer);
  answer.current = onAnswer;
  // Order in the row: Cancel, Leave Them, Move Notes. Move Notes has the
  // focus at the start.
  useEffect(() => buttons.current[2]?.focus(), []);
  // The keys go to the window. In WKWebView a click does not give a button
  // the focus, so the focus can be on the page body after a click in the
  // dialog.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        answer.current("cancel");
      }
      if (e.key === "Tab") {
        e.preventDefault();
        const list = buttons.current.filter((b): b is HTMLButtonElement => b !== null);
        const at = list.indexOf(document.activeElement as HTMLButtonElement);
        const next = at < 0 ? list.length - 1 : (at + (e.shiftKey ? list.length - 1 : 1)) % list.length;
        list[next]?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const button = (index: number, label: string, value: Answer, prominent = false) => (
    <button
      ref={(b) => {
        buttons.current[index] = b;
      }}
      type="button"
      onClick={() => onAnswer(value)}
      className={`rounded-md px-2.5 py-[3px] whitespace-nowrap outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${
        prominent ? "bg-accent text-white active:bg-accent-pressed" : "bg-fill text-label active:bg-fill-pressed"
      }`}
    >
      {label}
    </button>
  );
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-black/20 px-4 pt-[18vh] pb-4">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-message`}
        className="fade-in flex max-h-[calc(82vh-1rem)] w-[22rem] max-w-[calc(100vw-2rem)] flex-col rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]"
      >
        <h2 id={`${id}-title`} className="shrink-0 font-semibold [overflow-wrap:anywhere]">
          Move your notes to “{folderName(folder)}”?
        </h2>
        <div
          id={`${id}-message`}
          className="mt-1 grid min-h-0 gap-1.5 overflow-y-auto text-[0.85rem] text-label-secondary [overflow-wrap:anywhere]"
        >
          <p>Move Notes puts every note in the new folder. You can keep working while they move.</p>
          <p>Leave Them keeps the notes in the old folder. Longhand no longer lists them.</p>
        </div>
        <div className="mt-4 flex shrink-0 flex-wrap justify-end gap-2">
          {button(0, "Cancel", "cancel")}
          {button(1, "Leave Them", "leave")}
          {button(2, "Move Notes", "move", true)}
        </div>
      </div>
    </div>
  );
}

/// Change the storage folder: pick a folder, ask what to do with the
/// notes of the old folder, then change. `change` resolves to `null` data
/// when the user closes the folder dialog or selects Cancel. `dialog` is
/// the question to render, or null. `moved` is true when the change
/// started a Move.
export function useFolderChange() {
  const [asking, setAsking] = useState<{ folder: string; answer: (a: Answer) => void } | null>(null);
  const change = useCallback(async (): Promise<Result<{ settings: Settings; moved: boolean } | null, string>> => {
    const picked = await commands.pickStorageFolder();
    if (picked.status === "error") return picked;
    const folder = picked.data;
    if (folder === null) return { status: "ok", data: null };
    const answer = await new Promise<Answer>((resolve) => setAsking({ folder, answer: resolve }));
    setAsking(null);
    if (answer === "cancel") return { status: "ok", data: null };
    const changed = await commands.changeStorageFolder(folder, answer);
    if (changed.status === "error") return changed;
    return { status: "ok", data: { settings: changed.data, moved: answer === "move" } };
  }, []);
  const dialog = asking && <OldNotesDialog folder={asking.folder} onAnswer={asking.answer} />;
  return { change, dialog };
}
