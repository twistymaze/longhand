// Signs Windows files with Azure Artifact Signing and checks the signatures
// of an install folder before a release attaches the installer.
//
//   node scripts/windows-signing.ts sign <file>
//   node scripts/windows-signing.ts verify <folder> <installer>
//
// `sign` reads LONGHAND_SIGNTOOL, LONGHAND_SIGNING_DLIB and
// LONGHAND_SIGNING_METADATA. `verify` reads WINDOWS_PUBLISHER, the common name
// of the Artifact Signing certificate subject.
import { execFileSync, spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { basename, join } from "node:path";

const TIMESTAMP_SERVER = "http://timestamp.acs.microsoft.com";

// Files from other vendors that the installer contains, with the publisher
// that signs them. A listed file that WINDOWS_PUBLISHER signs also passes.
const VENDOR_FILES: { name: RegExp; publisher: string }[] = [
  { name: /^msvcp140.*\.dll$/i, publisher: "Microsoft Corporation" },
  { name: /^vcruntime140.*\.dll$/i, publisher: "Microsoft Corporation" },
  { name: /^vcomp140.*\.dll$/i, publisher: "Microsoft Corporation" },
  // The Vulkan loader from the LunarG runtime archive (prepare-vulkan-runtime.ps1).
  { name: /^vulkan-1\.dll$/i, publisher: "LunarG, Inc." },
];

export type Signature = { status: string; subject: string; timestamped: boolean };

export function signCommand(file: string, env: Record<string, string | undefined>): { program: string; args: string[] } {
  const value = (name: string) => {
    const found = env[name];
    if (!found) throw new Error(`${name} is not set`);
    return found;
  };
  const program = value("LONGHAND_SIGNTOOL");
  const dlib = value("LONGHAND_SIGNING_DLIB");
  const metadata = value("LONGHAND_SIGNING_METADATA");
  return {
    program,
    args: [
      "sign", "/v",
      "/fd", "SHA256",
      "/tr", TIMESTAMP_SERVER, "/td", "SHA256",
      "/dlib", dlib,
      "/dmdf", metadata,
      file,
    ],
  };
}

function binaries(folder: string): string[] {
  return readdirSync(folder, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile() && /\.(exe|dll)$/i.test(entry.name))
    .map(entry => join(entry.parentPath, entry.name));
}

export function verifyTargets(folder: string, installer: string): string[] {
  const files = binaries(folder);
  // An empty folder means that the install did not occur, not that all files pass.
  if (files.length === 0) throw new Error(`${folder} has no .exe or .dll files`);
  return [...files, installer];
}

// Reads the CN value of an X.500 subject, for example
// `CN="LunarG, Inc.", O=...` or `CN=Ervin Walter, O=...`.
export function publisherName(subject: string): string | undefined {
  const match = /(?:^|,\s*)CN=("(?:[^"]|"")*"|[^,]*)/.exec(subject);
  if (!match) return undefined;
  const value = match[1]!.trim();
  return value.startsWith("\"") ? value.slice(1, -1).replaceAll("\"\"", "\"") : value;
}

// Returns why the file fails the gate, or undefined when it passes.
export function signatureFailure(file: string, signature: Signature, publisher: string): string | undefined {
  if (signature.status !== "Valid") return `status ${signature.status}`;
  if (!signature.timestamped) return "no timestamp";
  const actual = publisherName(signature.subject);
  if (actual === publisher) return undefined;
  const vendor = VENDOR_FILES.find(item => item.name.test(basename(file)));
  if (vendor && actual === vendor.publisher) return undefined;
  const expected = vendor ? `${publisher} or ${vendor.publisher}` : publisher;
  return `publisher ${actual ?? "(none)"}, expected ${expected}`;
}

// Reads the signatures of all files with one PowerShell process. The paths go
// in through standard input, so no path is part of a command line.
function readSignatures(files: string[]): Signature[] {
  const script = [
    "$ErrorActionPreference = 'Stop'",
    "$paths = [Console]::In.ReadToEnd() | ConvertFrom-Json",
    "$result = foreach ($path in $paths) {",
    "  $s = Get-AuthenticodeSignature -LiteralPath $path",
    "  [pscustomobject]@{",
    "    status = $s.Status.ToString()",
    "    subject = if ($s.SignerCertificate) { $s.SignerCertificate.Subject } else { '' }",
    "    timestamped = $null -ne $s.TimeStamperCertificate",
    "  }",
    "}",
    "ConvertTo-Json -Compress -InputObject @($result)",
  ].join("\n");
  const output = execFileSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
    input: JSON.stringify(files),
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  const signatures = JSON.parse(output) as Signature[];
  if (signatures.length !== files.length) throw new Error(`PowerShell returned ${signatures.length} results for ${files.length} files`);
  return signatures;
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  if (command === "sign" && rest.length === 1) {
    const { program, args } = signCommand(rest[0]!, process.env);
    const result = spawnSync(program, args, { stdio: "inherit" });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      console.error(`windows-signing: SignTool failed with exit code ${result.status} for ${rest[0]}`);
      process.exit(1);
    }
  } else if (command === "verify" && rest.length === 2) {
    const publisher = process.env.WINDOWS_PUBLISHER;
    if (!publisher) {
      console.error("windows-signing: WINDOWS_PUBLISHER is not set");
      process.exit(1);
    }
    const files = verifyTargets(rest[0]!, rest[1]!);
    const signatures = readSignatures(files);
    let failures = 0;
    files.forEach((file, i) => {
      const failure = signatureFailure(file, signatures[i]!, publisher);
      if (failure) failures++;
      console.log(`${failure ? "FAIL" : "ok"}  ${file}  ${failure ?? publisherName(signatures[i]!.subject)}`);
    });
    if (failures > 0) {
      console.error(`windows-signing: ${failures} of ${files.length} files failed the signature check`);
      process.exit(1);
    }
    console.log(`windows-signing: ${files.length} files have valid, timestamped signatures`);
  } else {
    console.error("usage: windows-signing.ts sign <file> | windows-signing.ts verify <folder> <installer>");
    process.exit(2);
  }
}
