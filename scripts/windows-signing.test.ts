import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import { publisherName, signatureFailure, signCommand, verifyTargets, type Signature } from "./windows-signing.ts";

const fixtures: string[] = [];
afterEach(() => { for (const root of fixtures.splice(0)) rmSync(root, { recursive: true }); });
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "longhand-signing-"));
  fixtures.push(root);
  return root;
}

const PUBLISHER = "Ervin Walter";
const OWN = "CN=Ervin Walter, O=Ervin Walter, L=Seattle, S=Washington, C=US";
const MICROSOFT = "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
const LUNARG = "CN=\"LunarG, Inc.\", O=\"LunarG, Inc.\", L=Boulder, S=Colorado, C=US";
const OTHER = "CN=Other Vendor, O=Other Vendor, C=US";
function signed(subject: string): Signature {
  return { status: "Valid", subject, timestamped: true };
}

test("file selection finds .exe and .dll files in nested folders and ignores other types", () => {
  const root = fixture();
  mkdirSync(join(root, "nemo-runtime", "lib"), { recursive: true });
  writeFileSync(join(root, "longhand.exe"), "");
  writeFileSync(join(root, "uninstall.EXE"), "");
  writeFileSync(join(root, "whisper.dll"), "");
  writeFileSync(join(root, "nemo-runtime", "lib", "onnxruntime.DLL"), "");
  writeFileSync(join(root, "nemo-runtime", "model.onnx"), "");
  writeFileSync(join(root, "notes.txt"), "");
  writeFileSync(join(root, "dll"), "");
  const installer = join(root, "..", "Longhand_1.2.3_x64-setup.exe");
  assert.deepEqual(verifyTargets(root, installer).sort(), [
    installer,
    join(root, "longhand.exe"),
    join(root, "nemo-runtime", "lib", "onnxruntime.DLL"),
    join(root, "uninstall.EXE"),
    join(root, "whisper.dll"),
  ].sort());
});

test("an install folder with no .exe or .dll files fails", () => {
  const root = fixture();
  writeFileSync(join(root, "notes.txt"), "");
  assert.throws(() => verifyTargets(root, join(root, "setup.exe")), /no \.exe or \.dll files/);
});

test("reads the common name of a certificate subject, also a quoted name", () => {
  assert.equal(publisherName(OWN), "Ervin Walter");
  assert.equal(publisherName(LUNARG), "LunarG, Inc.");
  assert.equal(publisherName("O=No Name, C=US"), undefined);
});

test("a file with a status other than valid fails", () => {
  const failure = signatureFailure("longhand.exe", { status: "NotSigned", subject: "", timestamped: false }, PUBLISHER);
  assert.match(failure ?? "", /NotSigned/);
  assert.notEqual(signatureFailure("longhand.exe", { ...signed(OWN), status: "HashMismatch" }, PUBLISHER), undefined);
});

test("a valid file with no timestamp fails", () => {
  const failure = signatureFailure("longhand.exe", { ...signed(OWN), timestamped: false }, PUBLISHER);
  assert.match(failure ?? "", /timestamp/);
});

test("Longhand's own file signed by the expected publisher passes", () => {
  assert.equal(signatureFailure("longhand.exe", signed(OWN), PUBLISHER), undefined);
  assert.equal(signatureFailure("Longhand_1.2.3_x64-setup.exe", signed(OWN), PUBLISHER), undefined);
});

test("Longhand's own file with another publisher fails", () => {
  assert.match(signatureFailure("whisper.dll", signed(MICROSOFT), PUBLISHER) ?? "", /Microsoft Corporation/);
  assert.notEqual(signatureFailure("longhand.exe", signed("CN=Ervin Walter Jr, C=US"), PUBLISHER), undefined);
});

test("a Microsoft-signed VC runtime DLL passes with its own publisher", () => {
  for (const name of ["msvcp140.dll", "MSVCP140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll", "VCOMP140.DLL"]) {
    assert.equal(signatureFailure(name, signed(MICROSOFT), PUBLISHER), undefined, name);
  }
  assert.notEqual(signatureFailure("msvcp140.dll", signed(OTHER), PUBLISHER), undefined);
});

test("vulkan-1.dll passes with its vendor publisher or with the expected publisher", () => {
  assert.equal(signatureFailure("vulkan-1.dll", signed(LUNARG), PUBLISHER), undefined);
  assert.equal(signatureFailure("vulkan-1.dll", signed(OWN), PUBLISHER), undefined);
  assert.notEqual(signatureFailure("vulkan-1.dll", signed(MICROSOFT), PUBLISHER), undefined);
});

test("a file not on the vendor list that another publisher signed fails", () => {
  assert.notEqual(signatureFailure("ggml-vulkan.dll", signed(LUNARG), PUBLISHER), undefined);
  assert.notEqual(signatureFailure("vulkan-1.exe", signed(LUNARG), PUBLISHER), undefined);
  assert.notEqual(signatureFailure("msvcp140.dll.exe", signed(MICROSOFT), PUBLISHER), undefined);
});

test("the sign command has the digest, timestamp, dlib and metadata arguments, with the file as its own argument", () => {
  const file = "C:\\Build Folder\\longhand & co.exe";
  const command = signCommand(file, {
    LONGHAND_SIGNTOOL: "C:\\tools\\signtool.exe",
    LONGHAND_SIGNING_DLIB: "C:\\tools\\Azure.CodeSigning.Dlib.dll",
    LONGHAND_SIGNING_METADATA: "C:\\tools\\metadata.json",
  });
  assert.equal(command.program, "C:\\tools\\signtool.exe");
  assert.deepEqual(command.args, [
    "sign", "/v",
    "/fd", "SHA256",
    "/tr", "http://timestamp.acs.microsoft.com", "/td", "SHA256",
    "/dlib", "C:\\tools\\Azure.CodeSigning.Dlib.dll",
    "/dmdf", "C:\\tools\\metadata.json",
    file,
  ]);
});

test("the sign command fails when an environment variable is missing", () => {
  assert.throws(() => signCommand("a.exe", { LONGHAND_SIGNTOOL: "signtool.exe", LONGHAND_SIGNING_DLIB: "dlib.dll" }),
    /LONGHAND_SIGNING_METADATA/);
});
