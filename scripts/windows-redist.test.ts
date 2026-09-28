import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { windowsRedistRoots } from "./windows-redist";

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
