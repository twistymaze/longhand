import { expect, test } from "bun:test";
import { patchNemoSession } from "./patch-nemo";

const pinned = [
  "#include <algorithm>",
  "#include <mutex>",
  "",
  "        if (!ggml_graph_compute_helper_async(sched.get(), cr.gf, 4)) {",
  "",
].join("\n");

test("CPU graph threads are capped at the hardware thread count", () => {
  const patched = patchNemoSession(pinned);
  expect(patched).not.toContain("cr.gf, 4)");
  expect(patched).toContain("#include <mutex>\n#include <thread>\n");
  expect(patched).toContain("std::min(4u, hw)");
});

test("a patched source stays the same", () => {
  const patched = patchNemoSession(pinned);
  expect(patchNemoSession(patched)).toBe(patched);
});

test("Windows line endings stay Windows line endings", () => {
  const patched = patchNemoSession(pinned.replaceAll("\n", "\r\n"));
  expect(patched.replaceAll("\r\n", "")).not.toContain("\n");
  expect(patched).toContain("#include <thread>\r\n");
});

test("a source that differs from the pinned revision is refused", () => {
  expect(() => patchNemoSession(pinned.replace("cr.gf, 4)", "cr.gf, n_threads)"))).toThrow("pinned revision");
});
