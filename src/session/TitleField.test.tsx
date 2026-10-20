import { afterEach, describe, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";

import { mockTauri } from "../test/tauri";
import { TitleField } from "./TitleField";

afterEach(() => {
  cleanup();
  clearMocks();
});

function setup(title: string | null) {
  const tauri = mockTauri({ rename_session: () => ({ kind: "later" }) });
  const onRenamed = vi.fn(() => {});
  const view = render(<TitleField sessionId="s1" title={title} placeholder="Start" onRenamed={onRenamed} />);
  const field = () => screen.getByLabelText("Title") as HTMLInputElement;
  const rerender = (next: string | null) =>
    view.rerender(<TitleField sessionId="s1" title={next} placeholder="Start" onRenamed={onRenamed} />);
  const renames = () => tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args);
  return { field, rerender, renames, onRenamed };
}

describe("TitleField", () => {
  test("a focus with no typing does not rename, and a new title shows", async () => {
    const { field, rerender, renames } = setup("Old");
    act(() => field().focus());
    rerender("New from elsewhere");
    expect(field().value).toBe("New from elsewhere");
    act(() => field().blur());
    // A rename is a command call: give it the time to start.
    await act(async () => {});
    expect(renames()).toEqual([]);
    expect(field().value).toBe("New from elsewhere");
  });

  test("a proposed title for a note with no title stays after a blur", async () => {
    const { field, rerender, renames } = setup(null);
    act(() => field().focus());
    rerender("Proposed");
    act(() => field().blur());
    await act(async () => {});
    expect(renames()).toEqual([]);
    expect(field().value).toBe("Proposed");
  });

  test("typed text still renames, also when the title changes during the typing", async () => {
    const { field, rerender, renames, onRenamed } = setup("Old");
    act(() => field().focus());
    fireEvent.change(field(), { target: { value: "Typed" } });
    rerender("New from elsewhere");
    expect(field().value).toBe("Typed");
    act(() => field().blur());
    await waitFor(() => expect(onRenamed).toHaveBeenCalled());
    expect(renames()).toEqual([{ sessionId: "s1", title: "Typed" }]);
  });

  test("Escape puts the title back and does not rename", async () => {
    const { field, renames } = setup("Old");
    act(() => field().focus());
    fireEvent.change(field(), { target: { value: "Typed" } });
    fireEvent.keyDown(field(), { key: "Escape" });
    await act(async () => {});
    expect(renames()).toEqual([]);
    expect(field().value).toBe("Old");
  });
});
