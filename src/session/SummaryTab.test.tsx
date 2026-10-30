import { afterEach, describe, expect, test } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { mockTauri } from "../test/tauri";
import type { SummaryView } from "../bindings";
import { SummaryTab } from "./SummaryTab";

afterEach(() => clearMocks());

describe("SummaryTab links", () => {
  test("a click on a link in a summary opens it in the browser, not in the window", () => {
    const tauri = mockTauri({ "plugin:opener|open_url": () => null });
    const summary = { body: "See https://example.com.", stub: false, model: "m", host: "h", prompt: null };
    render(<SummaryTab summary={summary as unknown as SummaryView} regenerate={null} />);
    const link = screen.getByRole("link", { name: "https://example.com" });
    expect(fireEvent.click(link)).toBe(false);
    expect(tauri.calls.filter((c) => c.cmd === "plugin:opener|open_url").map((c) => c.args.url)).toEqual([
      "https://example.com",
    ]);
  });
});
