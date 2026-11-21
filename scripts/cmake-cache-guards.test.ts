import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
// function says the build folder must be configured again. vcTools is the
// VCToolsVersion value of the MSVC environment, or undefined for none.
function stale(file: string, lines: string[], vcTools?: string): boolean {
  const dir = mkdtempSync(join(tmpdir(), "cmake-guard-"));
  dirs.push(dir);
  const cache = join(dir, "CMakeCache.txt");
  writeFileSync(cache, `${lines.join("\n")}\n`);
  const env = { ...process.env };
  delete env.VCToolsVersion;
  if (vcTools !== undefined) env.VCToolsVersion = vcTools;
  const r = spawnSync("bash", ["-c", `${missingInput(file)}\nmissing_input "$1"`, "guard", cache], { encoding: "utf8", env });
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

// A runner image can have two MSVC toolsets. A cached folder from the older
// one names tools that are still there, and the older linker cannot always
// link the objects of the newer compiler. So, in an MSVC environment, a
// folder whose tools are under another toolset version is configured again.
const TOOL_KEYS = ["CMAKE_C_COMPILER", "CMAKE_CXX_COMPILER", "CMAKE_LINKER", "CMAKE_AR", "CMAKE_MT"];

// An existing file under <tmp>/VS/VC/Tools/MSVC/<version>/bin/Hostx64/x64/.
function msvcTool(version: string): string {
  const dir = mkdtempSync(join(tmpdir(), "msvc-"));
  dirs.push(dir);
  const bin = join(dir, "VS", "VC", "Tools", "MSVC", version, "bin", "Hostx64", "x64");
  mkdirSync(bin, { recursive: true });
  const path = join(bin, "tool.exe");
  writeFileSync(path, "");
  return path.replace(/\\/g, "/");
}

for (const file of GUARDS) {
  for (const key of TOOL_KEYS) {
    test(`${file}: in MSVC 14.52.1, a cached ${key} from MSVC 14.51.1 that still exists is configured again`, () => {
      assert.equal(stale(file, [`${key}:FILEPATH=${msvcTool("14.51.1")}`], "14.52.1"), true);
    });
  }

  test(`${file}: in MSVC 14.52.1, a cached folder whose tools are all from 14.52.1 is kept`, () => {
    const lines = TOOL_KEYS.map((key) => `${key}:FILEPATH=${msvcTool("14.52.1")}`);
    assert.equal(stale(file, lines, "14.52.1"), false);
  });

  test(`${file}: with no MSVC environment, a cached tool from another toolset that exists is kept`, () => {
    assert.equal(stale(file, [`CMAKE_LINKER:FILEPATH=${msvcTool("14.51.1")}`]), false);
  });

  test(`${file}: the toolset folder is found in a Windows path with backslashes`, () => {
    // The path has no "/", so the missing-path check does not apply, and only
    // the toolset check can decide.
    const windows = "C:\\VS\\VC\\Tools\\MSVC\\14.51.1\\bin\\Hostx64\\x64\\link.exe";
    assert.equal(stale(file, [`CMAKE_LINKER:FILEPATH=${windows}`], "14.52.1"), true);
    assert.equal(stale(file, [`CMAKE_LINKER:FILEPATH=${windows}`], "14.51.1"), false);
  });

  test(`${file}: the toolset folder is found in any letter case`, () => {
    // macOS volumes ignore letter case, so the changed path still exists.
    const path = msvcTool("14.51.1").replace("/VC/Tools/MSVC/", "/vc/tools/Msvc/");
    assert.equal(stale(file, [`CMAKE_LINKER:FILEPATH=${path}`], "14.52.1"), true);
  });
}
