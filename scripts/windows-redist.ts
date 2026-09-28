import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

export function windowsRedistRoots(redist: string | undefined, architecture: string): string[] {
  if (!redist) return [];
  const arch = architecture === "arm64" ? "arm64" : "x64";
  const root = join(redist, arch);
  if (!existsSync(root)) return [];
  return readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && /^Microsoft\.VC\d+\.(CRT|OpenMP)$/.test(entry.name))
    .map(entry => join(root, entry.name));
}
