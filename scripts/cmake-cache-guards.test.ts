import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

// Two scripts configure a cached CMake build folder again when it names a
// build tool or input that is not there: build-nemo.sh for the NeMo cache, and
// the sccache setup action for the CMake crates in target/. A runner image
// update can remove the linker and keep the compiler, so both must check the
// same keys. The NeMo copy once checked only the compiler, and a cache from an
// older image failed the Windows job with a missing link.exe.

const ROOT = join(import.meta.dirname, "..");
const GUARDS = ["scripts/build-nemo.sh", ".github/actions/sccache-setup/action.yml"];
const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

// The missing_input() function of a file, as shell text.
function missingInput(file: string): string {
  const text = readFileSync(join(ROOT, file), "utf8");
  const start = text.indexOf("missing_input() {");
  assert.ok(start >= 0, `${file} has no missing_input() function`);
  // The function ends at the first line that holds only "}".
  const end = /\n *\}\n/.exec(text.slice(start));
  assert.ok(end, `${file}: the end of missing_input() is not found`);
  return text.slice(start, start + end.index + end[0].length);
}

// Runs missing_input on a CMakeCache.txt with the given lines. True when the
// function says the build folder must be configured again.
function stale(file: string, lines: string[]): boolean {
  const dir = mkdtempSync(join(tmpdir(), "cmake-guard-"));
  dirs.push(dir);
  const cache = join(dir, "CMakeCache.txt");
  writeFileSync(cache, `${lines.join("\n")}\n`);
  const r = spawnSync("bash", ["-c", `${missingInput(file)}\nmissing_input "$1"`, "guard", cache], { encoding: "utf8" });
  assert.ok(r.status === 0 || r.status === 1, `${file}: ${r.stderr}`);
  return r.status === 0;
}

const here = process.execPath.replace(/\\/g, "/");
const gone = "/no/such/tool/link.exe";

for (const file of GUARDS) {
  test(`${file}: a cached folder whose tools are all present is kept`, () => {
    assert.equal(stale(file, [`CMAKE_CXX_COMPILER:FILEPATH=${here}`, `CMAKE_LINKER:FILEPATH=${here}`]), false);
  });

  for (const key of ["CMAKE_LINKER", "CMAKE_AR", "CMAKE_MT", "CMAKE_C_COMPILER", "CMAKE_CXX_COMPILER", "CMAKE_TOOLCHAIN_FILE", "Vulkan_LIBRARY"]) {
    test(`${file}: a cached folder that names a missing ${key} is configured again`, () => {
      assert.equal(stale(file, [`CMAKE_CXX_COMPILER:FILEPATH=${here}`, `${key}:FILEPATH=${gone}`]), true);
    });
  }
}
