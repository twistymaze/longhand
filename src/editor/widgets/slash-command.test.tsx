import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  ProseMirror,
  ProseMirrorDoc,
  reactKeys,
  useEditorEffect,
} from "@handlewithcare/react-prosemirror";
import { EditorState, TextSelection, type Transaction } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

import { schema } from "../note/schema";
import { SLASH_COMMANDS, SlashCommandMenu } from "./slash-command";

afterEach(cleanup);

const para = (text?: string) => schema.node("paragraph", null, text ? [schema.text(text)] : []);

// A view with a state and a dispatch, enough for a command's action.
function viewOf(doc: ReturnType<typeof schema.node>, cursor: number) {
  let state = EditorState.create({ doc, selection: TextSelection.create(doc, cursor) });
  return {
    get state() {
      return state;
    },
    dispatch(tr: Transaction) {
      state = state.apply(tr);
    },
  } as Pick<EditorView, "dispatch" | "state"> as EditorView;
}

// Run the command `id` on "Call Anna /": the slash is at the end.
function runAfterText(id: string) {
  const doc = schema.node("doc", null, [para("Call Anna /")]);
  const to = doc.firstChild!.nodeSize - 1;
  const view = viewOf(doc, to);
  SLASH_COMMANDS.find((c) => c.id === id)!.action(view, to - 1, to);
  return view.state;
}

describe("slash commands after text", () => {
  test("Task list keeps the text of the line in the task", () => {
    const state = runAfterText("taskList");
    expect(state.doc.toJSON()).toEqual(
      schema
        .node("doc", null, [
          schema.node("taskList", null, [schema.node("taskItem", { checked: false }, [para("Call Anna ")])]),
        ])
        .toJSON(),
    );
  });

  test("Divider keeps the line and puts the rule after it", () => {
    const state = runAfterText("horizontalRule");
    expect(state.doc.toJSON()).toEqual(
      schema.node("doc", null, [para("Call Anna "), schema.node("horizontalRule"), para()]).toJSON(),
    );
    expect(state.selection.$from.parent.type.name).toBe("paragraph");
    expect(state.selection.$from.parent.childCount).toBe(0);
  });

  test("Task list in a bullet item keeps the item and adds the task after its text", () => {
    const doc = schema.node("doc", null, [
      schema.node("bulletList", null, [schema.node("listItem", null, [para("item /")])]),
    ]);
    const to = 1 + 1 + 1 + "item /".length;
    const view = viewOf(doc, to);
    SLASH_COMMANDS.find((c) => c.id === "taskList")!.action(view, to - 1, to);
    expect(view.state.doc.toJSON()).toEqual(
      schema
        .node("doc", null, [
          schema.node("bulletList", null, [
            schema.node("listItem", null, [
              para("item "),
              schema.node("taskList", null, [schema.node("taskItem", { checked: false }, [para()])]),
            ]),
          ]),
        ])
        .toJSON(),
    );
  });

  test("Divider on an empty line replaces the line", () => {
    const doc = schema.node("doc", null, [para("/")]);
    const view = viewOf(doc, 2);
    SLASH_COMMANDS.find((c) => c.id === "horizontalRule")!.action(view, 1, 2);
    expect(view.state.doc.toJSON()).toEqual(schema.node("doc", null, [schema.node("horizontalRule"), para()]).toJSON());
  });
});

function CaptureView({ onView }: { onView: (view: EditorView) => void }) {
  useEditorEffect((view) => {
    if (view) onView(view);
  });
  return null;
}

describe("slash menu keys", () => {
  test("Enter runs the shown item after the filter makes the list shorter", () => {
    let view: EditorView | null = null;
    const doc = schema.node("doc", null, [para()]);
    render(
      <ProseMirror defaultState={EditorState.create({ doc, plugins: [reactKeys()] })}>
        <ProseMirrorDoc />
        <CaptureView
          onView={(next) => {
            if (view === next) return;
            view = next;
            spyOn(next, "coordsAtPos").mockReturnValue({ bottom: 20, left: 0, right: 40, top: 0 });
          }}
        />
        <SlashCommandMenu />
      </ProseMirror>,
    );
    const v = view as unknown as EditorView;
    const typeText = (text: string) =>
      act(() => {
        v.dispatch(v.state.tr.insertText(text));
      });
    typeText("/");
    // ArrowUp from the first item selects the last item.
    act(() => {
      fireEvent.keyDown(v.dom, { key: "ArrowUp" });
    });
    typeText("h1");
    expect(screen.getAllByRole("button")).toHaveLength(1);
    act(() => {
      fireEvent.keyDown(v.dom, { key: "Enter" });
    });
    expect(v.state.doc.firstChild!.type.name).toBe("heading");
  });
});
