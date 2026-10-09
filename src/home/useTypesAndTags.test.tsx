import { describe, expect, test } from "bun:test";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { NoteTypeList } from "../bindings";
import { mockTauri } from "../test/tauri";
import { useTypesAndTags } from "./useTypesAndTags";

const typesNamed = (...names: string[]): NoteTypeList => ({
  types: names.map((name) => ({ name, valid: true, shared: true, notes: 0, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" })),
  default_type: null,
});
const settle = () => act(() => new Promise((r) => setTimeout(r, 20)));

describe("useTypesAndTags", () => {
  test("a read that ends after a newer read does not replace the newer list", async () => {
    const reads: ((l: NoteTypeList) => void)[] = [];
    const tauri = mockTauri({
      note_types: () => new Promise<NoteTypeList>((r) => reads.push(r)),
      list_tags: () => [],
    });
    const { result } = renderHook(() => useTypesAndTags(true));
    await waitFor(() => expect(reads).toHaveLength(1));
    await act(() => tauri.emit("settings-changed", null));
    await waitFor(() => expect(reads).toHaveLength(2));
    await act(async () => reads[1](typesNamed("New")));
    await waitFor(() => expect(result.current.types?.types.map((t) => t.name)).toEqual(["New"]));
    await act(async () => reads[0](typesNamed("Old")));
    await settle();
    expect(result.current.types?.types.map((t) => t.name)).toEqual(["New"]);
  });

  test("a read that started before setTypes does not replace the list that setTypes gave", async () => {
    const reads: ((l: NoteTypeList) => void)[] = [];
    mockTauri({
      note_types: () => new Promise<NoteTypeList>((r) => reads.push(r)),
      list_tags: () => [],
    });
    const { result } = renderHook(() => useTypesAndTags(true));
    await waitFor(() => expect(reads).toHaveLength(1));
    act(() => result.current.setTypes(typesNamed("Made")));
    await act(async () => reads[0](typesNamed("Old")));
    await settle();
    expect(result.current.types?.types.map((t) => t.name)).toEqual(["Made"]);
  });
});
