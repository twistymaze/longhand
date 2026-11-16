import { afterEach, describe, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";

import { imeKeyDown } from "../test/keys";
import { mockTauri } from "../test/tauri";
import { NewTypeDialog } from "./NewTypeDialog";

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("NewTypeDialog", () => {
  test("the Enter that ends an input method composition does not create the type", async () => {
    const tauri = mockTauri({ add_note_type: (args) => ({ types: [{ name: args.name }] }) });
    const onCreated = vi.fn((_name: string) => {});
    render(<NewTypeDialog onCreated={onCreated} onCancel={() => {}} />);
    const field = screen.getByRole("textbox", { name: "Type name" });
    fireEvent.change(field, { target: { value: "面談" } });
    imeKeyDown(field, "Enter");
    await act(async () => {});
    expect(tauri.calls.some((c) => c.cmd === "add_note_type")).toBe(false);
    fireEvent.keyDown(field, { key: "Enter", keyCode: 13 });
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith("面談", expect.anything()));
  });

  test("the Escape that cancels a composition does not close the dialog", () => {
    mockTauri({});
    const onCancel = vi.fn(() => {});
    render(<NewTypeDialog onCreated={() => {}} onCancel={onCancel} />);
    const field = screen.getByRole("textbox", { name: "Type name" });
    fireEvent.change(field, { target: { value: "面談" } });
    imeKeyDown(field, "Escape");
    expect(onCancel).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: "Escape", keyCode: 27 });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
