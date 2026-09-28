// Run the production recognition stages on local audio. Keep raw output local.
import { join, resolve } from 'node:path';
import { mkdir, copyFile, chmod } from 'node:fs/promises';

const [pathsFile, wav, helper, directory] = process.argv.slice(2);
if (!pathsFile || !wav || !helper || !directory) {
  throw new Error('Use route-matrix.ts MODEL-PATHS.json WAV INSTALLED-HELPER OUTPUT-DIRECTORY');
}
const root = resolve(import.meta.dir, '../..');
const output = resolve(directory);
const seconds = Number(process.env.LONGHAND_ROUTE_MATRIX_SECONDS ?? 30);
if (!Number.isSafeInteger(seconds) || seconds <= 0) throw new Error('Invalid excerpt duration');
await mkdir(output, { recursive: true });
const paths: { id: string; path: string; sha256: string }[] = await Bun.file(pathsFile).json();
const catalog = await Bun.file(join(root, 'crates/models/catalog.json')).json();
for (const artifact of paths) {
  const hash = new Bun.CryptoHasher('sha256');
  for await (const chunk of Bun.file(artifact.path).stream()) hash.update(chunk);
  if (hash.digest('hex') !== artifact.sha256) throw new Error(`Artifact mismatch: ${artifact.id}`);
}
const cases: { model: string; speaker: string; runtime: string }[] = [];
for (const model of catalog.models.filter((item: { kind: string }) => item.kind === 'asr')) {
  for (const speaker of ['off', 'pyannote', 'sortformer', 'nemotron']) {
    cases.push({ model: model.id, speaker, runtime: 'transcribe' });
  }
}
for (const model of ['nemotron-speech-streaming-en-0.6b-q80', 'nemotron-3.5-asr-streaming-0.6b-q80', 'parakeet-tdt-0.6b-v3-q80']) {
  for (const speaker of ['sortformer', 'nemotron']) cases.push({ model, speaker, runtime: 'nemo' });
}
// Compile once so every case uses the same implementation while work continues.
const build = Bun.spawn(['cargo', 'test', '-p', 'pipeline', '--test', 'recognition_native', '--features', 'diarize/coreml', '--no-run', '--message-format=json'], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
const [messages, buildErrors, buildCode] = await Promise.all([
  new Response(build.stdout).text(), new Response(build.stderr).text(), build.exited,
]);
await Bun.write(join(output, 'build.log'), buildErrors);
if (buildCode !== 0) throw new Error(`Test build failed; see ${join(output, 'build.log')}`);
const executable = messages.split('\n').filter(Boolean).map(line => JSON.parse(line))
  .find(message => message.reason === 'compiler-artifact' && message.target?.name === 'recognition_native' && message.executable)?.executable;
if (!executable) throw new Error('The route test executable is absent');
const testBinary = join(output, process.platform === 'win32' ? 'recognition-test.exe' : 'recognition-test');
await copyFile(executable, testBinary);
await chmod(testBinary, 0o755);
const results: object[] = [];
let failures = 0;
for (const item of cases) {
  const id = `${item.model}--${item.speaker}--${item.runtime}`;
  const started = performance.now();
  const child = Bun.spawn([testBinary, '--exact', 'production_route_recording_excerpt', '--ignored', '--nocapture'], {
    cwd: root,
    env: {
      ...process.env,
      LONGHAND_ROUTE_MODEL: item.model, LONGHAND_ROUTE_SPEAKER: item.speaker,
      LONGHAND_ROUTE_RUNTIME: item.runtime, LONGHAND_ROUTE_PATHS: resolve(pathsFile),
      LONGHAND_NEMO_HELPER: resolve(helper), LONGHAND_ROUTE_GPU: '0',
      LONGHAND_ROUTE_WAV: resolve(wav), LONGHAND_ROUTE_SECONDS: String(seconds),
      LONGHAND_ROUTE_OUTPUT: join(output, `${id}.json`),
    },
    stdout: 'pipe', stderr: 'pipe',
  });
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
  ]);
  await Bun.write(join(output, `${id}.log`), stdout + stderr);
  if (code !== 0) failures++;
  results.push({ ...item, id, seconds, exitCode: code, elapsedMs: performance.now() - started });
  await Bun.write(join(output, 'summary.json'), JSON.stringify({
    scope: 'production recognition stages; excludes live capture, summary and publication', results, failures,
  }, null, 2) + '\n');
  console.log(`${code === 0 ? 'PASS' : 'FAIL'} ${id}`);
}
if (failures) process.exitCode = 1;
