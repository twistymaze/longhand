import { describe, expect, mock, test } from "bun:test";
import type { SessionDocument } from "../bindings";
import { copyText, tabMarkdown } from "./copy";

const doc: SessionDocument = {
  has_speakers: true,
  lines: [{ kind: "turn", position_ms: 0, speaker: 1, text: "Hello." }],
  transcript_markdown: "# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n",
  summary: { body: "## Overview\n\nWe agreed.", stub: false, host: "h", model: "m", prompt: null },
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

describe("copyText", () => {
  test("writes the text to the clipboard and reports success", async () => {
    const writeText = mock(async (_t: string) => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    expect(await copyText("abc")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("abc");
  });

  test("reports a clipboard that refuses", async () => {
    const writeText = mock(async (_t: string) => {
      throw new Error("denied");
    });
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    expect(await copyText("abc")).toBe(false);
  });
});
