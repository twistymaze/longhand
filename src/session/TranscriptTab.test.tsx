import { describe, expect, test } from "bun:test";
import { render, screen } from "@testing-library/react";
import type { DocLine } from "../bindings";
import { formatClock, TranscriptTab } from "./TranscriptTab";

const turn = (position_ms: number, speaker: number | null, text: string): DocLine => ({
  kind: "turn",
  position_ms,
  speaker,
  text,
});

function renderTab(lines: DocLine[], { hasSpeakers = true, speakers = 3 as number | null, durationMs = 35 * 60_000 + 48_000, transcriptMarkdown = "" } = {}) {
  return render(
    <TranscriptTab lines={lines} hasSpeakers={hasSpeakers} foundSpeakers={speakers} durationMs={durationMs} transcriptMarkdown={transcriptMarkdown} />,
  );
}

describe("TranscriptTab", () => {
  test("consecutive turns of one speaker show one label and one block each", () => {
    renderTab([
      turn(0, 1, "First paragraph."),
      turn(9_000, 1, "Second paragraph."),
      turn(18_000, 1, "Third paragraph."),
      turn(27_000, 2, "A reply."),
    ]);
    expect(screen.getAllByText("Speaker 1")).toHaveLength(1);
    expect(screen.getAllByText("Speaker 2")).toHaveLength(1);
    for (const text of ["First paragraph.", "Second paragraph.", "Third paragraph."]) {
      expect(screen.getByText(text).tagName).toBe("P");
    }
  });

  test("the time of a paragraph is there for the pointer", () => {
    renderTab([turn(0, 1, "Hello."), turn(83_000, 1, "Later.")]);
    expect(screen.getByText("Later.").parentElement?.textContent).toBe("00:01:23Later.");
  });

  // Covers AE8.
  test("a gap with the next segment's start shows when the recording resumed", () => {
    renderTab([
      turn(0, 1, "Before."),
      { kind: "gap", at_ms: 9_000, duration_ms: 3_600_000, wall_start: new Date(2026, 8, 30, 10, 2).toISOString() },
      turn(9_000, 1, "After."),
    ]);
    expect(screen.getByText(/^Resumed (30 Sep|Sep 30), 10:02/)).toBeTruthy();
    // A resume starts a new group, so the speaker label shows again.
    expect(screen.getAllByText("Speaker 1")).toHaveLength(2);
  });

  // Covers AE6.
  test("a pause shows its length in minutes and reads differently from a resume", () => {
    renderTab([
      turn(0, 1, "Before."),
      { kind: "pause", duration_ms: 15 * 60_000 },
      turn(3_900_000, 1, "After the pause."),
      { kind: "gap", at_ms: 6_300_000, duration_ms: 3_600_000, wall_start: new Date(2026, 8, 30, 10, 2).toISOString() },
      turn(6_300_000, 1, "After the resume."),
    ]);
    const pause = screen.getByText("Paused · 15 min");
    const resume = screen.getByText(/^Resumed /);
    // The lines on each side of the label differ.
    expect(pause.firstElementChild?.className).toContain("border-dashed");
    expect(resume.firstElementChild?.className).not.toContain("border-dashed");
  });

  test("a gap with no start time says that time was not recorded", () => {
    renderTab([turn(0, 1, "Before."), { kind: "gap", at_ms: 9_000, duration_ms: 65_000, wall_start: null }]);
    expect(screen.getByText(/Not recorded/)).toBeTruthy();
  });

  test("the header shows the speaker count and the duration", () => {
    renderTab([turn(0, 1, "Hello.")]);
    expect(screen.getByText("3 speakers · 35:48")).toBeTruthy();
  });

  test("a transcript with no speaker marks says why and shows no labels", () => {
    renderTab([turn(0, null, "Hello.")], { hasSpeakers: false, speakers: null });
    expect(screen.getByText(/no speaker marks/).textContent).toContain("this note");
    expect(screen.queryByText(/^Speaker \d/)).toBeNull();
  });

  test("a transcript made with speaker recognition off says so without a failure notice", () => {
    renderTab([turn(0, null, "Hello.")], {
      hasSpeakers: false,
      speakers: null,
      transcriptMarkdown: "# Transcript\n\n> Speaker recognition was off for this transcript.\n\n- [00:00:00] Hello.\n",
    });
    expect(screen.getByText("Speaker recognition was off for this note.")).toBeTruthy();
    expect(screen.queryByText(/Speaker separation failed/)).toBeNull();
    expect(screen.queryByText(/^Speaker \d/)).toBeNull();
  });

  test("words in a transcript turn do not change the speaker failure notice", () => {
    const text = "> Speaker recognition was off for this transcript.";
    renderTab([turn(0, null, text)], {
      hasSpeakers: false,
      speakers: null,
      transcriptMarkdown: `# Transcript\n\n> This transcript carries no speaker marks: speaker separation failed for this session.\n\n- [00:00:00] ${text}\n`,
    });
    expect(screen.getByText("Speaker separation failed for this note, so the transcript has no speaker marks.")).toBeTruthy();
    expect(screen.queryByText("Speaker recognition was off for this note.")).toBeNull();
  });

  test("the tab opens at the end of the transcript", () => {
    const shown: Element[] = [];
    const original = HTMLElement.prototype.scrollIntoView;
    HTMLElement.prototype.scrollIntoView = function (this: HTMLElement) {
      shown.push(this);
    };
    try {
      const lines = Array.from({ length: 200 }, (_, i) => turn(i * 9_000, (i % 3) + 1, `Paragraph ${i}.`));
      renderTab(lines);
    } finally {
      HTMLElement.prototype.scrollIntoView = original;
    }
    expect(shown).toHaveLength(1);
    const last = screen.getByText("Paragraph 199.");
    // The element shown is the end of the transcript: nothing follows it.
    expect(last.compareDocumentPosition(shown[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(shown[0].nextElementSibling).toBeNull();
  });

  // Chromium returns a Promise from scrollIntoView. React must not get that
  // value as the cleanup of the scroll effect, or the page throws and goes
  // blank when the tab closes.
  test("the tab closes with no error when scrollIntoView returns a Promise", () => {
    const original = HTMLElement.prototype.scrollIntoView;
    let scrolled = 0;
    HTMLElement.prototype.scrollIntoView = function () {
      scrolled += 1;
      return Promise.resolve() as unknown as void;
    };
    try {
      const view = renderTab([turn(0, 1, "Hello.")]);
      expect(scrolled).toBe(1);
      expect(() => view.unmount()).not.toThrow();
    } finally {
      HTMLElement.prototype.scrollIntoView = original;
    }
  });
});

describe("formatClock", () => {
  test("shows minutes and seconds, and hours when there are any", () => {
    expect(formatClock(35 * 60_000 + 48_000)).toBe("35:48");
    expect(formatClock(5_000)).toBe("0:05");
    expect(formatClock(4 * 3_600_000 + 2 * 60_000 + 3_000)).toBe("4:02:03");
  });
});
