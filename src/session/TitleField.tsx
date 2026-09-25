// The session page's title (R9, R34, R35). It is a text field in every
// state. Return or a click elsewhere commits the text, and Escape restores
// the title. An empty title clears it, and the start time then shows as
// placeholder text (R28).

import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { commands } from "../bindings";
import { sentence } from "../processing/describe";

export type Notice = { tone: "caution" | "negative"; text: string };

/// Rename a session, and say what the user must know about the result:
/// an error, or a folder that kept its old name. Null when all is well.
export async function commitTitle(sessionId: string, title: string): Promise<Notice | null> {
  const result = await commands.renameSession(sessionId, title);
  if (result.status === "error") return { tone: "negative", text: sentence(result.error) };
  if (result.data.kind === "kept") {
    return {
      tone: "caution",
      text: `The title changed, but the folder kept its old name. ${sentence(result.data.message)}`,
    };
  }
  return null;
}

export type TitleFieldHandle = { edit: () => void };

export const TitleField = forwardRef<
  TitleFieldHandle,
  {
    sessionId: string;
    title: string | null;
    /// The start time, shown while the session has no title.
    placeholder: string;
    /// The rename completed, with a notice for the user or null.
    onRenamed: (notice: Notice | null) => void;
  }
>(function TitleField({ sessionId, title, placeholder, onRenamed }, ref) {
  const [draft, setDraft] = useState(title ?? "");
  const input = useRef<HTMLInputElement>(null);
  const cancelled = useRef(false);

  useImperativeHandle(ref, () => ({
    edit: () => {
      input.current?.focus();
      input.current?.select();
    },
  }));

  // A title from elsewhere (Home, or a proposed title) replaces the text,
  // except while the user types.
  useEffect(() => {
    if (document.activeElement !== input.current) setDraft(title ?? "");
  }, [title]);

  const commit = async () => {
    if (cancelled.current) {
      cancelled.current = false;
      return;
    }
    const text = draft.trim();
    setDraft(text);
    if (text === (title ?? "")) return;
    onRenamed(await commitTitle(sessionId, text));
  };

  return (
    <input
      ref={input}
      aria-label="Title"
      value={draft}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => void commit()}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.currentTarget.blur();
        }
        if (e.key === "Escape") {
          e.preventDefault();
          cancelled.current = true;
          setDraft(title ?? "");
          e.currentTarget.blur();
        }
      }}
      className="-mx-1.5 w-[calc(100%+0.75rem)] min-w-0 truncate rounded-md bg-transparent px-1.5 py-0.5 text-[1.65rem] leading-[1.2] font-bold tracking-[-0.022em] text-label placeholder:text-label hover:bg-fill focus:bg-control focus:outline-3 focus:outline-accent/50"
    />
  );
});
