import { expect, test } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { Settings } from "../bindings";
import { mockTauri } from "../test/tauri";
import { useAppSettings } from "./useAppSettings";

test("a late settings read cannot replace a newer refresh", async () => {
  const old = { asr_model: "old" } as Settings;
  const current = { asr_model: "current" } as Settings;
  let release!: (settings: Settings) => void;
  let reads = 0;
  mockTauri({
    get_settings: () => {
      if (++reads === 2) return new Promise<Settings>((done) => { release = done; });
      return reads === 1 ? old : current;
    },
    readiness: () => ({ armed: true }),
    model_statuses: () => [],
  });
  const { result } = renderHook(useAppSettings);
  await waitFor(() => expect(result.current.settings).toEqual(old));
  let previous!: Promise<void>;
  act(() => { previous = result.current.refresh(); });
  await act(async () => { await result.current.refresh(); });
  expect(result.current.settings).toEqual(current);
  await act(async () => {
    release(old);
    await previous;
  });
  expect(result.current.settings).toEqual(current);
});

test("a late read failure cannot replace a successful refresh", async () => {
  const saved = { asr_model: "saved" } as Settings;
  let reject!: (error: Error) => void;
  let reads = 0;
  mockTauri({
    get_settings: () => ++reads === 2
      ? new Promise<Settings>((_, fail) => { reject = fail; })
      : saved,
    readiness: () => ({ armed: true }),
    model_statuses: () => [],
  });
  const { result } = renderHook(useAppSettings);
  await waitFor(() => expect(result.current.settings).toEqual(saved));
  let previous!: Promise<void>;
  act(() => { previous = result.current.refresh(); });
  await act(async () => { await result.current.refresh(); });
  await act(async () => {
    reject(new Error("Old read failed"));
    await previous;
  });
  expect(result.current.settings).toEqual(saved);
  expect(result.current.error).toBeNull();
});

test("a current read failure keeps saved values and a later refresh recovers", async () => {
  const saved = { asr_model: "saved" } as Settings;
  let fail = false;
  mockTauri({
    get_settings: () => {
      if (fail) throw new Error("Settings unavailable");
      return saved;
    },
    readiness: () => ({ armed: true }),
    model_statuses: () => [],
  });
  const { result } = renderHook(useAppSettings);
  await waitFor(() => expect(result.current.settings).toEqual(saved));
  fail = true;
  await act(async () => { await result.current.refresh(); });
  expect(result.current.settings).toEqual(saved);
  expect(result.current.error).toBe("Settings unavailable.");
  fail = false;
  await act(async () => { await result.current.refresh(); });
  expect(result.current.settings).toEqual(saved);
  expect(result.current.error).toBeNull();
});
