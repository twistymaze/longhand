// Settings that the scripts give to the child processes that they start.

import { spawnSync } from "node:child_process";

/// The output buffer of a child process. Node's default of 1 MB is too small
/// for some tools: cargo-deny, for example, writes more JSON than that.
export const MAX_OUTPUT_BYTES = 256 * 1024 * 1024;

/// Runs `command` in `cwd` and returns its standard output. Throws when the
/// program cannot start or ends with an exit code that is not 0.
export function run(command: string[], cwd: string): string {
  const [program, ...args] = command;
  const result = spawnSync(program, args, { cwd, encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES });
  // spawnSync does not throw when it cannot start the program: it sets `error`.
  if (result.error) throw new Error(`cannot run ${program}: ${result.error.message}`);
  if (result.status !== 0) {
    throw new Error(`${command.join(" ")} failed with exit code ${result.status}: ${result.stderr}`);
  }
  return result.stdout;
}
