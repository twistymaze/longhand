// Component checks for every proposed NeMo ASR/speaker pairing.
// Full app route checks must also exercise scheduling, joining, and persistence.
import { join, resolve } from 'node:path';
import { readFileSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import { capture, sha256File } from './process.ts';
const [helper, modelsDirectory, wav, outputDirectory] = process.argv.slice(2);
if (!helper || !modelsDirectory || !wav || !outputDirectory) throw new Error('Use helper-matrix.ts HELPER MODEL-DIRECTORY WAV OUTPUT-DIRECTORY');
const output = resolve(outputDirectory);
await mkdir(output, { recursive: true });
const catalog = JSON.parse(readFileSync(join(import.meta.dirname, '../../crates/models/catalog.json'), 'utf8'));
const speakers = catalog.models.filter((model: any) => ['sortformer-v2-q80', 'nemotron-3-diarization-q80'].includes(model.id));
const transcription = catalog.runtime_artifacts;
for (const artifact of [...speakers, ...transcription]) {
  const file = await sha256File(join(modelsDirectory, artifact.filename));
  if (file.size !== artifact.size_bytes || file.sha256 !== artifact.sha256) throw new Error(`Artifact mismatch: ${artifact.id}`);
}
const results: object[] = [];
let failures = 0;
for (const gpu of [-1, 0]) {
  for (const speaker of speakers) {
    for (const asr of [null, ...transcription]) {
      const route = !asr ? 'separate' : asr.streaming ? 'combined_stream' : 'combined_offline';
      const id = `${asr?.id ?? 'speaker-only'}--${speaker.id}--${gpu === -1 ? 'cpu' : 'gpu'}`;
      const command = [process.execPath, join(import.meta.dirname, '../test-nemo.ts'), '--helper', resolve(helper), '--speaker', join(resolve(modelsDirectory), speaker.filename), '--route', route, '--wav', resolve(wav), '--sample-count', '480000', '--gpu', String(gpu), '--timeout-ms', '600000', '--output', join(output, `${id}.json`)];
      if (asr) command.push('--asr', join(resolve(modelsDirectory), asr.filename));
      const { stdout, stderr, code } = await capture(command);
      await writeFile(join(output, `${id}.log`), stdout + stderr);
      let metrics: unknown = null;
      if (code === 0) metrics = JSON.parse(stdout.trim().split('\n').at(-1)!);
      else failures++;
      results.push({ id, route, gpu, asr: asr?.sha256 ?? null, speaker: speaker.sha256, exitCode: code, metrics });
      await writeFile(join(output, 'summary.json'), JSON.stringify({ componentOnly: true, sampleCount: 480000, results, failures }, null, 2) + '\n');
      console.log(`${code === 0 ? 'PASS' : 'FAIL'} ${id}`);
    }
  }
}
if (failures) process.exitCode = 1;
