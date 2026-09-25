import { describe, expect, test } from "bun:test";
import { useEffect, useState } from "react";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { commands, events, type Settings } from "../bindings";
import { Pane } from "../settings/ui";
import { mockTauri } from "./tauri";

function Shortcut() {
  const [shortcut, setShortcut] = useState<string | null>(null);
  useEffect(() => {
    void commands.getSettings().then((s) => setShortcut(s.shortcut));
  }, []);
  return <p>{shortcut ?? "Loading"}</p>;
}

function DownloadModel() {
  const [heard, setHeard] = useState<string | null>(null);
  useEffect(() => {
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => setHeard(payload.model_id));
    return () => void unlisten.then((stop) => stop());
  }, []);
  return <p>{heard ?? "No download"}</p>;
}

function Counter() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount(count + 1)}>Clicked {count}</button>;
}

describe("DOM test environment", () => {
  test("renders a component and finds its text", () => {
    render(<Pane title="General">body</Pane>);
    expect(screen.getByRole("heading", { name: "General" })).toBeTruthy();
    expect(screen.getByText("body")).toBeTruthy();
  });

  test("user-event clicks change the rendered state", async () => {
    render(<Counter />);
    await userEvent.setup().click(screen.getByRole("button"));
    expect(screen.getByRole("button").textContent).toBe("Clicked 1");
  });

  test("a component that calls a mocked command receives the mocked result", async () => {
    const tauri = mockTauri({ get_settings: () => ({ shortcut: "Alt+Space" }) as Partial<Settings> });
    render(<Shortcut />);
    expect(await screen.findByText("Alt+Space")).toBeTruthy();
    expect(tauri.calls).toEqual([{ cmd: "get_settings", args: {} }]);
  });

  test("a command with no handler rejects", async () => {
    mockTauri();
    await expect(commands.getSettings()).rejects.toThrow("no mock for command get_settings");
  });

  test("a mocked event reaches the component listener", async () => {
    const tauri = mockTauri();
    render(<DownloadModel />);
    // The listener registers through the mocked IPC after the first render.
    await act(async () => {});
    await act(() => tauri.emit("model-download-event", { model_id: "whisper", update: null }));
    expect(screen.getByText("whisper")).toBeTruthy();
  });
});
