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
import {
  type ComponentType,
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";

import { addNotesSend } from "../lib/notesSaves";
import { json2md, type JSONContent, md2json } from "./markdown";
import { TaskItemView } from "./node-views/task-item-view";
import { buildInputRules, buildKeymap } from "./note/keymap";
import { schema } from "./note/schema";
import { autolinkPlugin } from "./plugins/autolink";
import { clearMarksOnEnterPlugin } from "./plugins/clear-marks-on-enter";
import { clipboardPlugin } from "./plugins/clipboard";
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
  /** False while the notes cannot change, for example while they wait for
   * the sync. The editor then shows the document and takes no input. */
  editable?: boolean;
  /** The id of the element that names the editor. Without it, the name is
   * "My Notes". */
  labelledBy?: string;
  /** The id of the element that describes the editor, for example a hint. */
  describedBy?: string;
  /** The text of an empty editor. The default is the My Notes text. Null
   * shows no placeholder. */
  placeholder?: string | null;
  /** Call `onChange` at each change, with no delay. The parent then sets its
   * own save delay. The editor reads this value at mount only. */
  immediate?: boolean;
  /** Called when the focus leaves the editor, after a pending change is sent. */
  onBlur?: () => void;
  /** More classes for the editable element. */
  className?: string;
};

const NOTES_PLACEHOLDER = "Take notes. Type / for headings, lists and more.";

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

/** `flush` sends the pending change now, if there is one. */
export type NoteEditorHandle = { flush: () => void };

export const NoteEditor = forwardRef<NoteEditorHandle, NoteEditorProps>(function NoteEditor(
  {
    markdown,
    onChange,
    autoFocus = false,
    editable = true,
    labelledBy,
    describedBy,
    placeholder = NOTES_PLACEHOLDER,
    immediate = false,
    onBlur,
    className,
  },
  ref,
) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const onBlurRef = useRef(onBlur);
  onBlurRef.current = onBlur;
  const editableRef = useRef(editable);
  editableRef.current = editable;

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
  useImperativeHandle(ref, () => ({ flush }), [flush]);

  const defaultState = useMemo(() => {
    const schedule = (doc: PMNode) => {
      const p = pending.current;
      p.doc = doc;
      if (immediate) {
        flush();
        return;
      }
      clearTimeout(p.debounce);
      p.debounce = setTimeout(flush, SAVE_DEBOUNCE_MS);
      p.maxWait ??= setTimeout(flush, SAVE_MAX_WAIT_MS);
    };
    const flushOnBlur = new Plugin({
      props: {
        handleDOMEvents: {
          blur: () => {
            flush();
            onBlurRef.current?.();
            return false;
          },
        },
      },
    });
    // A view that is not editable still lets a task checkbox send a change.
    // This plugin stops each change to the document while the editor is not
    // editable.
    const readOnlyGuard = new Plugin({
      filterTransaction: (tr) => editableRef.current || !tr.docChanged,
    });
    return EditorState.create({
      doc: createDoc(markdown),
      plugins: [
        reactKeys(),
        readOnlyGuard,
        docChangeListenerPlugin(schedule),
        buildInputRules(),
        buildKeymap(),
        history(),
        dropCursor(),
        gapCursor(),
        ...(placeholder === null
          ? []
          : [
              placeholderPlugin(undefined, ({ doc, pos }) =>
                pos === 0 && doc.childCount === 1 ? placeholder : "",
              ),
            ]),
        clearMarksOnEnterPlugin(),
        autolinkPlugin(),
        linkBoundaryGuardPlugin(),
        clipboardPlugin(),
        flushOnBlur,
      ],
    });
    // The editor reads `markdown`, `placeholder` and `immediate` at mount
    // only (see NoteEditorProps).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Send a pending change when the editor closes, and when a leave of the
  // note asks for it.
  useEffect(() => {
    const remove = addNotesSend(flush);
    return () => {
      remove();
      flush();
    };
  }, [flush]);

  return (
    <ProseMirror
      defaultState={defaultState}
      editable={() => editable}
      nodeViewComponents={nodeViews}
      attributes={{
        spellCheck: "true",
        role: "textbox",
        "aria-multiline": "true",
        ...(labelledBy ? { "aria-labelledby": labelledBy } : { "aria-label": "My Notes" }),
        ...(describedBy ? { "aria-describedby": describedBy } : {}),
        ...(editable ? {} : { "aria-readonly": "true" }),
        class: `prosemirror-editor note-typography${className ? ` ${className}` : ""}`,
      }}
    >
      <ProseMirrorDoc />
      {autoFocus && <FocusAtEnd />}
      {/* The toolbar and the `/` menu change the text, so they do not show
          while the editor is not editable. */}
      {editable && <FormatToolbar />}
      {editable && <SlashCommandMenu />}
    </ProseMirror>
  );
});

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
