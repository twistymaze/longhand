// Writes the full licence list for each shipped platform (KTD4):
// licences/generated/macos.txt and licences/generated/windows.txt.
//
// It merges three sources:
//   - cargo-about, run once for each target (about.toml), for the Rust crates;
//   - licences/generated/js-packages.json, which `bun run build` writes, for the
//     interface packages;
//   - licences/native/manifest.json, for what no tool can read.
//
//   bun scripts/licences/generate.ts [--js-packages <file>]

import { spawnSync } from "child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "fs";
import { homedir } from "os";
import { dirname, join, resolve } from "path";
import {
  checkOnnxRuntimeVersion,
  copyrightFromFiles,
  renderList,
  selectNative,
  splitCopyright,
  type LicensedComponent,
  type NativeEntry,
} from "./group";

const ROOT = resolve(import.meta.dir, "../..");
const GENERATED = join(ROOT, "licences/generated");

const PLATFORMS = [
  { platform: "macos", target: "aarch64-apple-darwin", product: "Longhand for macOS" },
  { platform: "windows", target: "x86_64-pc-windows-msvc", product: "Longhand for Windows" },
];

interface Manifest {
  onnxRuntimeVersion: string;
  entries: NativeEntry[];
  crateNotes: Record<string, string>;
}

interface JsPackage {
  name: string;
  version: string;
  license: string | null;
  licenseText: string | null;
  noticeText: string | null;
}

interface CargoAboutCrate {
  name: string;
  version: string;
  manifest_path: string;
}

interface CargoAboutOutput {
  licenses: { text: string; used_by: { crate: CargoAboutCrate }[] }[];
  crates: { package: CargoAboutCrate }[];
}

function fail(message: string): never {
  console.error(`licences: ${message}`);
  process.exit(1);
}

function run(command: string, args: string[]): { stdout: string; stderr: string } {
  const result = spawnSync(command, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.error) fail(`could not start ${command}: ${result.error.message}`);
  if (result.status !== 0) fail(`${command} ${args.join(" ")} failed:\n${result.stderr}`);
  return { stdout: result.stdout, stderr: result.stderr };
}

// cargo-about reports a clarify entry whose checksum does not match only as a
// warning, and then uses the files it finds itself. Any warning fails here, so
// a changed licence file cannot pass unreviewed.
function cargoAbout(target: string): CargoAboutOutput {
  const { stdout, stderr } = run("cargo", [
    "about",
    "generate",
    "--format",
    "json",
    "--locked",
    "--offline",
    "--fail",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--target",
    target,
  ]);
  // The log lines carry colour codes even when stderr is not a terminal.
  const warnings = stderr
    .split("\n")
    .map((line) => line.replace(/\x1b\[[0-9;]*m/g, ""))
    .filter((line) => /\[(WARN|ERROR)\]/.test(line));
  if (warnings.length > 0) fail(`cargo about reported problems for ${target}:\n${warnings.join("\n")}`);
  return JSON.parse(stdout) as CargoAboutOutput;
}

// The licence files at the top of a crate's source folder.
function crateLicenceFiles(crate: CargoAboutCrate): string[] {
  const dir = dirname(crate.manifest_path);
  return readdirSync(dir)
    .filter((name) => /licen[cs]e|copying/i.test(name))
    .map((name) => join(dir, name))
    .filter((path) => statSync(path).isFile())
    .map((path) => readFileSync(path, "utf8"));
}

function rustComponents(about: CargoAboutOutput, notes: Record<string, string>): LicensedComponent[] {
  // Only the name, the version and the text are kept: cargo-about's paths are
  // absolute and differ on each host.
  return about.licenses.flatMap((licence) =>
    licence.used_by.map(({ crate }) => {
      const own = splitCopyright(licence.text).copyright;
      const found = own.length > 0 ? [] : copyrightFromFiles(licence.text, crateLicenceFiles(crate));
      return {
        part: "rust" as const,
        name: crate.name,
        version: crate.version,
        note: notes[crate.name],
        text: found.length > 0 ? [...found, "", licence.text].join("\n") : licence.text,
      };
    }),
  );
}

function jsComponents(packages: JsPackage[]): LicensedComponent[] {
  return packages.flatMap((pkg) => {
    const base = { part: "interface" as const, name: pkg.name, version: pkg.version };
    const licence = pkg.licenseText ?? `${pkg.license} licence. The package includes no licence file.`;
    const out: LicensedComponent[] = [{ ...base, text: licence }];
    if (pkg.noticeText) out.push({ ...base, note: "notice", text: pkg.noticeText });
    return out;
  });
}

function checkOrtVersion(about: CargoAboutOutput, manifest: Manifest): void {
  const ortSys = about.crates.find((c) => c.package.name === "ort-sys");
  if (!ortSys) fail("ort-sys is not in the cargo-about output; review the ONNX Runtime entries in licences/native/manifest.json");
  const distFile = join(dirname(ortSys.package.manifest_path), "dist.txt");
  if (!existsSync(distFile)) fail(`ort-sys ${ortSys.package.version} has no dist.txt`);
  try {
    checkOnnxRuntimeVersion(readFileSync(distFile, "utf8"), manifest.onnxRuntimeVersion);
  } catch (error) {
    fail((error as Error).message);
  }
}

// The stored text must not show where the tools ran.
function checkNoHostPaths(platform: string, text: string): void {
  for (const path of [ROOT, homedir(), "/.cargo/registry/"]) {
    if (text.includes(path)) fail(`${platform}.txt contains a host path (${path})`);
  }
}

function main(): void {
  const args = process.argv.slice(2);
  const jsIndex = args.indexOf("--js-packages");
  const jsFile = jsIndex >= 0 ? resolve(args[jsIndex + 1]) : join(GENERATED, "js-packages.json");
  if (!existsSync(jsFile)) {
    fail(`${jsFile} does not exist. Run \`bun run build\` first: the Vite build writes it.`);
  }
  const packages = JSON.parse(readFileSync(jsFile, "utf8")) as JsPackage[];
  if (packages.length === 0) fail(`${jsFile} lists no packages; check rollup-plugin-license in vite.config.ts`);

  const manifest = JSON.parse(readFileSync(join(ROOT, "licences/native/manifest.json"), "utf8")) as Manifest;
  const read = (path: string) => readFileSync(join(ROOT, path), "utf8");

  for (const { platform, target, product } of PLATFORMS) {
    const about = cargoAbout(target);
    checkOrtVersion(about, manifest);
    const components = [
      ...rustComponents(about, manifest.crateNotes),
      ...jsComponents(packages),
      ...selectNative(manifest.entries, platform, read),
    ];
    const text = renderList(product, components);
    checkNoHostPaths(platform, text);
    writeFileSync(join(GENERATED, `${platform}.txt`), text);
    console.log(`licences: wrote licences/generated/${platform}.txt (${components.length} licence texts)`);
  }
}

if (import.meta.main) main();
