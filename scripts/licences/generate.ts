// Writes the full licence list for each shipped platform (KTD4):
// licences/generated/macos.txt and licences/generated/windows.txt.
//
// It merges three sources:
//   - cargo-about, run once for each target (about.toml), for the Rust crates;
//   - licences/generated/js-packages.json, which `npm run build` writes, for the
//     interface packages;
//   - licences/manifest.json, for what no tool can read.
//
//   node scripts/licences/generate.ts [--js-packages <file>]

import { spawnSync } from "child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "fs";
import { homedir } from "os";
import { dirname, join, resolve } from "path";
import { MAX_OUTPUT_BYTES } from "../lib/spawn.ts";
import {
  cargoAboutProblems,
  checkNpmVersion,
  checkOnnxRuntimeVersion,
  copyrightFromFiles,
  packageLicenceTexts,
  renderList,
  selectNative,
  splitCopyright,
  type JsPackage,
  type LicensedComponent,
  type NativeEntry,
  type PackageLicence,
} from "./group.ts";

const ROOT = resolve(import.meta.dirname, "../..");
const GENERATED = join(ROOT, "licences/generated");

const PLATFORMS = [
  { platform: "macos", target: "aarch64-apple-darwin", product: "Longhand for macOS" },
  { platform: "windows", target: "x86_64-pc-windows-msvc", product: "Longhand for Windows" },
];

interface Manifest {
  onnxRuntimeVersion: string;
  entries: NativeEntry[];
  packageLicences: Record<string, PackageLicence>;
  crateNotes: Record<string, string>;
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
  const result = spawnSync(command, args, { cwd: ROOT, encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES });
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
  const warnings = cargoAboutProblems(stderr);
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

function jsComponents(
  packages: JsPackage[],
  replacements: Record<string, PackageLicence>,
  read: (path: string) => string,
): LicensedComponent[] {
  for (const name of Object.keys(replacements)) {
    if (!packages.some((pkg) => pkg.name === name)) {
      fail(`the build does not include ${name}; remove its packageLicences entry from licences/manifest.json`);
    }
  }
  return packages.flatMap((pkg) => {
    const base = { part: "interface" as const, name: pkg.name, version: pkg.version };
    let texts: string[];
    try {
      texts = packageLicenceTexts(pkg, replacements, read);
    } catch (error) {
      fail((error as Error).message);
    }
    const out: LicensedComponent[] = texts.map((text) => ({ ...base, text }));
    if (pkg.noticeText) out.push({ ...base, note: "notice", text: pkg.noticeText });
    return out;
  });
}

// A native entry with an npm package must name the installed version.
function checkNpmVersions(manifest: Manifest): void {
  for (const entry of manifest.entries) {
    if (!entry.npmPackage) continue;
    const file = join(ROOT, "node_modules", entry.npmPackage, "package.json");
    if (!existsSync(file)) fail(`${entry.npmPackage} is not in node_modules; run \`npm ci\``);
    const { version } = JSON.parse(readFileSync(file, "utf8")) as { version: string };
    try {
      checkNpmVersion(entry, version);
    } catch (error) {
      fail((error as Error).message);
    }
  }
}

function checkOrtVersion(about: CargoAboutOutput, manifest: Manifest): void {
  const ortSys = about.crates.find((c) => c.package.name === "ort-sys");
  if (!ortSys) fail("ort-sys is not in the cargo-about output; review the ONNX Runtime entries in licences/manifest.json");
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
    fail(`${jsFile} does not exist. Run \`npm run build\` first: the Vite build writes it.`);
  }
  const packages = JSON.parse(readFileSync(jsFile, "utf8")) as JsPackage[];
  if (packages.length === 0) fail(`${jsFile} lists no packages; check rollup-plugin-license in vite.config.ts`);

  const manifest = JSON.parse(readFileSync(join(ROOT, "licences/manifest.json"), "utf8")) as Manifest;
  const read = (path: string) => readFileSync(join(ROOT, path), "utf8");
  checkNpmVersions(manifest);
  const js = jsComponents(packages, manifest.packageLicences ?? {}, read);

  for (const { platform, target, product } of PLATFORMS) {
    const about = cargoAbout(target);
    checkOrtVersion(about, manifest);
    const components = [
      ...rustComponents(about, manifest.crateNotes),
      ...js,
      ...selectNative(manifest.entries, platform, read),
    ];
    const text = renderList(product, components);
    checkNoHostPaths(platform, text);
    writeFileSync(join(GENERATED, `${platform}.txt`), text);
    console.log(`licences: wrote licences/generated/${platform}.txt (${components.length} licence texts)`);
  }
}

if (import.meta.main) main();
