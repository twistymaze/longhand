import { describe, expect, test } from "vitest";
import { parseInline, parseMarkdown } from "./markdown";

describe("summary markdown", () => {
  test("headings, lists and paragraphs become blocks", () => {
    const blocks = parseMarkdown("# Summary\n\nThe party met.\nThey argued.\n\n- one\n- two\n\n1. first\n2. second");
    expect(blocks.map((b) => b.kind)).toEqual(["heading", "paragraph", "list", "list"]);
    expect(blocks[1]).toEqual({ kind: "paragraph", content: [{ kind: "text", text: "The party met. They argued." }] });
    expect(blocks[3]).toMatchObject({ kind: "list", ordered: true });
  });

  test("a wrapped list item stays one item", () => {
    const [list] = parseMarkdown("- a long item\n  that wraps\n- next");
    expect(list).toMatchObject({
      kind: "list",
      items: [{ content: [{ text: "a long item that wraps" }] }, { content: [{ text: "next" }] }],
    });
  });

  // Covers AE1.
  test("a bold-label bullet with indented children gives one item with a child list", () => {
    const blocks = parseMarkdown(
      [
        "## Launch Plan",
        "",
        "The team set the launch plan.",
        "",
        "*   **Budget:** costs and staff",
        "    *   $40,000 for ads",
        "    *   two contractors",
        "    *   one new laptop",
        "*   **Decision:** launch on May 12",
      ].join("\n"),
    );
    expect(blocks.map((b) => b.kind)).toEqual(["heading", "paragraph", "list"]);
    const list = blocks[2];
    if (list.kind !== "list") throw new Error("not a list");
    expect(list.items).toHaveLength(2);
    expect(list.items[0].content).toEqual([
      { kind: "strong", text: "Budget:" },
      { kind: "text", text: " costs and staff" },
    ]);
    expect(list.items[0].children).toHaveLength(1);
    expect(list.items[0].children[0]).toMatchObject({
      kind: "list",
      ordered: false,
      items: [
        { content: [{ text: "$40,000 for ads" }], children: [] },
        { content: [{ text: "two contractors" }], children: [] },
        { content: [{ text: "one new laptop" }], children: [] },
      ],
    });
    expect(list.items[1]).toMatchObject({ content: [{ text: "Decision:" }, { text: " launch on May 12" }], children: [] });
  });

  test("a wrapped, indented continuation line joins its item and starts no child list", () => {
    const [list] = parseMarkdown("- parent item\n    - child item\n      that wraps\n  parent text again\n- next");
    expect(list).toMatchObject({
      kind: "list",
      items: [
        {
          content: [{ text: "parent item parent text again" }],
          children: [{ items: [{ content: [{ text: "child item that wraps" }], children: [] }] }],
        },
        { content: [{ text: "next" }], children: [] },
      ],
    });
  });

  test("a numbered list inside a bullet list is an ordered child", () => {
    const [list] = parseMarkdown("- steps\n    1. first\n    2. second\n- after");
    expect(list).toMatchObject({
      kind: "list",
      ordered: false,
      items: [
        { content: [{ text: "steps" }], children: [{ ordered: true, items: [{ content: [{ text: "first" }] }, { content: [{ text: "second" }] }] }] },
        { content: [{ text: "after" }] },
      ],
    });
  });

  test("a bullet list inside a numbered list is an unordered child", () => {
    const blocks = parseMarkdown("1. one\n   - a\n   - b\n2. two");
    expect(blocks).toHaveLength(1);
    expect(blocks[0]).toMatchObject({
      kind: "list",
      ordered: true,
      items: [
        { content: [{ text: "one" }], children: [{ ordered: false, items: [{ content: [{ text: "a" }] }, { content: [{ text: "b" }] }] }] },
        { content: [{ text: "two" }], children: [] },
      ],
    });
  });

  test("lists nest three levels deep and a shallower bullet closes the deeper lists", () => {
    const [list] = parseMarkdown("- a\n    - b\n        - c\n    - d\n- e");
    expect(list).toMatchObject({
      items: [
        {
          content: [{ text: "a" }],
          children: [
            {
              items: [
                { content: [{ text: "b" }], children: [{ items: [{ content: [{ text: "c" }], children: [] }] }] },
                { content: [{ text: "d" }], children: [] },
              ],
            },
          ],
        },
        { content: [{ text: "e" }], children: [] },
      ],
    });
  });

  test("a tab counts as four spaces of indent", () => {
    const [list] = parseMarkdown("- a\n\t- b\n- c");
    expect(list).toMatchObject({
      items: [{ content: [{ text: "a" }], children: [{ items: [{ content: [{ text: "b" }] }] }] }, { content: [{ text: "c" }] }],
    });
  });

  test("a blank line between nested items keeps them in one child list", () => {
    const blocks = parseMarkdown("- a\n    - x\n\n    - y\n- b\n\nAfter the list.");
    expect(blocks.map((b) => b.kind)).toEqual(["list", "paragraph"]);
    expect(blocks[0]).toMatchObject({
      items: [
        { content: [{ text: "a" }], children: [{ items: [{ content: [{ text: "x" }] }, { content: [{ text: "y" }] }] }] },
        { content: [{ text: "b" }], children: [] },
      ],
    });
  });

  test("a heading after a list closes the list", () => {
    const blocks = parseMarkdown("- a\n    - b\n## Next\n- c");
    expect(blocks.map((b) => b.kind)).toEqual(["list", "heading", "list"]);
  });

  test("bold, italic and code runs are kept and everything else is text", () => {
    expect(parseInline("a **b** *c* `d` e")).toEqual([
      { kind: "text", text: "a " },
      { kind: "strong", text: "b" },
      { kind: "text", text: " " },
      { kind: "em", text: "c" },
      { kind: "text", text: " " },
      { kind: "code", text: "d" },
      { kind: "text", text: " e" },
    ]);
  });

  test("markup that is HTML stays text", () => {
    expect(parseInline("<img src=x onerror=alert(1)>")).toEqual([
      { kind: "text", text: "<img src=x onerror=alert(1)>" },
    ]);
  });

  test("a lone asterisk is not emphasis", () => {
    expect(parseInline("3 * 4 = 12")).toEqual([{ kind: "text", text: "3 * 4 = 12" }]);
  });

  test("underscores inside words stay text", () => {
    for (const text of ["Check user_id and order_id.", "a__b and c__d", "file_name_v2.txt"]) {
      expect(parseInline(text)).toEqual([{ kind: "text", text }]);
    }
  });

  test("underscores around words are still emphasis", () => {
    expect(parseInline("say _hi_ now")).toEqual([
      { kind: "text", text: "say " },
      { kind: "em", text: "hi" },
      { kind: "text", text: " now" },
    ]);
    expect(parseInline("__bold__ end")).toEqual([
      { kind: "strong", text: "bold" },
      { kind: "text", text: " end" },
    ]);
  });
});

describe("summary links", () => {
  test("a markdown link becomes a link", () => {
    expect(parseInline("See [the plan](https://example.com/plan) now")).toEqual([
      { kind: "text", text: "See " },
      { kind: "link", text: "the plan", href: "https://example.com/plan" },
      { kind: "text", text: " now" },
    ]);
  });

  test("a bare web address becomes a link without the punctuation after it", () => {
    expect(parseInline("Link used: https://example.com, shown")).toEqual([
      { kind: "text", text: "Link used: " },
      { kind: "link", text: "https://example.com", href: "https://example.com" },
      { kind: "text", text: ", shown" },
    ]);
  });

  test("a file name or an address in code is not a link", () => {
    expect(parseInline("Read README.md")).toEqual([{ kind: "text", text: "Read README.md" }]);
    expect(parseInline("`https://example.com`")).toEqual([{ kind: "code", text: "https://example.com" }]);
  });

  test("a markdown link to another kind of address keeps only its text", () => {
    expect(parseInline("[hosts](file:///etc/hosts) and [x](javascript:alert(1))")).toEqual([
      { kind: "text", text: "hosts" },
      { kind: "text", text: " and " },
      { kind: "text", text: "x" },
      { kind: "text", text: ")" },
    ]);
  });
});
