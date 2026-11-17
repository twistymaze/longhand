import { afterEach, describe, expect, test, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Prompt } from "../bindings";
import { mockTauri } from "../test/tauri";
import { StillRecordingBar, formatCountdown, promptReason } from "./StillRecording";

const NOW = Date.UTC(2026, 10, 16, 15, 30, 0);

const prompt: Prompt = {
  note_id: "rec",
  generation: 7,
  silence_check_minutes: 30,
  time_limit_hours: null,
  deadline_ms: NOW + 120_000,
  countdown_minutes: 2,
};

describe("the countdown text", () => {
  test("is m:ss of the time to the deadline, rounded up to the next second", () => {
    expect(formatCountdown(NOW + 120_000, NOW)).toBe("2:00");
    expect(formatCountdown(NOW + 61_500, NOW)).toBe("1:02");
    expect(formatCountdown(NOW + 9_001, NOW)).toBe("0:10");
    expect(formatCountdown(NOW + 600_000, NOW)).toBe("10:00");
  });

  test("stays at 0:00 after the deadline", () => {
    expect(formatCountdown(NOW, NOW)).toBe("0:00");
    expect(formatCountdown(NOW - 5_000, NOW)).toBe("0:00");
  });
});

describe("the reason of the prompt", () => {
  test("names each check in the prompt", () => {
    expect(promptReason(prompt)).toBe("No speech for 30 minutes.");
    expect(promptReason({ ...prompt, silence_check_minutes: null, time_limit_hours: 6 })).toBe("Recording for 6 hours.");
    expect(promptReason({ ...prompt, silence_check_minutes: 1, time_limit_hours: 1 })).toBe(
      "No speech for 1 minute. Recording for 1 hour.",
    );
  });
});

describe("the main-window prompt (KTD12)", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  test("the live region does not change while the countdown runs, and the countdown changes each second", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(NOW);
    mockTauri({});
    render(<StillRecordingBar prompt={prompt} />);
    const live = screen.getByRole("status");
    const text = live.textContent;
    expect(text).toContain("Keep recording?");
    expect(text).toContain("2 minutes");
    expect(text).not.toContain("2:00");
    expect(screen.getByText("2:00")).toBeTruthy();
    await act(async () => {
      vi.advanceTimersByTime(1_000);
    });
    expect(screen.getByText("1:59")).toBeTruthy();
    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getByText("0:59")).toBeTruthy();
    expect(live.textContent).toBe(text);
    // The m:ss text is outside the live region, and a screen reader skips it.
    expect(screen.getByText("0:59").closest("[aria-hidden=true]")).not.toBeNull();
  });

  test("takes no focus and has no prominent button", () => {
    mockTauri({});
    const before = document.activeElement;
    render(<StillRecordingBar prompt={prompt} />);
    expect(document.activeElement).toBe(before);
    for (const name of ["Keep Recording", "Stop"]) {
      const button = screen.getByRole("button", { name });
      expect(button.className).not.toContain("bg-accent");
      expect(button.hasAttribute("autofocus")).toBe(false);
    }
  });

  test("each button calls its command with the generation of the prompt", async () => {
    const tauri = mockTauri({ keep_recording: () => null, stop_from_prompt: () => null });
    const answered = vi.fn();
    render(<StillRecordingBar prompt={prompt} onAnswered={answered} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Keep Recording" }));
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(tauri.calls.map((c) => [c.cmd, c.args])).toEqual([
      ["keep_recording", { generation: 7 }],
      ["stop_from_prompt", { generation: 7 }],
    ]);
    expect(answered).toHaveBeenCalledTimes(2);
  });
});
