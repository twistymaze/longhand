import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findWindowsDependency, windowsRedistRoots } from "./windows-redist";

const fixtures: string[] = [];
afterEach(() => { for (const root of fixtures.splice(0)) rmSync(root, { recursive: true }); });
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "longhand-redist-"));
  fixtures.push(root);
  for (const arch of ["x64", "arm64"]) {
    for (const name of ["Microsoft.VC143.CRT", "Microsoft.VC143.OpenMP", "Microsoft.VC143.DebugCRT", "Microsoft.VC143.MFC"]) {
      mkdirSync(join(root, arch, name), { recursive: true });
    }
    writeFileSync(join(root, arch, "Microsoft.VC143.OpenMP", "vcomp140.dll"), "fixture");
    writeFileSync(join(root, arch, "Microsoft.VC999.CRT"), "not a directory");
  }
  return root;
}
test("includes the OpenMP redistributable that provides VCOMP140 alongside the CRT", () => {
  const root = fixture();
  expect(windowsRedistRoots(root, "x64").sort()).toEqual([
    join(root, "x64", "Microsoft.VC143.CRT"),
    join(root, "x64", "Microsoft.VC143.OpenMP"),
  ]);
});
test("uses only the requested runtime architecture and excludes debug or unrelated folders", () => {
  const root = fixture();
  expect(windowsRedistRoots(root, "arm64").sort()).toEqual([
    join(root, "arm64", "Microsoft.VC143.CRT"),
    join(root, "arm64", "Microsoft.VC143.OpenMP"),
  ]);
});
test("an absent redistributable directory leaves dependency validation to fail closed", () => {
  expect(windowsRedistRoots(undefined, "x64")).toEqual([]);
  expect(windowsRedistRoots(join(fixture(), "absent"), "x64")).toEqual([]);
});

test("resolves the private SDK, CRT, OpenMP and explicit Vulkan loader dependency chain", () => {
  const root = fixture();
  const sdk = join(root, "sdk");
  const vulkan = join(root, "vulkan", "x64");
  mkdirSync(sdk); mkdirSync(vulkan, { recursive: true });
  writeFileSync(join(sdk, "ggml-vulkan.dll"), "fixture");
  writeFileSync(join(root, "x64", "Microsoft.VC143.CRT", "msvcp140.dll"), "fixture");
  writeFileSync(join(vulkan, "vulkan-1.dll"), "fixture");
  const roots = [sdk, ...windowsRedistRoots(root, "x64")];
  const dependencies: Record<string, string[]> = {
    "ggml-vulkan.dll": ["MSVCP140.dll", "VCOMP140.DLL", "vulkan-1.dll"],
    "MSVCP140.dll": [], "VCOMP140.DLL": [], "vulkan-1.dll": [],
  };
  const queue = ["ggml-vulkan.dll"];
  for (let i = 0; i < queue.length; i++) {
    const name = queue[i]!;
    expect(findWindowsDependency(name, roots, vulkan)).toBeDefined();
    queue.push(...dependencies[name]!);
  }
  expect(findWindowsDependency("VULKAN-1.DLL", roots, vulkan)).toBe(join(vulkan, "vulkan-1.dll"));
});
test("the explicit Vulkan directory cannot supply unrelated runtime or system DLLs", () => {
  const root = fixture();
  writeFileSync(join(root, "msvcp140.dll"), "fixture");
  writeFileSync(join(root, "kernel32.dll"), "fixture");
  expect(findWindowsDependency("msvcp140.dll", [], root)).toBeUndefined();
  expect(findWindowsDependency("kernel32.dll", [], root)).toBeUndefined();
  expect(findWindowsDependency("vulkan-1.dll", [], root)).toBeUndefined();
});

test("a missing explicit loader fails closed even when an SDK loader exists", () => {
  const root = fixture();
  writeFileSync(join(root, "vulkan-1.dll"), "unselected SDK copy");
  expect(findWindowsDependency("vulkan-1.dll", [root], join(root, "absent"))).toBeUndefined();
});
