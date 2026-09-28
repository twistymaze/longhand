// Native helper protocol smoke test. Optional model/audio inputs stay local.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const args = process.argv.slice(2);
function option(name: string, fallback = ""): string { const i = args.indexOf(name); return i < 0 ? fallback : args[i + 1]!; }
const helper = option("--helper");
if (!helper) throw new Error("Use --helper PATH [--speaker PATH --asr PATH --route separate|combined_stream|combined_offline|asr_only --streaming true|false --wav PATH --gpu -1]");
if (args.includes("--parent-death")) {
  const workerCode = 'const {spawn}=require("node:child_process");const child=spawn(process.argv[1],["--parent",String(process.pid)],{stdio:["pipe","ignore","ignore"]});console.log(child.pid);setInterval(()=>{},1000);';
  const worker = spawn(process.execPath, ["-e", workerCode, helper], { stdio: ["ignore", "pipe", "pipe"] });
  const pid = await new Promise<number>((resolve, reject) => { worker.stdout.once("data", data => resolve(Number(data.toString().trim()))); worker.once("error", reject); });
  assert(Number.isSafeInteger(pid) && pid > 0);
  worker.kill("SIGKILL");
  let alive = true;
  const start = performance.now();
  while (performance.now() - start < 5000) { try { process.kill(pid, 0); } catch { alive = false; break; } await Bun.sleep(50); }
  if (alive) { try { process.kill(pid, "SIGKILL"); } catch {} }
  assert(!alive, "Native helper survived parent death");
  console.log(JSON.stringify({ passed: true, test: "parent-death", elapsedMs: performance.now() - start }));
  process.exit(0);
}
const speaker = option("--speaker");
const route = option("--route", "separate");
const inference = Boolean(speaker || option("--asr"));
const cancelStage = option("--cancel-stage");
if (cancelStage) {
  assert(["load", "push", "finish"].includes(cancelStage));
  assert(speaker, "Cancellation tests require --speaker");
  const processUnderTest = spawn(helper, ["--parent", String(process.pid)], { stdio: ["pipe", "pipe", "pipe"] });
  processUnderTest.stderr.resume();
  let sequence = 0, buffer = Buffer.alloc(0), cancelStarted = 0, terminalResult = false;
  function request(value: object, pcm = Buffer.alloc(0)) {
    const json = Buffer.from(JSON.stringify({ version: 1, generation: 77, epoch: 0, seq: sequence++, ...value }));
    const header = Buffer.alloc(8); header.writeUInt32LE(json.length); header.writeUInt32LE(pcm.length, 4);
    processUnderTest.stdin.write(Buffer.concat([header, json, pcm]));
  }
  const result = await new Promise<number>((resolve, reject) => {
    const timeout = setTimeout(() => { processUnderTest.kill("SIGKILL"); reject(new Error("Cancellation test timed out")); }, Number(option("--timeout-ms", "180000")));
    function cancel() { setTimeout(() => { cancelStarted = performance.now(); processUnderTest.kill("SIGKILL"); }, Number(option("--cancel-delay-ms", "1"))); }
    processUnderTest.once("error", reject);
    processUnderTest.stdin.on("error", error => { if (!cancelStarted) reject(error); });
    processUnderTest.once("close", code => { clearTimeout(timeout); if (!cancelStarted || terminalResult || code === 0) reject(new Error("Helper finished or failed before cancellation")); else resolve(performance.now() - cancelStarted); });
    processUnderTest.stdout.on("data", data => {
      try {
        buffer = Buffer.concat([buffer, data]);
        while (buffer.length >= 8) {
          const length = buffer.readUInt32LE(0); assert(length > 0 && length <= 1048576); assert.equal(buffer.readUInt32LE(4), 0);
          if (buffer.length < 8 + length) break;
          const event = JSON.parse(buffer.subarray(8, 8 + length).toString()); buffer = buffer.subarray(8 + length);
          if (event.event === "error" || event.event === "finished") terminalResult = true;
          if (event.event === "hello" && cancelStage === "load") cancel();
          if (event.event === "ready" && cancelStage !== "load") { request({ command: "push", offset: 0 }, Buffer.alloc(16000 * 4)); if (cancelStage === "push") cancel(); }
          if (event.event === "ack" && cancelStage === "finish") { request({ command: "finish" }); cancel(); }
        }
      } catch (error) { processUnderTest.kill("SIGKILL"); reject(error); }
    });
    request({ command: "start", route, speaker_model: speaker, asr_model: option("--asr"), gpu: Number(option("--gpu", "-1")), epoch_start: 0, max_epoch_samples: 16000 });
  });
  assert(result < 2000, "Cancellation did not reap the helper within two seconds");
  console.log(JSON.stringify({ passed: true, test: "cancel", phase: cancelStage, elapsedMs: result }));
  process.exit(0);
}
const child = spawn(helper, ["--parent", String(process.pid)], { stdio: ["pipe", "pipe", "pipe"] });
let diagnosticBytes = 0;
child.stderr.on("data", (data: Buffer) => { diagnosticBytes += data.length; });
let bytes = Buffer.alloc(0);
let completed = false;
child.stdin.on("error", error => fail(error));
let receivedEnd = 0;
let wordCount = 0;
let segmentCount = 0;
let sequence = 0;
const epochStart = Number(option("--epoch-start", "32000"));
assert(Number.isSafeInteger(epochStart) && epochStart >= 0);
let rejectFailure!: (error: Error) => void;
const failure = new Promise<never>((_, reject) => { rejectFailure = reject; });
function fail(error: unknown) { rejectFailure(error instanceof Error ? error : new Error(String(error))); child.kill("SIGKILL"); }
function wait<T>(promise: Promise<T>): Promise<T> { return Promise.race([promise, failure]); }
let acceptedEnd = 0;
let backend = "";
const startedAt = performance.now();
let readyAt = 0;
let finishAt = 0;
let coverageStart = 0;
let coverageEnd = 0;
let speakerFinalWords = 0;
let interimUpdates = 0;
let interimTaggedUpdates = 0;
let interimSpeakerRevisions = 0;
let previousInterim = new Map<string, number>();
let wordRevisionUpdates = 0;
const seenWordKeys = new Set<string>();
const finalWords = new Map<string, any>();
const texts = new Map<number, string>();
let currentSegments: any[] = [];
let pendingSegments: any[] = [];
const pending = new Map<number, () => void>();
const timer = setTimeout(() => fail(new Error("Helper test timed out")), Number(option("--timeout-ms", "180000")));
function send(metadata: object, samples = new Float32Array()) {
  const seq = sequence++;
  const json = Buffer.from(JSON.stringify({ version: 1, generation: 19, epoch: 2, seq, ...metadata }));
  const header = Buffer.alloc(8); header.writeUInt32LE(json.length); header.writeUInt32LE(samples.length * 4, 4);
  const pcm = Buffer.alloc(samples.length * 4);
  for (let i = 0; i < samples.length; i++) pcm.writeFloatLE(samples[i]!, i * 4);
  child.stdin.write(Buffer.concat([header, json, pcm]));
  return seq;
}
const exit = new Promise<number | null>((resolve, reject) => { child.once("error", error => { fail(error); reject(error); }); child.once("close", code => { if (!completed) fail(new Error(`Helper exited before completion (${code})`)); resolve(code); }); });
const ready = new Promise<void>((resolve, reject) => {
  child.stdout.on("data", (data: Buffer) => {
    try {
      bytes = Buffer.concat([bytes, data]);
      while (bytes.length >= 8) {
        const length = bytes.readUInt32LE(0), pcmLength = bytes.readUInt32LE(4);
        assert(length > 0 && length <= 1048576); assert.equal(pcmLength, 0);
        if (bytes.length < 8 + length) return;
        const message = JSON.parse(bytes.subarray(8, 8 + length).toString()); bytes = bytes.subarray(8 + length);
        if (inference) { assert.equal(message.generation, 19); assert.equal(message.epoch, 2); }
        assert.equal(message.version, 1);
        if (message.event === "hello") assert.equal(message.runtime_revision, "97a15afa5caa9bce5baaa86c1184103877af4101");
        if (message.event === "ready") { backend = message.backend; readyAt = performance.now(); resolve(); }
        if (message.event === "error") { if (inference) fail(new Error(message.message)); else { completed = true; resolve(); } }
        if (message.event === "ack") { receivedEnd = message.accepted_end; pending.get(message.seq)?.(); pending.delete(message.seq); }
        if (message.event === "text") texts.set(message.result_id, message.text);
        if (message.event === "punctuation") texts.set(message.result_id, (texts.get(message.result_id) ?? "") + message.text);
        if (message.event === "segments_begin") pendingSegments = [];
        if (message.event === "segments_end") currentSegments = pendingSegments;
        if (message.event === "interim") {
          interimUpdates++;
          const next = new Map<string, number>();
          for (const word of message.words) {
            const key = `${message.result_id}:${word.start}:${word.text}`;
            if (word.speaker > 0) interimTaggedUpdates++;
            if (previousInterim.has(key) && previousInterim.get(key) !== word.speaker) interimSpeakerRevisions++;
            next.set(key, word.speaker);
          }
          previousInterim = next;
        }
        if (message.event === "words") {
          wordCount += message.words.length;
          for (const word of message.words) {
            const key = `${message.result_id}:${word.index}`;
            if (seenWordKeys.has(key)) wordRevisionUpdates++;
            seenWordKeys.add(key);
            finalWords.set(key, { ...word, result_id: message.result_id });
            if (word.speaker_final) speakerFinalWords++;
            assert(word.start <= word.end); assert(word.end <= acceptedEnd);
          }
        }
        if (message.event === "segments") { segmentCount += message.segments.length; pendingSegments.push(...message.segments); for (const segment of message.segments) { assert(segment.end <= message.accepted_end || segment.end <= acceptedEnd); assert(segment.start <= segment.end); } }
        if (message.event === "finished") { assert.equal(message.coverage_end, acceptedEnd); assert.equal(message.coverage_start, epochStart); coverageStart = message.coverage_start; coverageEnd = message.coverage_end; completed = true; }
      }
    } catch (error) { fail(error); reject(error); }
  });
  child.once("exit", () => { if (!completed && !backend) reject(new Error("Helper exited before ready")); });
});
try {
  if (!inference) {
    send({ version: 99, command: "start" });
    await wait(ready); assert.notEqual(await wait(exit), 0); assert(completed);
    console.log("Native protocol rejection passed");
  } else {
    let samples = new Float32Array(16000);
    const wav = option("--wav");
    if (wav) {
      const file = readFileSync(wav); assert.equal(file.toString("ascii", 0, 4), "RIFF");
      let format = 0, bits = 0, data = Buffer.alloc(0);
      for (let at = 12; at + 8 <= file.length;) {
        const name = file.toString("ascii", at, at + 4), size = file.readUInt32LE(at + 4), value = file.subarray(at + 8, at + 8 + size);
        if (name === "fmt ") { format = value.readUInt16LE(0); assert.equal(value.readUInt16LE(2), 1); assert.equal(value.readUInt32LE(4), 16000); bits = value.readUInt16LE(14); }
        if (name === "data") data = value;
        at += 8 + size + (size % 2);
      }
      assert((format === 1 && bits === 16) || (format === 3 && bits === 32));
      samples = new Float32Array(data.length / (bits / 8));
      for (let i = 0; i < samples.length; i++) samples[i] = format === 1 ? data.readInt16LE(i * 2) / 32768 : data.readFloatLE(i * 4);
    }
    const sampleStart = Number(option("--sample-start", "0"));
    const sampleCount = Number(option("--sample-count", String(samples.length - sampleStart)));
    assert(Number.isSafeInteger(sampleStart) && sampleStart >= 0 && sampleStart <= samples.length);
    assert(Number.isSafeInteger(sampleCount) && sampleCount > 0 && sampleStart + sampleCount <= samples.length);
    samples = samples.subarray(sampleStart, sampleStart + sampleCount);
    acceptedEnd = epochStart;
    send({ command: "start", route, ...(speaker ? { speaker_model: speaker } : {}), asr_model: option("--asr"), streaming: option("--streaming", "true") === "true", gpu: Number(option("--gpu", "-1")), epoch_start: epochStart, max_epoch_samples: Math.max(samples.length, 1) });
    await wait(ready);
    for (let at = 0; at < samples.length; at += 2560) {
      const chunk = samples.subarray(at, at + 2560); const offset = acceptedEnd; acceptedEnd += chunk.length;
      await wait(new Promise<void>(resolve => { const seq = send({ command: "push", offset }, chunk); pending.set(seq, resolve); }));
    }
    assert.equal(receivedEnd, acceptedEnd);
    finishAt = performance.now();
    send({ command: "finish" });
    assert.equal(await wait(exit), 0); assert(completed); assert.equal(bytes.length, 0);
    const attributedWords = [...finalWords.values()].filter(word => word.speaker > 0).length;
    const uniqueSpeakers = [...new Set([...finalWords.values(), ...currentSegments].map(item => item.speaker).filter(speaker => speaker > 0))].sort();
    const finalSegments = currentSegments.filter(segment => segment.end > segment.start && segment.speaker > 0);
    if (wav) {
      if (route === "asr_only") {
        assert.equal(attributedWords, 0, "ASR-only invented speaker labels");
        assert.equal(currentSegments.length, 0, "ASR-only performed speaker analysis");
        assert([...finalWords.values()].every(word => word.speaker === 0 && word.speaker_final), "ASR-only labels must be explicitly absent and final");
      } else {
        assert(finalSegments.length > 0 || (route === "combined_stream" && attributedWords > 0), "Real audio produced no speaker assignments");
      }
      if (route !== "separate") { assert(finalWords.size > 0); assert([...texts.values()].some(text => text.trim().length > 0)); }
      if (route === "combined_stream") assert(attributedWords > 0, "Combined streaming produced no attributed words");
    }
    const metrics = { passed: true, route, backend, samples: samples.length, coverageStart, coverageEnd, loadMs: readyAt - startedAt, processMs: finishAt - readyAt, finishMs: performance.now() - finishAt, totalMs: performance.now() - startedAt, wordUpdates: wordCount, uniqueWords: seenWordKeys.size, attributedWords, uniqueSpeakers, wordRevisionUpdates, speakerFinalWords, interimUpdates, interimTaggedUpdates, interimSpeakerRevisions, segmentUpdates: segmentCount, finalSegments: finalSegments.length, diagnosticBytes };
    const output = option("--output");
    if (output) await Bun.write(output, JSON.stringify({ metrics, texts: [...texts.entries()], words: [...finalWords.values()], segments: currentSegments }, null, 2) + "\n");
    console.log(JSON.stringify(metrics));
  }
} finally { clearTimeout(timer); child.kill("SIGKILL"); }
