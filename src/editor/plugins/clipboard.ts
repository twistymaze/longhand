// A copy or cut in the editor puts the selection on the clipboard with no
// colours (see src/lib/clipboard.ts). It replaces ProseMirror's own copy,
// whose HTML WebKit changes.

import { DOMSerializer } from "prosemirror-model";
import { Plugin } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { cleanHtml, writeCopyEvent } from "../../lib/clipboard";

function copy(view: EditorView, event: ClipboardEvent): boolean {
  const { selection } = view.state;
  if (selection.empty) return false;
  const slice = selection.content();
  const dom = DOMSerializer.fromSchema(view.state.schema).serializeFragment(slice.content);
  // The text is the one that ProseMirror writes: blocks apart by an empty line.
  writeCopyEvent(event, slice.content.textBetween(0, slice.content.size, "\n\n"), cleanHtml(dom));
  if (event.type === "cut" && view.editable) {
    view.dispatch(view.state.tr.deleteSelection().scrollIntoView().setMeta("uiEvent", "cut"));
  }
  return true;
}

export function clipboardPlugin(): Plugin {
  return new Plugin({ props: { handleDOMEvents: { copy, cut: copy } } });
}
