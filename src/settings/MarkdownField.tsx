// A Settings field that edits Markdown with the My Notes editor (KTD7). It
// shows a label, a hint under the label, a count and the editor in a field
// frame. The frame keeps the states of `fieldClass`: a focus ring, a red
// border over the limit and a read-only look.

import { type CSSProperties, useRef } from "react";

import { NoteEditor } from "../editor/NoteEditor";

const format = (n: number) => n.toLocaleString("en-US");

export type MarkdownFieldProps = {
  /** The base of the element ids. It must be unique on the page. */
  id: string;
  label: string;
  /** The line under the label. It stays visible while the user types. */
  hint: string;
  /** The text at mount. A later value has no effect: the parent gives the
   * field a new `key` to show a different text (KTD10). */
  value: string;
  /** The number of characters of the current text. The parent counts the
   * Markdown that it saves. */
  count: number;
  limit: number;
  /** False hides the count. The red border over the limit stays. */
  showCount?: boolean;
  /** True shows the text and takes no input. */
  readOnly?: boolean;
  /** The minimum height of the editor, in lines. */
  rows?: number;
  /** Receives the whole text as Markdown at each change, with no delay. */
  onChange: (markdown: string) => void;
  /** Called when the focus leaves the editor. */
  onBlur?: () => void;
};

export function MarkdownField({
  id,
  label,
  hint,
  value,
  count,
  limit,
  showCount = true,
  readOnly = false,
  rows = 4,
  onChange,
  onBlur,
}: MarkdownFieldProps) {
  const frame = useRef<HTMLDivElement>(null);
  const labelId = `${id}-label`;
  const hintId = `${id}-hint`;
  const over = count > limit;

  // The label is not a <label> element, because the editor is not a form
  // control. A click on the label puts the focus in the editor, as a click on
  // a <label> does.
  const focusEditor = () => {
    if (readOnly) return;
    frame.current?.querySelector<HTMLElement>('[role="textbox"]')?.focus();
  };

  return (
    <div>
      <div className="mb-2 flex items-end gap-3">
        <div className="min-w-0 flex-1">
          <div id={labelId} className="font-medium" onClick={focusEditor}>
            {label}
          </div>
          <p id={hintId} className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">
            {hint}
          </p>
        </div>
        {showCount && (
          <span
            className={`shrink-0 text-[0.85rem] tabular-nums ${over ? "text-negative" : "text-label-secondary"}`}
          >
            {`${format(count)} / ${format(limit)}`}
          </span>
        )}
      </div>
      <div
        ref={frame}
        data-markdown-field
        style={{ "--markdown-field-rows": rows } as CSSProperties}
        className={`rounded-md border ${over ? "border-negative" : "border-control-border"} bg-control px-3 py-1.5 outline-offset-0 focus-within:outline-3 focus-within:outline-accent/50 ${
          readOnly ? "text-label-secondary" : "text-label"
        }`}
      >
        <NoteEditor
          markdown={value}
          onChange={onChange}
          onBlur={onBlur}
          editable={!readOnly}
          labelledBy={labelId}
          describedBy={hintId}
          placeholder={null}
          immediate
          className="markdown-field"
        />
      </div>
    </div>
  );
}
