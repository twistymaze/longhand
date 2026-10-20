// Process and file helpers for the recognition checks, with Node built-ins only.
import { spawn, type SpawnOptions } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, statSync } from 'node:fs';

/// The size and the SHA-256 hex digest of a file. It reads the file as a stream, because a model file is large.
export async function sha256File(path: string): Promise<{ size: number; sha256: string }> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk as Buffer);
  return { size: statSync(path).size, sha256: hash.digest('hex') };
}

/// Runs `command` and gives its standard output, its standard error and its exit code.
/// The exit code is null when a signal stopped the process.
export function capture(command: string[], options: SpawnOptions = {}): Promise<{ stdout: string; stderr: string; code: number | null }> {
  const [program, ...args] = command;
  return new Promise((done, fail) => {
    const child = spawn(program, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout!.setEncoding('utf8').on('data', (text: string) => { stdout += text; });
    child.stderr!.setEncoding('utf8').on('data', (text: string) => { stderr += text; });
    child.on('error', fail);
    child.on('close', (code) => done({ stdout, stderr, code }));
  });
}

/// Runs `command` with the standard output and standard error of this process, and gives its exit code.
export function inherit(command: string[], options: SpawnOptions = {}): Promise<number | null> {
  const [program, ...args] = command;
  return new Promise((done, fail) => {
    const child = spawn(program, args, { ...options, stdio: ['ignore', 'inherit', 'inherit'] });
    child.on('error', fail);
    child.on('close', done);
  });
}
