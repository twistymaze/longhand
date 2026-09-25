import { describe, expect, test } from "bun:test";
import { Node as PMNode } from "prosemirror-model";

import { json2md, type JSONContent, markdownSchema, md2json } from "./markdown";
import { escapeText } from "./markdown/serializer";
import { schema as noteSchema } from "./note/schema";

// Parse, load into the editor schema, and serialize: the path that the editor
// takes when it opens and saves a document.
function roundTrip(markdown: string): string {
  const doc = PMNode.fromJSON(noteSchema, md2json(markdown));
  return json2md(doc.toJSON() as JSONContent);
}

const FULL_DOCUMENT = `# Meeting

## Actions

- first bullet
- second bullet
  - nested bullet

1. first step
2. second step

- [ ] open task
- [x] done task

> a quote

\`\`\`ts
const x = 1;
\`\`\`

---

**bold** *italic* ~~strike~~ ==highlight== [a link](https://example.com) ==**bold highlight**==`;

describe("round trip", () => {
  test("keeps every supported block and mark unchanged, twice", () => {
    const once = roundTrip(FULL_DOCUMENT);
    expect(once).toBe(FULL_DOCUMENT);
    expect(roundTrip(once)).toBe(FULL_DOCUMENT);
  });

  test("keeps a tight list tight", () => {
    const markdown = "- one\n- two\n- three";
    expect(roundTrip(markdown)).toBe(markdown);
  });

  test("keeps plain text as typed", () => {
    for (const text of [
      "- [00:12:40] ask about headcount",
      "5 * 3 = 15 for snake_case names",
      "AT&T and 1 + 1, x ~ y",
      "see [brackets] and <angle> text",
      "`code` stays text",
      "-5 degrees and #hashtag",
    ]) {
      expect(roundTrip(text)).toBe(text);
    }
  });

  test("keeps text that looks like syntax as text", () => {
    for (const text of [
      "*not italic*",
      "==not highlight==",
      "~~not strike~~",
      "[not](a link)",
      "# not a heading",
      "1. not a list",
      "- not a bullet",
      "> not a quote",
      "---",
      "&amp; not an entity",
      "back\\slash \\* here",
    ]) {
      const json: JSONContent = {
        type: "doc",
        content: [{ type: "paragraph", content: [{ type: "text", text }] }],
      };
      expect(md2json(json2md(json))).toEqual(json);
    }
  });

  test("keeps empty paragraphs", () => {
    const json: JSONContent = {
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "first" }] },
        { type: "paragraph" },
        { type: "paragraph" },
        { type: "paragraph", content: [{ type: "text", text: "second" }] },
      ],
    };
    expect(md2json(json2md(json))).toEqual(json);
  });
});

describe("md2json", () => {
  test("reads an old timestamped note as a bullet with its text (AE7)", () => {
    expect(md2json("- [00:12:40] ask about headcount")).toEqual({
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
                  content: [
                    { type: "text", text: "[00:12:40] ask about headcount" },
                  ],
                },
              ],
            },
          ],
        },
      ],
    });
  });

  test("reads marks nested inside highlight and strike", () => {
    const json = md2json("==**a**== ~~*b*~~");
    expect(json.content![0].content).toEqual([
      { type: "text", text: "a", marks: [{ type: "highlight" }, { type: "bold" }] },
      { type: "text", text: " " },
      { type: "text", text: "b", marks: [{ type: "strike" }, { type: "italic" }] },
    ]);
  });

  test("reads checked and unchecked tasks", () => {
    const json = md2json("- [ ] Task 1\n- [x] Task 2");
    const items = json.content![0].content!;
    expect(json.content![0].type).toBe("taskList");
    expect(items.map((item) => item.attrs?.checked)).toEqual([false, true]);
  });

  test("keeps the language of a fenced code block", () => {
    const json = md2json("```rust\nfn main() {}\n```");
    expect(json.content![0]).toEqual({
      type: "codeBlock",
      attrs: { language: "rust" },
      content: [{ type: "text", text: "fn main() {}" }],
    });
  });

  test("gives one empty paragraph for empty markdown", () => {
    expect(md2json("")).toEqual({ type: "doc", content: [{ type: "paragraph" }] });
  });
});

describe("json2md", () => {
  test("writes a checked task as - [x]", () => {
    const markdown = json2md({
      type: "doc",
      content: [
        {
          type: "taskList",
          content: [
            {
              type: "taskItem",
              attrs: { checked: true },
              content: [
                { type: "paragraph", content: [{ type: "text", text: "done" }] },
              ],
            },
          ],
        },
      ],
    });
    expect(markdown).toBe("- [x] done");
  });

  test("writes an empty paragraph as an extra blank line", () => {
    const markdown = json2md({
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "a" }] },
        { type: "paragraph" },
        { type: "paragraph", content: [{ type: "text", text: "b" }] },
      ],
    });
    expect(markdown).toBe("a\n\n\nb");
  });
});

describe("escapeText", () => {
  test("escapes a bracket only where it would start a link", () => {
    expect(escapeText("[00:12:40] note", true)).toBe("[00:12:40] note");
    expect(escapeText("[a](b)", false)).toBe("[a\\](b)");
  });
});

describe("schema mirror", () => {
  // A node or mark in one schema without its mirror in the other makes
  // PMNode.fromJSON throw, and json2md then returns "". The mark order sets
  // which mark is outer in the markdown, so it must match too.
  test("the editor and markdown schemas have the same nodes and marks", () => {
    expect(Object.keys(noteSchema.nodes).sort()).toEqual(
      Object.keys(markdownSchema.nodes).sort(),
    );
    expect(Object.keys(noteSchema.marks)).toEqual(
      Object.keys(markdownSchema.marks),
    );
  });
});
