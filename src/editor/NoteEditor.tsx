// The My Notes editor (R21–R23, R25). It shows markdown as formatted text
// while the user types, with a selection toolbar and a `/` menu, and gives
// the document back as markdown.
//
// Longhand's own component. It follows the pattern of Anarlog's
// `note/index.tsx` (see PROVENANCE.md) with only the plugins that notes use.

import "./editor.css";

import {
  ProseMirror,
  ProseMirrorDoc,
  reactKeys,
  useEditorEffect,
  type NodeViewComponentProps,
} from "@handlewithcare/react-prosemirror";
import { dropCursor } from "prosemirror-dropcursor";
import { gapCursor } from "prosemirror-gapcursor";
import { history } from "prosemirror-history";
import { Node as PMNode } from "prosemirror-model";
import { EditorState, Plugin, Selection } from "prosemirror-state";
import { type ComponentType, useEffect, useMemo, useRef } from "react";

import { json2md, type JSONContent, md2json } from "./markdown";
import { TaskItemView } from "./node-views/task-item-view";
import { buildInputRules, buildKeymap } from "./note/keymap";
import { schema } from "./note/schema";
import { autolinkPlugin } from "./plugins/autolink";
import { clearMarksOnEnterPlugin } from "./plugins/clear-marks-on-enter";
import { docChangeListenerPlugin } from "./plugins/doc-change-listener";
import { linkBoundaryGuardPlugin } from "./plugins/link-boundary-guard";
import { placeholderPlugin } from "./plugins/placeholder";
import { FormatToolbar } from "./widgets/format-toolbar";
import { SlashCommandMenu } from "./widgets/slash-command";

/** The time without typing after which the editor sends the document. */
export const SAVE_DEBOUNCE_MS = 500;
/** The longest time that a change waits, so that continuous typing still
 * sends the document at least once each second (R23). */
export const SAVE_MAX_WAIT_MS = 1000;

export type NoteEditorProps = {
  /** The document at mount. A later value has no effect: give the editor a
   * new `key` to open a different document. */
  markdown: string;
  /** Receives the whole document as markdown after a change. */
  onChange: (markdown: string) => void;
  autoFocus?: boolean;
};

const nodeViews = {
  taskItem: TaskItemView as ComponentType<NodeViewComponentProps>,
};

function createDoc(markdown: string): PMNode {
  try {
    return PMNode.fromJSON(schema, md2json(markdown));
  } catch {
    return schema.node("doc", null, [schema.node("paragraph")]);
  }
}

export function NoteEditor({ markdown, onChange, autoFocus = false }: NoteEditorProps) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // The document that is not yet sent, and the two timers that send it: one
  // restarts on each change, the other starts at the first change only.
  const pending = useRef<{
    doc: PMNode | null;
    debounce?: ReturnType<typeof setTimeout>;
    maxWait?: ReturnType<typeof setTimeout>;
  }>({ doc: null });

  const flush = useRef(() => {
    const p = pending.current;
    clearTimeout(p.debounce);
    clearTimeout(p.maxWait);
    p.debounce = p.maxWait = undefined;
    if (!p.doc) return;
    const doc = p.doc;
    p.doc = null;
    onChangeRef.current(json2md(doc.toJSON() as JSONContent));
  }).current;

  const defaultState = useMemo(() => {
    const schedule = (doc: PMNode) => {
      const p = pending.current;
      p.doc = doc;
      clearTimeout(p.debounce);
      p.debounce = setTimeout(flush, SAVE_DEBOUNCE_MS);
      p.maxWait ??= setTimeout(flush, SAVE_MAX_WAIT_MS);
    };
    const flushOnBlur = new Plugin({
      props: {
        handleDOMEvents: {
          blur: () => {
            flush();
            return false;
          },
        },
      },
    });
    return EditorState.create({
      doc: createDoc(markdown),
      plugins: [
        reactKeys(),
        docChangeListenerPlugin(schedule),
        buildInputRules(),
        buildKeymap(),
        history(),
        dropCursor(),
        gapCursor(),
        placeholderPlugin(undefined, ({ doc, pos }) =>
          pos === 0 && doc.childCount === 1 ? "Take notes. Type / for headings, lists and more." : "",
        ),
        clearMarksOnEnterPlugin(),
        autolinkPlugin(),
        linkBoundaryGuardPlugin(),
        flushOnBlur,
      ],
    });
    // The editor reads `markdown` at mount only (see NoteEditorProps).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Send a pending change when the editor closes.
  useEffect(() => flush, [flush]);

  return (
    <ProseMirror
      defaultState={defaultState}
      nodeViewComponents={nodeViews}
      attributes={{
        spellCheck: "true",
        role: "textbox",
        "aria-multiline": "true",
        "aria-label": "My Notes",
        class: "prosemirror-editor note-typography",
      }}
    >
      <ProseMirrorDoc />
      {autoFocus && <FocusAtEnd />}
      <FormatToolbar />
      <SlashCommandMenu />
    </ProseMirror>
  );
}

// Put the cursor at the end of the document once, when the view exists.
function FocusAtEnd() {
  const done = useRef(false);
  useEditorEffect((view) => {
    if (!view || done.current) return;
    done.current = true;
    view.dispatch(view.state.tr.setSelection(Selection.atEnd(view.state.doc)));
    view.focus();
  });
  return null;
}
