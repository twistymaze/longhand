import { describe, expect, test } from "bun:test";
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
    expect(list).toMatchObject({ kind: "list", items: [[{ text: "a long item that wraps" }], [{ text: "next" }]] });
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
});
