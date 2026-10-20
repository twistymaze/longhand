import { afterEach, describe, expect, it } from "vitest";
import { EditorState, TextSelection, type Transaction } from "prosemirror-state";

import { schema } from "../note/schema";
import { getClipBoundary, isMarkActive, setLink } from "./format-toolbar";

afterEach(() => {
  document.body.innerHTML = "";
});

function createState(from: number, to: number) {
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [schema.text("Follow up")]),
  ]);

  return EditorState.create({
    doc,
    selection: TextSelection.create(doc, from, to),
  });
}

function run(state: EditorState, href: string | null) {
  let next = state;
  const done = setLink(state, (tr: Transaction) => (next = state.apply(tr)), href);
  return { done, state: next };
}

describe("setLink", () => {
  it("links the selected text", () => {
    const { done, state } = run(createState(1, 7), "example.com/plan");

    expect(done).toBe(true);
    expect(isMarkActive(state, schema.marks.link)).toBe(true);
    expect(state.doc.firstChild!.firstChild!.marks[0].attrs.href).toBe(
      "https://example.com/plan",
    );
  });

  it("gives no link for text that is not a web address", () => {
    const { done, state } = run(createState(1, 7), "not a link");

    expect(done).toBe(false);
    expect(isMarkActive(state, schema.marks.link)).toBe(false);
  });

  it("removes a link", () => {
    const linked = run(createState(1, 7), "https://example.com").state;
    const { state } = run(linked, null);

    expect(isMarkActive(state, schema.marks.link)).toBe(false);
  });

  it("does nothing with no selection", () => {
    expect(run(createState(3, 3), "https://example.com").done).toBe(false);
  });
});

describe("getClipBoundary", () => {
  it("returns the nearest overflow ancestor", () => {
    const scroller = document.createElement("div");
    scroller.style.overflowY = "auto";
    const inner = document.createElement("div");
    const target = document.createElement("div");
    inner.append(target);
    scroller.append(inner);
    document.body.append(scroller);

    expect(getClipBoundary(target)).toBe(scroller);
  });

  it("falls back to the element when there is no overflow ancestor", () => {
    const target = document.createElement("div");
    document.body.append(target);

    expect(getClipBoundary(target)).toBe(target);
  });
});
