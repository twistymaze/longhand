import {
  EditorState,
  Selection,
  TextSelection,
  type Transaction,
} from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { describe, expect, it } from "bun:test";

import { buildInputRules, buildKeymap } from "./keymap";
import { schema } from "./schema";

describe("buildInputRules", () => {
  it("creates an unchecked task item when typing [] followed by space", () => {
    const inputRules = buildInputRules();
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, [schema.text("[]")]),
    ]);
    let state = EditorState.create({
      schema,
      doc,
      selection: Selection.atEnd(doc),
      plugins: [inputRules],
    });

    const view = {
      composing: false,
      get state() {
        return state;
      },
      dispatch(tr: Transaction) {
        state = state.apply(tr);
      },
    } as Pick<EditorView, "composing" | "dispatch" | "state"> as EditorView;

    const handleTextInput = inputRules.props.handleTextInput as
      | ((
          view: EditorView,
          from: number,
          to: number,
          text: string,
          deflt: () => Transaction,
        ) => boolean | void)
      | undefined;

    const handled = handleTextInput?.(
      view,
      state.selection.from,
      state.selection.to,
      " ",
      () => state.tr.insertText(" ", state.selection.from, state.selection.to),
    );

    expect(handled).toBe(true);
    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [{ type: "paragraph" }],
            },
          ],
        },
      ],
    });
  });

  it("keeps the cursor inside the new task before another paragraph", () => {
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, [schema.text("[]")]),
      schema.node("paragraph"),
    ]);
    const { state } = runTextInput(doc, " ", 3);

    expect(state.selection.$from.parent.type.name).toBe("paragraph");
    expect(state.selection.$from.node(-1).type.name).toBe("taskItem");
    expect(state.tr.insertText("My task").doc.firstChild?.textContent).toBe(
      "My task",
    );
  });

  it("leaves a third dash alone so --- can still become a horizontal rule", () => {
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, [schema.text("--")]),
    ]);
    const { handled, state } = runTextInput(doc, "-");

    expect(handled).toBeFalsy();
    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [{ type: "text", text: "--" }],
        },
      ],
    });
  });

  it("turns --- followed by a space into a horizontal rule", () => {
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, [schema.text("---")]),
    ]);
    const { handled, state } = runTextInput(doc, " ");

    expect(handled).toBe(true);
    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [{ type: "horizontalRule" }, { type: "paragraph" }],
    });
  });

});

describe("typed markdown", () => {
  it("makes a heading from ## at a line start", () => {
    const state = typeText(emptyDoc(), "## Plan");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "heading",
          attrs: { level: 2 },
          content: [{ type: "text", text: "Plan" }],
        },
      ],
    });
  });

  it("makes a task from - [ ] at a line start", () => {
    const state = typeText(emptyDoc(), "- [ ] Send notes");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "Send notes" }],
                },
              ],
            },
          ],
        },
      ],
    });
  });

  it("makes a rule from --- and a space", () => {
    const state = typeText(emptyDoc(), "--- ");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [{ type: "horizontalRule" }, { type: "paragraph" }],
    });
  });

  it("keeps quotes, dashes and arrows as typed", () => {
    const text = `"Yes" -- it's 1/2 -> (c)...`;
    const state = typeText(emptyDoc(), text);

    expect(state.doc.textContent).toBe(text);
  });
});

describe("buildKeymap", () => {
  it("does not handle Shift+Enter as a hard break shortcut", () => {
    const doc = schema.node("doc", null, [
      schema.node("paragraph", null, [schema.text("hello")]),
    ]);
    const { handled, state } = runKeyDownAtEnd(doc, "Enter", {
      shiftKey: true,
    });

    expect(handled).not.toBe(true);
    expect(state.doc.toJSON()).toEqual(doc.toJSON());
  });

  it("merges task item text backward without changing the list structure", () => {
    const doc = schema.node("doc", null, [
      schema.node("taskList", null, [
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("one")])],
        ),
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("two")])],
        ),
      ]),
    ]);
    const { state } = runBackspaceAtTextStart(doc, "two");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "onetwo" }],
                },
              ],
            },
          ],
        },
      ],
    });
  });

  it("lifts the first task item to a paragraph instead of merging into a previous bullet list", () => {
    const doc = schema.node("doc", null, [
      schema.node("bulletList", null, [
        schema.node("listItem", null, [
          schema.node("paragraph", null, [schema.text("one")]),
        ]),
      ]),
      schema.node("taskList", null, [
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("two")])],
        ),
      ]),
    ]);
    const { state } = runBackspaceAtTextStart(doc, "two");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "bulletList",
          content: [
            {
              type: "listItem",
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "one" }],
                },
              ],
            },
          ],
        },
        {
          type: "paragraph",
          content: [{ type: "text", text: "two" }],
        },
      ],
    });
  });

  it("lifts the first task item to a paragraph when the list starts the doc", () => {
    const doc = schema.node("doc", null, [
      schema.node("taskList", null, [
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("two")])],
        ),
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("three")])],
        ),
      ]),
    ]);
    const { state } = runBackspaceAtTextStart(doc, "two");

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [{ type: "text", text: "two" }],
        },
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "three" }],
                },
              ],
            },
          ],
        },
      ],
    });
  });

  it("joins later task item paragraphs within the same task item", () => {
    const doc = schema.node("doc", null, [
      schema.node("taskList", null, [
        schema.node(
          "taskItem",
          { checked: false },
          [schema.node("paragraph", null, [schema.text("one")])],
        ),
        schema.node(
          "taskItem",
          { checked: false },
          [
            schema.node("paragraph", null, [schema.text("two")]),
            schema.node("paragraph", null, [schema.text("three")]),
          ],
        ),
      ]),
    ]);
    const { state } = runBackspaceAtTextStart(doc, "three", true);

    expect(state.doc.toJSON()).toEqual({
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "one" }],
                },
              ],
            },
            {
              type: "taskItem",
              attrs: { checked: false },
              content: [
                {
                  type: "paragraph",
                  content: [{ type: "text", text: "twothree" }],
                },
              ],
            },
          ],
        },
      ],
    });
  });
});

