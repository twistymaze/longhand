// Apply Longhand's change to the pinned NeMo SDK source before it is built.
// The pinned SDK computes every CPU graph with 4 threads. ggml's CPU workers
// busy-wait for each other, so on a machine with fewer hardware threads each
// operation waits for the scheduler: a 3-vCPU runner took 256 s, not 7 s, for
// 34 s of audio. The change keeps 4 threads and caps them at the hardware
// thread count. Upstream's recognizer applies the same cap; its diarizer does not.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const include = "#include <mutex>\n";
const call = "ggml_graph_compute_helper_async(sched.get(), cr.gf, 4)";
const marker = "Modified by Longhand";

export function patchNemoSession(source: string): string {
  if (source.includes(marker)) return source;
  const newline = source.includes("\r\n") ? "\r\n" : "\n";
  const text = source.replaceAll("\r\n", "\n");
  if (text.split(include).length !== 2 || text.split(call).length !== 2) {
    throw new Error("NeMo session.cpp does not match the pinned revision");
  }
  const threads = `[] { /* ${marker}: no more CPU workers than hardware threads. */ const unsigned hw = std::thread::hardware_concurrency(); return hw == 0 ? 4 : static_cast<int>(std::min(4u, hw)); }()`;
  return text
    .replace(include, `${include}#include <thread>\n`)
    .replace(call, call.replace("cr.gf, 4)", `cr.gf, ${threads})`))
    .replaceAll("\n", newline);
}

if (import.meta.main) {
  const source = process.argv[2];
  if (!source) throw new Error("Use patch-nemo.ts NEMO-SOURCE-DIRECTORY");
  const path = join(source, "src/runtime/ggml/session.cpp");
  const before = readFileSync(path, "utf8");
  const after = patchNemoSession(before);
  // An unchanged file keeps its timestamp, so a warm build does not recompile it.
  if (after !== before) writeFileSync(path, after);
}
