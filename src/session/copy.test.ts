import { describe, expect, test, vi } from "vitest";
import type { SessionDocument } from "../bindings";
import { copyText, tabHtml, tabMarkdown } from "./copy";

const doc: SessionDocument = {
  has_speakers: true,
  lines: [{ kind: "turn", position_ms: 0, speaker: 1, text: "Hello." }],
  transcript_markdown: "# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n",
  summary: { body: "## Overview\n\nWe agreed.", stub: false, host: "h", model: "m", prompt: null, prompt_id: null },
};

describe("tabMarkdown", () => {
  test("the Transcript tab gives the transcript.md text", () => {
    expect(tabMarkdown("transcript", doc, "# Notes")).toBe("# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n");
  });

  test("My notes gives the notes document", () => {
    expect(tabMarkdown("notes", doc, "# Notes\n\n- ask about headcount")).toBe("# Notes\n\n- ask about headcount");
    // Notes exist before the session is saved.
    expect(tabMarkdown("notes", null, "- first")).toBe("- first");
  });

  test("the Summary tab gives the summary body", () => {
    expect(tabMarkdown("summary", doc, "")).toBe("## Overview\n\nWe agreed.");
  });

  test("a tab with nothing to copy gives null", () => {
    expect(tabMarkdown("transcript", null, "x")).toBeNull();
    expect(tabMarkdown("summary", { ...doc, summary: null }, "x")).toBeNull();
    expect(tabMarkdown("notes", doc, null)).toBeNull();
    expect(tabMarkdown("notes", doc, "  \n")).toBeNull();
  });
});

describe("tabHtml", () => {
  test("the summary gives headings, bold and nested lists", () => {
    expect(tabHtml("summary", "## Overview\n\nWe **agreed**.\n\n- one\n  - two")).toBe(
      "<h2>Overview</h2>\n<p>We <strong>agreed</strong>.</p>\n<ul><li>one<ul><li>two</li></ul></li></ul>\n",
    );
  });

  test("the summary keeps the nesting that the Summary tab shows", () => {
    // CommonMark would close the ordered list here; the tab nests the bullet.
    expect(tabHtml("summary", "1. **Budget:**\n  - detail")).toBe(
      "<ol><li><strong>Budget:</strong><ul><li>detail</li></ul></li></ol>\n",
    );
  });

  test("the transcript gives a heading and a list item with the time and the speaker", () => {
    expect(tabHtml("transcript", "# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n")).toBe(
      "<h1>Transcript</h1>\n<ul>\n<li>[00:00:00] Speaker 1: Hello.</li>\n</ul>\n",
    );
  });

  test("notes give strikethrough and highlight", () => {
    expect(tabHtml("notes", "~~old~~ and ==key==")).toBe("<p><s>old</s> and <mark>key</mark></p>\n");
  });

  test("raw HTML stays text", () => {
    for (const tab of ["summary", "transcript", "notes"] as const) {
      const html = tabHtml(tab, "<script>alert(1)</script> <b>x</b>");
      expect(html).not.toContain("<script>");
      expect(html).not.toContain("<b>");
      expect(html).toContain("&lt;script&gt;");
    }
  });

  test("a javascript address is not a link, and a web address is", () => {
    expect(tabHtml("notes", "[x](javascript:alert(1))")).not.toContain("<a");
    expect(tabHtml("notes", "[site](https://example.com)")).toContain('<a href="https://example.com">site</a>');
  });

  test("an image is not loaded", () => {
    // As in the editor, the image rule is off, so the text is "!" and a link.
    expect(tabHtml("notes", "![x](https://example.com/a.png)")).toBe(
      '<p>!<a href="https://example.com/a.png">x</a></p>\n',
    );
  });

  test("the HTML has no style or class", () => {
    for (const tab of ["summary", "transcript", "notes"] as const) {
      const html = tabHtml(tab, "# A\n\n- **b** `c`\n\n~~d~~ ==e==\n\n```js\nf()\n```");
      expect(html).not.toMatch(/\s(style|class)=/);
    }
  });
});

describe("copyText", () => {
  // A ClipboardItem as WebKit makes it: the types in their order, and a blob for each.
  class FakeClipboardItem {
    readonly types: string[];
    constructor(readonly items: Record<string, Blob>) {
      this.types = Object.keys(items);
    }
  }
  const refuse = async () => {
    throw new Error("denied");
  };
  const setClipboard = (clipboard: object, item: unknown = FakeClipboardItem) => {
    Object.defineProperty(navigator, "clipboard", { value: clipboard, configurable: true });
    Object.defineProperty(globalThis, "ClipboardItem", { value: item, configurable: true, writable: true });
  };

  test("writes one item with the HTML first and the markdown as plain text", async () => {
    const write = vi.fn(async (_items: FakeClipboardItem[]) => {});
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ write, writeText });
    expect(await copyText("# A", "<h1>A</h1>\n")).toBe(true);
    const [item] = write.mock.calls[0][0];
    expect(item.types).toEqual(["text/html", "text/plain"]);
    expect(await item.items["text/html"].text()).toBe("<h1>A</h1>\n");
    expect(await item.items["text/plain"].text()).toBe("# A");
    expect(writeText).not.toHaveBeenCalled();
  });

  test("writes the markdown only when the clipboard refuses the item", async () => {
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ write: refuse, writeText });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("# A");
  });

  test("writes the markdown only when there is no ClipboardItem", async () => {
    const writeText = vi.fn(async (_t: string) => {});
    setClipboard({ writeText }, undefined);
    expect(await copyText("# A", "<h1>A</h1>")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("# A");
  });

  test("reports a clipboard that refuses both", async () => {
    setClipboard({ write: refuse, writeText: refuse });
    expect(await copyText("# A", "<h1>A</h1>")).toBe(false);
  });
});