function runKeyDownAtEnd(
  doc: ReturnType<typeof schema.node>,
  key: string,
  init?: KeyboardEventInit,
) {
  const keymap = buildKeymap();
  let state = EditorState.create({
    schema,
    doc,
    selection: Selection.atEnd(doc),
    plugins: [keymap],
  });
  const view = {
    get state() {
      return state;
    },
    dispatch(tr: Transaction) {
      state = state.apply(tr);
    },
    endOfTextblock: () => false,
  } as Pick<EditorView, "dispatch" | "endOfTextblock" | "state"> as EditorView;
  const handleKeyDown = keymap.props.handleKeyDown;

  const handled = handleKeyDown?.call(
    keymap,
    view,
    new KeyboardEvent("keydown", {
      key,
      ...init,
    }),
  );

  return { handled, state };
}

function runBackspaceAtTextStart(
  doc: ReturnType<typeof schema.node>,
  text: string,
  isEndOfTextblock = false,
) {
  const keymap = buildKeymap();
  const textPos = getTextStartPos(doc, text);
  let state = EditorState.create({
    schema,
    doc,
    selection: TextSelection.create(doc, textPos),
    plugins: [keymap],
  });
  const view = {
    get state() {
      return state;
    },
    dispatch(tr: Transaction) {
      state = state.apply(tr);
    },
    endOfTextblock: () => isEndOfTextblock,
  } as Pick<EditorView, "dispatch" | "endOfTextblock" | "state"> as EditorView;
  const handleKeyDown = keymap.props.handleKeyDown;

  const handled = handleKeyDown?.call(
    keymap,
    view,
    new KeyboardEvent("keydown", { key: "Backspace" }),
  );

  expect(handled).toBe(true);
  return { state };
}

function runTextInput(
  doc: ReturnType<typeof schema.node>,
  text: string,
  pos?: number,
) {
  const inputRules = buildInputRules();
  let state = EditorState.create({
    schema,
    doc,
    selection:
      pos === undefined ? Selection.atEnd(doc) : TextSelection.create(doc, pos),
    plugins: [inputRules],
  });

  const view = {
    composing: false,
    get state() {
      return state;
    },
    dispatch(tr: Transaction) {
      state = state.apply(tr);
    },
  } as Pick<EditorView, "composing" | "dispatch" | "state"> as EditorView;

  const handleTextInput = inputRules.props.handleTextInput as
    | ((
        view: EditorView,
        from: number,
        to: number,
        text: string,
        deflt: () => Transaction,
      ) => boolean | void)
    | undefined;

  const handled = handleTextInput?.(
    view,
    state.selection.from,
    state.selection.to,
    text,
    () => state.tr.insertText(text, state.selection.from, state.selection.to),
  );

  return { handled, state };
}

function getTextStartPos(doc: ReturnType<typeof schema.node>, text: string) {
  let textPos = -1;

  doc.descendants((node, pos) => {
    if (node.isText && node.text === text) {
      textPos = pos;
      return false;
    }

    return undefined;
  });

  if (textPos === -1) {
    throw new Error(`Missing text node: ${text}`);
  }

  return textPos;
}

function emptyDoc() {
  return schema.node("doc", null, [schema.node("paragraph")]);
}

// Type `text` one character at a time, as the view does: the input rules see
// each character first, and a character that no rule takes is inserted.
function typeText(doc: ReturnType<typeof schema.node>, text: string) {
  const inputRules = buildInputRules();
  let state = EditorState.create({
    schema,
    doc,
    selection: Selection.atEnd(doc),
    plugins: [inputRules],
  });
  const view = {
    composing: false,
    get state() {
      return state;
    },
    dispatch(tr: Transaction) {
      state = state.apply(tr);
    },
  } as Pick<EditorView, "composing" | "dispatch" | "state"> as EditorView;
  const handleTextInput = inputRules.props.handleTextInput as (
    view: EditorView,
    from: number,
    to: number,
    text: string,
    deflt: () => Transaction,
  ) => boolean | void;

  for (const char of text) {
    const { from, to } = state.selection;
    const insert = () => state.tr.insertText(char, from, to);
    if (!handleTextInput(view, from, to, char, insert)) {
      view.dispatch(insert());
    }
  }
  return state;
}
