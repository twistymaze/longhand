import { type Node as ProseMirrorNode } from "prosemirror-model";
import { EditorState } from "prosemirror-state";
import { describe, expect, it } from "vitest";

import { schema } from "../note/schema";
import { autolinkPlugin } from "./autolink";
import { getChangedRanges } from "./changed-ranges";
import { linkBoundaryGuardPlugin } from "./link-boundary-guard";

function item(text: string) {
  return schema.node("listItem", null, [
    schema.node("paragraph", null, text ? [schema.text(text)] : []),
  ]);
}

describe("getChangedRanges", () => {
  it("gives ranges in the final document when a typed bullet joins the list above", () => {
    // Positions: the list is 0-7, its only item is 1-6, the paragraph is 7-18.
    const doc = schema.node("doc", null, [
      schema.node("bulletList", null, [item("a")]),
      schema.node("paragraph", null, [schema.text("bbbbbbbbb")]),
    ]);
    const state = EditorState.create({ schema, doc });
    const listEnd = 7;
    const tr = state.tr
      // Type at the end of the paragraph after the list.
      .insertText("c", listEnd + 10)
      // Remove that paragraph. The document is now shorter than the step above.
      .delete(listEnd, listEnd + 12)
      // Add the typed text as a new item at the end of the list.
      .insert(listEnd - 1, item("bc"));

    const ranges = getChangedRanges([tr]);
    const itemStart = listEnd - 1;
    const itemEnd = itemStart + tr.doc.nodeAt(itemStart)!.nodeSize;

    expect(tr.doc.childCount).toBe(1);
    expect(ranges.length).toBeGreaterThan(0);
    for (const range of ranges) {
      expect(range.from).toBeGreaterThanOrEqual(0);
      expect(range.to).toBeLessThanOrEqual(tr.doc.content.size);
      expect(range.from).toBeLessThanOrEqual(range.to);
    }
    expect(
      ranges.some((range) => range.from <= itemStart && range.to >= itemEnd),
    ).toBe(true);
  });

  it("gives the inserted range for a single-step insert", () => {
    const state = createState(
      schema.node("doc", null, [
        schema.node("paragraph", null, [schema.text("ab")]),
      ]),
    );
    const tr = state.tr.insertText("xy", 2);

    expect(getChangedRanges([tr])).toEqual([{ from: 2, to: 4 }]);
  });

  it("gives no ranges when the transactions do not change the document", () => {
    const state = createState(
      schema.node("doc", null, [
        schema.node("paragraph", null, [schema.text("ab")]),
      ]),
    );
    const tr = state.tr.setMeta("x", true);

    expect(getChangedRanges([tr])).toEqual([]);
    expect(getChangedRanges([])).toEqual([]);
  });
});

describe("autolinkPlugin with more than one step", () => {
  it("links a URL that is typed in a transaction with more than one step", () => {
    const state = createState(
      schema.node("doc", null, [schema.node("paragraph")]),
    );
    const tr = state.tr.insertText("https://example.com", 1);
    tr.insertText(" ", tr.doc.content.size - 1);
    expect(tr.steps.length).toBeGreaterThan(1);

    const next = state.applyTransaction(tr).state;
    const link = next.doc.firstChild!.firstChild!;

    expect(link.text).toBe("https://example.com");
    expect(link.marks.map((mark) => mark.type.name)).toEqual(["link"]);
  });
});

function createState(doc: ProseMirrorNode) {
  return EditorState.create({
    schema,
    doc,
    plugins: [autolinkPlugin(), linkBoundaryGuardPlugin()],
  });
}
