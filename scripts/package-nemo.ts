// Stage the helper and its private native dependency closure.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, renameSync, rmSync, chmodSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve } from "node:path";
import { createHash } from "node:crypto";
import { MAX_OUTPUT_BYTES } from "./lib/spawn.ts";
import { findWindowsDependency, windowsRedistRoots } from "./windows-redist.ts";

const manifestPath = process.argv[2];
if (!manifestPath) throw new Error("Use node scripts/package-nemo.ts BUILD-OUTPUT.json [OUTPUT-DIR]");
const input = JSON.parse(readFileSync(manifestPath, "utf8")) as { helper: string; sdkBin: string };
const helper = realpathSync(input.helper);
const sdkBin = realpathSync(input.sdkBin);
const output = resolve(process.argv[3] ?? join(import.meta.dirname, "../src-tauri/nemo-runtime"));
const staging = `${output}.staging-${process.pid}`;
if (existsSync(staging)) throw new Error(`Staging directory already exists: ${staging}`);
if (existsSync(output) && !existsSync(join(output, "longhand-nemo-package.json"))) throw new Error("Refusing to replace an output directory without a generated NeMo package manifest");
mkdirSync(staging, { recursive: true });
const binaries = join(staging, "bin");
mkdirSync(binaries);
function run(command: string[]) {
  const [program, ...args] = command;
  const result = spawnSync(program, args, { encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES });
  if (result.status !== 0) throw new Error(`${program} failed: ${result.error?.message ?? result.stderr}`);
  return result.stdout;
}
function inside(path: string, root: string) { const rel = relative(root, path); return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel)); }
const copied = new Map<string, string>();
const queue: string[] = [];
function copy(source: string, name = basename(source)) {
  source = realpathSync(source);
  if (copied.has(name)) { if (copied.get(name) !== source) throw new Error(`Native dependency name collision: ${name}`); return; }
  copyFileSync(source, join(binaries, name)); chmodSync(join(binaries, name), 0o755);
  copied.set(name, source); queue.push(name);
}
const helperName = process.platform === "win32" ? "longhand-nemo-helper.exe" : "longhand-nemo-helper";
copy(helper, helperName);
function macDependencies(path: string) { return run(["otool", "-L", path]).split("\n").slice(1).map(line => line.trim().split(" (compatibility version")[0]!).filter(Boolean); }
function macId(path: string) { return run(["otool", "-D", path]).split("\n").slice(1).map(line => line.trim()).filter(Boolean)[0]; }
function systemMac(path: string) { return path.startsWith("/System/Library/") || path.startsWith("/usr/lib/"); }
function macRpaths(path: string) {
  return [...run(["otool", "-l", path]).matchAll(/cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset \d+\)/g)].map(match => match[1]!);
}
function windowsDependencies(path: string) { return run(["dumpbin", "/DEPENDENTS", path]).split("\n").map(line => line.trim()).filter(line => /^[A-Za-z0-9_.-]+\.dll$/i.test(line)); }
const windowsRoots = [sdkBin];
windowsRoots.push(...windowsRedistRoots(process.env.VCToolsRedistDir, process.arch));
if (process.env.VULKAN_SDK) windowsRoots.push(join(process.env.VULKAN_SDK, "Bin"));
function windowsSystem(name: string) {
  if (/^(api-ms-win-|ext-ms-win-)/i.test(name)) return true;
  // Runtime and Vulkan DLLs must be app-local, even if this host has a copy.
  if (/^(vcruntime|msvcp|concrt|vcomp|vulkan-)/i.test(name)) return false;
  const system = process.env.SystemRoot;
  return !!system && existsSync(join(system, "System32", name));
}
try {
  if (process.platform === "darwin") {
    for (let i = 0; i < queue.length; i++) {
      const name = queue[i]!;
      const source = copied.get(name)!;
      const target = join(binaries, name);
      const id = name.endsWith(".dylib") ? macId(source) : undefined;
      for (const dependency of macDependencies(source)) {
        if (dependency === id || systemMac(dependency)) continue;
        const dependencyName = basename(dependency);
        let candidate: string;
        if (dependency.startsWith("@loader_path/")) candidate = resolve(dirname(source), dependency.slice("@loader_path/".length));
        else if (dependency.startsWith("@rpath/")) candidate = join(sdkBin, dependencyName);
        else if (isAbsolute(dependency)) candidate = dependency;
        else throw new Error(`Unsupported native dependency: ${dependency}`);
        if (!existsSync(candidate) || !inside(realpathSync(candidate), sdkBin)) throw new Error(`Dependency is absent from the pinned SDK bin directory: ${dependency}`);
        copy(candidate, dependencyName);
        run(["install_name_tool", "-change", dependency, `@loader_path/${dependencyName}`, target]);
      }
      if (id) run(["install_name_tool", "-id", `@loader_path/${name}`, target]);
      for (const path of macRpaths(target)) run(["install_name_tool", "-delete_rpath", path, target]);
    }
    // Some SDK configurations use a separate Metal resource instead of embedded code.
    for (const name of readdirSync(sdkBin)) if (name.endsWith(".metallib")) copyFileSync(join(sdkBin, name), join(binaries, name));
    for (const name of queue) {
      for (const dependency of macDependencies(join(binaries, name))) {
        if (systemMac(dependency)) continue;
        if (!dependency.startsWith("@loader_path/") || !existsSync(join(binaries, dependency.slice("@loader_path/".length)))) throw new Error(`Nonrelocatable staged dependency: ${dependency}`);
      }
      if (macRpaths(join(binaries, name)).length) throw new Error(`Staged file retains an RPATH: ${name}`);
    }
  } else if (process.platform === "win32") {
    for (let i = 0; i < queue.length; i++) {
      for (const dependency of windowsDependencies(join(binaries, queue[i]!))) {
        if (windowsSystem(dependency)) continue;
        const candidate = findWindowsDependency(dependency, windowsRoots, process.env.LONGHAND_VULKAN_RUNTIME_DIR);
        if (!candidate) throw new Error(`Unresolved Windows dependency: ${dependency}`);
        copy(candidate, dependency);
      }
    }
    for (const name of queue) for (const dependency of windowsDependencies(join(binaries, name))) {
      if (!windowsSystem(dependency) && !readdirSync(binaries).some(item => item.toLowerCase() === dependency.toLowerCase())) throw new Error(`Missing staged DLL: ${dependency}`);
    }
  } else throw new Error("NeMo package staging supports macOS and Windows");
  if (process.platform === "darwin") {
    const identity = process.env.LONGHAND_NEMO_SIGNING_IDENTITY ?? process.env.APPLE_SIGNING_IDENTITY ?? "-";
    const options = identity === "-" ? [] : ["--options", "runtime", "--timestamp"];
    for (const name of [...queue.filter(name => name !== helperName), helperName]) {
      const path = join(binaries, name);
      run(["codesign", "--force", "--sign", identity, ...options, path]);
      run(["codesign", "--verify", "--strict", path]);
    }
  }
  const files = [];
  for (const name of readdirSync(binaries).sort()) {
    const bytes = readFileSync(join(binaries, name));
    files.push({ name: `bin/${name}`, bytes: bytes.byteLength, sha256: createHash("sha256").update(bytes).digest("hex") });
  }
  writeFileSync(join(staging, "longhand-nemo-package.json"), JSON.stringify({ format: 1, protocolVersion: 1, runtimeRevision: "97a15afa5caa9bce5baaa86c1184103877af4101", platform: process.platform, arch: process.arch, helper: `bin/${helperName}`, files }, null, 2) + "\n");
  // This folder contains generated build output only. The marker was checked above.
  if (existsSync(output)) rmSync(output, { recursive: true });
  renameSync(staging, output);
  console.log(JSON.stringify({ output, helper: join(output, "bin", helperName), files: files.length }));
} catch (error) {
  rmSync(staging, { recursive: true, force: true });
  throw error;
}
