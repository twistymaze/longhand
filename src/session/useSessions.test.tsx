import { expect, test } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { mockTauri } from "../test/tauri";
import { useSessions } from "./useSessions";

test("initial loading differs from empty and a failed refresh preserves successful data", async () => {
  let finish!: (value: unknown) => void;
  let fail = false;
  mockTauri({ list_sessions: () => { if (fail) throw "unavailable"; return new Promise((resolve) => { finish = resolve; }); } });
  const { result } = renderHook(() => useSessions());
  expect(result.current.loaded).toBe(false);
  await act(async () => finish([{ id: "a" }]));
  expect(result.current.loaded).toBe(true);
  expect(result.current.entries.map((e) => e.id)).toEqual(["a"]);
  fail = true;
  await act(() => result.current.refresh());
  expect(result.current.entries.map((e) => e.id)).toEqual(["a"]);
  expect(result.current.loadError).toBe("Could not load notes.");
});

test("a failed initial load can retry and an older response cannot overwrite a newer read", async () => {
  let fail = true;
  const pending: ((value: unknown) => void)[] = [];
  mockTauri({ list_sessions: () => { if (fail) throw "unavailable"; return new Promise((resolve) => pending.push(resolve)); } });
  const { result } = renderHook(() => useSessions());
  await act(async () => {});
  expect(result.current.loaded).toBe(false);
  expect(result.current.loadError).toBe("Could not load notes.");
  fail = false;
  let older!: Promise<void>;
  let newer!: Promise<void>;
  act(() => { older = result.current.refresh(); newer = result.current.refresh(); });
  await act(async () => { pending[1]([{ id: "b" }]); await newer; });
  await act(async () => { pending[0]([{ id: "a" }]); await older; });
  expect(result.current.entries.map((e) => e.id)).toEqual(["b"]);
  expect(result.current.loadError).toBeNull();
});

const started = (id: string, stage: string) => ({ note_id: id, update: { kind: "stage_started", stage } });

test("a stage start sets the stage and counts one start", async () => {
  const tauri = mockTauri({ list_sessions: () => [{ id: "a", state: "processing" }] });
  const { result } = renderHook(() => useSessions());
  await act(async () => {});
  await act(() => tauri.emit("processing-update", started("a", "transcribe")));
  expect(result.current.stages.a).toBe("transcribe");
  expect(result.current.starts.a).toBe(1);
  await act(() => tauri.emit("processing-update", started("a", "summarize")));
  expect(result.current.stages.a).toBe("summarize");
  expect(result.current.starts.a).toBe(2);
});

test("a wait removes the stage of the note", async () => {
  const tauri = mockTauri({ list_sessions: () => [{ id: "a", state: "processing" }] });
  const { result } = renderHook(() => useSessions());
  await act(async () => {});
  await act(() => tauri.emit("processing-update", started("a", "transcribe")));
  expect(result.current.stages.a).toBe("transcribe");
  await act(() => tauri.emit("processing-update", { note_id: "a", update: { kind: "waits", wait: { kind: "model" } } }));
  expect("a" in result.current.stages).toBe(false);
});

test("a refresh drops the stage of a note that is not processing", async () => {
  let state = "processing";
  const tauri = mockTauri({ list_sessions: () => [{ id: "a", state }] });
  const { result } = renderHook(() => useSessions());
  await act(async () => {});
  await act(() => tauri.emit("processing-update", started("a", "transcribe")));
  expect(result.current.stages.a).toBe("transcribe");
  state = "ready";
  await act(() => result.current.refresh());
  expect("a" in result.current.stages).toBe(false);
});
