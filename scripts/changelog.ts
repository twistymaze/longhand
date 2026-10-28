// Writes the changelog from the commits on main after v1.0.0 (monthly
// changelog plan, U1).
//
//   node scripts/changelog.ts write <folder>
//     Writes changelog.json, CHANGELOG.md and index.html into <folder>. The
//     release build runs it. It fails when git-cliff, the v1.0.0 tag or the
//     history is not available, because a release must have its changelog.
//   node scripts/changelog.ts local <file>
//     Writes changelog.json for a local build (beforeBuildCommand). When
//     LONGHAND_CHANGELOG is set, the build reads that file, so this command
//     writes nothing. When the changelog cannot be made, it writes the "not
//     available" file and ends without an error.
//
// Run it from the repository root. git-cliff applies the rules in cliff.toml,
// and scripts/lib/changelog.ts writes every text (KTD1).

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { changelog, html, markdown, NOT_AVAILABLE, type Changelog } from "./lib/changelog.ts";
import { MAX_OUTPUT_BYTES } from "./lib/spawn.ts";

/// The first release. The changelog holds only the commits after it.
export const FIRST_RELEASE = "v1.0.0";

function run(program: string, args: string[], cwd: string): string {
  const result = spawnSync(program, args, { cwd, encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES });
  if (result.error) throw new Error(`cannot run ${program}: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${program} ${args.join(" ")} failed with exit code ${result.status}: ${result.stderr}`);
  return result.stdout;
}

/// The changelog of the repository at `repository`, with the rules in `config`.
export function readChangelog(repository: string, config: string): Changelog {
  // A shallow clone can read some history and have no v1.0.0 tag. Then the
  // changelog would be incomplete, so the tag must be there.
  run("git", ["rev-parse", "--verify", "--quiet", `${FIRST_RELEASE}^{commit}`], repository);
  const [id, time] = run("git", ["log", "-1", "--format=%H%n%ct"], repository).trim().split("\n");
  const releases = JSON.parse(
    run("git-cliff", ["--config", config, "--repository", repository, "--context", `${FIRST_RELEASE}..HEAD`], repository),
  );
  return changelog(releases, { id, time: Number(time) });
}

/// Writes changelog.json, CHANGELOG.md and index.html into `folder`.
export function writeAll(data: Changelog, folder: string): void {
  mkdirSync(folder, { recursive: true });
  writeFileSync(join(folder, "changelog.json"), `${JSON.stringify(data)}\n`);
  writeFileSync(join(folder, "CHANGELOG.md"), markdown(data));
  writeFileSync(join(folder, "index.html"), html(data));
}

/// Writes changelog.json for a local build. Returns the reason when the
/// changelog is not available.
export function writeLocal(repository: string, config: string, file: string): string | null {
  let data: Changelog = NOT_AVAILABLE;
  let reason: string | null = null;
  try {
    data = readChangelog(repository, config);
  } catch (error) {
    reason = error instanceof Error ? error.message : String(error);
  }
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, `${JSON.stringify(data)}\n`);
  return reason;
}

if (import.meta.main) {
  const [command, target, ...rest] = process.argv.slice(2);
  const config = join(import.meta.dirname, "..", "cliff.toml");
  const repository = process.cwd();
  if (!target || rest.length > 0 || (command !== "write" && command !== "local")) {
    console.error("usage: changelog.ts write <folder> | changelog.ts local <file>");
    process.exit(2);
  }
  if (command === "write") {
    writeAll(readChangelog(repository, config), target);
  } else if (process.env.LONGHAND_CHANGELOG) {
    console.log(`changelog: the build reads LONGHAND_CHANGELOG (${process.env.LONGHAND_CHANGELOG})`);
  } else {
    const reason = writeLocal(repository, config, target);
    if (reason) console.log(`changelog: not available in this build: ${reason.trim()}`);
  }
}
