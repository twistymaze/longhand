import { describe, expect, test } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { useState } from "react";
import { type TauriMock, mockTauri } from "../test/tauri";
import { type CommitHow, CommitField } from "./ui";

/// The commits that the field sent, and a way to make them wait or fail.
function commits() {
  const calls: { text: string; how: CommitHow }[] = [];
  const control = {
    result: true,
    hold: null as Promise<void> | null,
    // The value that the parent shows after a commit that saved, or null to
    // show the committed text.
    normalize: null as ((text: string) => string) | null,
  };
  return { calls, control };
}

let setShown: (v: string) => void = () => {};

function Harness({ initial, c }: { initial: string; c: ReturnType<typeof commits> }) {
  const [value, setValue] = useState(initial);
  setShown = setValue;
  return (
    <CommitField
      label="Name"
      value={value}
      onCommit={async (text, how) => {
        c.calls.push({ text, how });
        if (c.control.hold) await c.control.hold;
        if (c.control.result) setValue(c.control.normalize ? c.control.normalize(text) : text);
        return c.control.result;
      }}
    />
  );
}

function setup(initial = "A") {
  const tauri = mockTauri({ "plugin:window|destroy": () => null, log_from_webview: () => null });
  mockWindows("settings");
  const c = commits();
  const view = render(<Harness initial={initial} c={c} />);
  return { tauri, c, view, user: userEvent.setup() };
}

const field = () => screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement;
const destroyed = (tauri: TauriMock) => tauri.calls.some((c) => c.cmd === "plugin:window|destroy");
const hold = (c: ReturnType<typeof commits>) => {
  let release: () => void = () => {};
  c.control.hold = new Promise<void>((r) => (release = r));
  return () => act(async () => release());
};

describe("CommitField", () => {
  test("a window close saves the typed text and waits for the save", async () => {
    const { tauri, c, user } = setup();
    const release = hold(c);
    await user.type(field(), "B");
    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(c.calls).toEqual([{ text: "AB", how: "unattended" }]));
    expect(destroyed(tauri)).toBe(false);
    await release();
    await waitFor(() => expect(destroyed(tauri)).toBe(true));
  });

  test("an unmount saves the typed text", async () => {
    const { c, view, user } = setup();
    await user.type(field(), "B");
    view.unmount();
    await waitFor(() => expect(c.calls).toEqual([{ text: "AB", how: "unattended" }]));
  });

  test("a new value from another computer keeps the text that the user types", async () => {
    const { c, user } = setup();
    await user.type(field(), "B");
    act(() => setShown("Remote"));
    expect(field().value).toBe("AB");
    expect(c.calls).toEqual([]);
  });

  test("a new value with no edit shows the new value", async () => {
    setup();
    act(() => setShown("Remote"));
    expect(field().value).toBe("Remote");
  });

  test("a saved commit shows the value that the parent saved", async () => {
    const { c, user } = setup();
    c.control.normalize = (text) => text.trim();
    await user.type(field(), "B {Enter}");
    await waitFor(() => expect(field().value).toBe("AB"));
  });

  test("a failed commit keeps the text against a new value from another computer", async () => {
    const { c, user } = setup();
    c.control.result = false;
    await user.type(field(), "B{Enter}");
    await waitFor(() => expect(c.calls).toEqual([{ text: "AB", how: "user" }]));
    act(() => setShown("Remote"));
    expect(field().value).toBe("AB");
  });

  test("Return after a failed commit tries again", async () => {
    const { c, user } = setup();
    c.control.result = false;
    await user.type(field(), "B{Enter}");
    await waitFor(() => expect(c.calls).toHaveLength(1));
    await user.keyboard("{Enter}");
    await waitFor(() => expect(c.calls).toHaveLength(2));
  });

  test("a blur and then a window close send the text once", async () => {
    const { tauri, c, user } = setup();
    const release = hold(c);
    await user.type(field(), "B");
    act(() => void fireEvent.blur(field()));
    await tauri.emit("tauri://close-requested");
    await release();
    await waitFor(() => expect(destroyed(tauri)).toBe(true));
    expect(c.calls).toEqual([{ text: "AB", how: "user" }]);
  });

  test("text typed during a commit is sent after it", async () => {
    const { c, user } = setup();
    const release = hold(c);
    await user.type(field(), "B{Enter}");
    await waitFor(() => expect(c.calls).toHaveLength(1));
    await user.type(field(), "C{Enter}");
    await release();
    await waitFor(() =>
      expect(c.calls).toEqual([
        { text: "AB", how: "user" },
        { text: "ABC", how: "user" },
      ]),
    );
    await waitFor(() => expect(field().value).toBe("ABC"));
  });
});
