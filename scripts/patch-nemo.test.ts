import assert from "node:assert/strict";
import { test } from "node:test";
import { patchNemoSession } from "./patch-nemo.ts";

const pinned = [
  "#include <algorithm>",
  "#include <mutex>",
  "",
  "        if (!ggml_graph_compute_helper_async(sched.get(), cr.gf, 4)) {",
  "",
].join("\n");

test("CPU graph threads are capped at the hardware thread count", () => {
  const patched = patchNemoSession(pinned);
  assert.ok(!patched.includes("cr.gf, 4)"));
  assert.ok(patched.includes("#include <mutex>\n#include <thread>\n"));
  assert.ok(patched.includes("std::min(4u, hw)"));
});

test("a patched source stays the same", () => {
  const patched = patchNemoSession(pinned);
  assert.equal(patchNemoSession(patched), patched);
});

test("Windows line endings stay Windows line endings", () => {
  const patched = patchNemoSession(pinned.replaceAll("\n", "\r\n"));
  assert.ok(!patched.replaceAll("\r\n", "").includes("\n"));
  assert.ok(patched.includes("#include <thread>\r\n"));
});

test("a source that differs from the pinned revision is refused", () => {
  assert.throws(() => patchNemoSession(pinned.replace("cr.gf, 4)", "cr.gf, n_threads)")), /pinned revision/);
});
