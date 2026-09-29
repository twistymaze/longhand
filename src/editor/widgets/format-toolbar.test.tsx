import { afterEach, beforeEach, describe, expect, jest, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  ProseMirror,
  ProseMirrorDoc,
  reactKeys,
  useEditorEffect,
} from "@handlewithcare/react-prosemirror";
import { EditorState, TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

import { schema } from "../note/schema";
import { FormatToolbar } from "./format-toolbar";

// Render tests for the composition gate. See PROVENANCE.md.

// ProseMirror's own composition handlers read the DOM selection back into the
// state. The test DOM does not keep the DOM selection in step with the view,
// so that read collapses the selection. The tests send composition events
// that do not bubble, and this listener cancels them before they get to the
// view. ProseMirror then runs the toolbar's listener, but not its own
// handler, for a cancelled event that does not bubble.
const cancel = (event: Event) => event.preventDefault();

beforeEach(() => {
  jest.useFakeTimers();
  document.addEventListener("compositionstart", cancel, true);
  document.addEventListener("compositionend", cancel, true);
});

afterEach(() => {
  cleanup();
  document.removeEventListener("compositionstart", cancel, true);
  document.removeEventListener("compositionend", cancel, true);
  jest.useRealTimers();
});

// Give the view to the test when it exists.
function CaptureView({ onView }: { onView: (view: EditorView) => void }) {
  useEditorEffect((view) => {
    if (view) onView(view);
  });
  return null;
}

// Mount the toolbar in an editor that holds one paragraph, "compose text".
function setup() {
  let view: EditorView | null = null;
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [schema.text("compose text")]),
  ]);
  const defaultState = EditorState.create({ doc, plugins: [reactKeys()] });
  const rendered = render(
    <ProseMirror defaultState={defaultState}>
      <ProseMirrorDoc />
      <CaptureView
        onView={(next) => {
          if (view === next) return;
          view = next;
          // The DOM in tests has no layout, so give fixed coordinates.
          spyOn(next, "coordsAtPos").mockReturnValue({ bottom: 20, left: 0, right: 40, top: 0 });
        }}
      />
      <FormatToolbar />
    </ProseMirror>,
  );
  if (!view) throw new Error("the editor view does not exist");
  const v: EditorView = view;
  const select = (from: number, to = from) =>
    act(() => {
      v.dispatch(v.state.tr.setSelection(TextSelection.create(v.state.doc, from, to)));
    });
  const composition = (type: "start" | "end") =>
    act(() => {
      if (type === "start") fireEvent.compositionStart(v.dom, { bubbles: false });
      else fireEvent.compositionEnd(v.dom, { bubbles: false });
    });
  const advance = (ms: number) =>
    act(() => {
      jest.advanceTimersByTime(ms);
    });
  return { view: v, rendered, select, composition, advance };
}

const toolbar = () => screen.queryByRole("toolbar");

describe("FormatToolbar composition gate", () => {
  test("shows the toolbar for a selection without composition", () => {
    const { select } = setup();
    select(1, 8);
    expect(toolbar()).not.toBeNull();
  });

  test("hides the toolbar when a composition starts over a selection", () => {
    const { view, select, composition } = setup();
    select(1, 8);
    expect(toolbar()).not.toBeNull();
    composition("start");
    // The selection is still a range, so only the gate hides the toolbar.
    expect(view.state.selection.empty).toBe(false);
    expect(toolbar()).toBeNull();
  });

  test("a range selection during composition does not show the toolbar", () => {
    const { select, composition } = setup();
    composition("start");
    select(1, 8);
    expect(toolbar()).toBeNull();
  });

  test("after the composition ends, the toolbar stays hidden until 50 ms pass", () => {
    const { view, select, composition, advance } = setup();
    composition("start");
    select(1, 8);
    composition("end");
    // A transaction that keeps the selection does not open the gate.
    act(() => {
      view.dispatch(view.state.tr.setMeta("meta-only", true));
    });
    expect(toolbar()).toBeNull();
    advance(40);
    expect(toolbar()).toBeNull();
    advance(20);
    // The same range now counts as a real selection.
    expect(toolbar()).not.toBeNull();
    select(2);
    expect(toolbar()).toBeNull();
    select(1, 8);
    expect(toolbar()).not.toBeNull();
  });

  test("a selection change after the composition ends shows the toolbar at once", () => {
    const { select, composition } = setup();
    composition("start");
    select(1, 8);
    composition("end");
    expect(toolbar()).toBeNull();
    select(2, 6);
    expect(toolbar()).not.toBeNull();
  });

  test("a new composition in the 50 ms window stays hidden after the old timer", () => {
    const { select, composition, advance } = setup();
    composition("start");
    composition("end");
    composition("start");
    select(1, 8);
    advance(100);
    expect(toolbar()).toBeNull();
    composition("end");
    advance(100);
    expect(toolbar()).not.toBeNull();
  });

  test("unmount in the 50 ms window gives no error", () => {
    const errors = spyOn(console, "error");
    try {
      const { rendered, select, composition, advance } = setup();
      composition("start");
      select(1, 8);
      composition("end");
      rendered.unmount();
      advance(100);
      expect(errors).not.toHaveBeenCalled();
    } finally {
      errors.mockRestore();
    }
  });
});
