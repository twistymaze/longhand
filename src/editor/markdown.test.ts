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

// Helpers for documents that the editor makes: a document must come back
// the same after a save and an open.
const t = (text: string, marks?: string[]): JSONContent =>
  marks ? { type: "text", text, marks: marks.map((type) => ({ type })) } : { type: "text", text };
const p = (...content: JSONContent[]): JSONContent =>
  content.length ? { type: "paragraph", content } : { type: "paragraph" };
const li = (...content: JSONContent[]): JSONContent => ({ type: "listItem", content });
const task = (checked: boolean, ...content: JSONContent[]): JSONContent => ({
  type: "taskItem",
  attrs: { checked },
  content,
});
const bullets = (...items: JSONContent[]): JSONContent => ({ type: "bulletList", content: items });
const tasks = (...items: JSONContent[]): JSONContent => ({ type: "taskList", content: items });
const ordered = (...items: JSONContent[]): JSONContent => ({
  type: "orderedList",
  attrs: { start: 1 },
  content: items,
});
const doc = (...content: JSONContent[]): JSONContent => ({ type: "doc", content });

// Save and open the document twice: the JSON stays the same.
function expectStable(json: JSONContent) {
  const once = md2json(json2md(json));
  expect(once).toEqual(json);
  expect(md2json(json2md(once))).toEqual(json);
}

describe("task lists in the file", () => {
  test("keep a nested task list", () => {
    const md = "- [ ] a\n  - [ ] b\n- [ ] c";
    expect(roundTrip(md)).toBe(md);
    expectStable(doc(tasks(task(false, p(t("a")), tasks(task(false, p(t("b"))))), task(false, p(t("c"))))));
  });

  test("keep a bullet list in a task item and a task list in a bullet item", () => {
    expect(roundTrip("- [ ] a\n  - b")).toBe("- [ ] a\n  - b");
    expect(roundTrip("- a\n  - [ ] b")).toBe("- a\n  - [ ] b");
  });

  test("read a list of empty tasks as tasks", () => {
    expect(md2json("- [ ]\n")).toEqual(doc(tasks(task(false, p()))));
    expectStable(doc(tasks(task(false, p()), task(true, p()))));
  });

  test("keep the text [ ] in a bullet item as text", () => {
    expectStable(doc(bullets(li(p(t("[ ]"))))));
  });

  test("do not take [ ] before a letter as a task prefix", () => {
    const json = md2json("- [ ] a\n- [ ]b");
    expect(json).toEqual(doc(tasks(task(false, p(t("a"))), task(false, p(t("[ ]b"))))));
  });
});

describe("text that looks like syntax", () => {
  test("keeps underscores and stars after an escaped backslash", () => {
    expectStable(doc(p(t("\\. _ab_ cd"))));
    expectStable(doc(p(t("\\!\\!\\!*a b c*d e f"))));
  });

  test("keeps a backslash at the end of a text before a mark or a line break", () => {
    expectStable(doc(p(t("a\\"), t("b", ["bold"]))));
    expectStable(doc(p(t("a\\"), t("b", ["italic"]))));
    expectStable(doc(p(t("a\\"), { type: "hardBreak" }, t("b"))));
  });

  test("keeps <br> typed as text", () => {
    expectStable(doc(p(t("use <br> tags"))));
    expectStable(doc(p(t("use <BR /> tags"))));
  });

  test("keeps leading spaces", () => {
    for (const text of [" one", "   three", "    four", "        eight", "\ttab"]) {
      expectStable(doc(p(t(text)), p(t("next"))));
    }
  });

  test("keeps leading spaces before a mark", () => {
    expectStable(doc(p(t("    "), t("x", ["bold"])), p(t("next"))));
  });

  test("writes a line of only spaces as it is", () => {
    expect(json2md(doc(p(t("   "))))).toBe("   ");
  });

  test("keeps line-start syntax after leading spaces as text", () => {
    for (const text of [" ```x", " > x", " - x", "  # x", "    **x**"]) {
      expectStable(doc(p(t(text)), p(t("after"))));
    }
  });

  test("keeps line-start syntax after a line break as text", () => {
    for (const text of ["# bar", "- bar", "> bar", "1. bar", "---", "```"]) {
      expectStable(doc(p(t("foo"), { type: "hardBreak" }, t(text))));
    }
  });
});

describe("adjacent lists", () => {
  test("stay apart", () => {
    expectStable(doc(tasks(task(false, p(t("a")))), bullets(li(p(t("b"))))));
    expectStable(doc(bullets(li(p(t("a")))), tasks(task(false, p(t("b"))))));
    expectStable(doc(bullets(li(p(t("a")))), bullets(li(p(t("b")))), bullets(li(p(t("c"))))));
    expectStable(doc(ordered(li(p(t("a")))), ordered(li(p(t("b"))))));
  });

  test("keep the empty paragraphs between and after them", () => {
    expectStable(doc(bullets(li(p(t("a")))), p(), bullets(li(p(t("b"))))));
    expectStable(doc(bullets(li(p(t("a")))), p(), p(t("c"))));
    expectStable(doc(ordered(li(p(t("a")))), p(), p(), ordered(li(p(t("b"))))));
  });
});

describe("links", () => {
  test("keep an address with a space", () => {
    const json = doc(
      p({
        type: "text",
        text: "link",
        marks: [{ type: "link", attrs: { href: "https://example.com/my file.pdf", target: null } }],
      }),
    );
    const back = md2json(json2md(json));
    expect(back.content![0].content![0].text).toBe("link");
    expect(back.content![0].content![0].marks?.[0].type).toBe("link");
  });
});
