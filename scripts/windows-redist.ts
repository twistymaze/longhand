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

export function findWindowsDependency(name: string, roots: string[], vulkanRuntime?: string): string | undefined {
  // An explicit loader source must not fall back to an unverified host copy.
  if (name.toLowerCase() === "vulkan-1.dll" && vulkanRuntime) roots = [vulkanRuntime];
  for (const root of roots) {
    if (!existsSync(root)) continue;
    const found = readdirSync(root, { withFileTypes: true })
      .find(item => item.isFile() && item.name.toLowerCase() === name.toLowerCase());
    if (found) return join(root, found.name);
  }
  return undefined;
}
