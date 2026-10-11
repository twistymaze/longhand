// Computes whether a release is due, its version and its notes (KTD1, KTD10).
//
//   bun scripts/release-version.ts <notes file>
//     Run from the repository root. Prints `due=<true|false>` and
//     `version=<X.Y.Z>` for $GITHUB_OUTPUT. When a release is due, it writes
//     the release notes to <notes file>.
//
// git-cliff computes the version from the release tags and the commits with
// the rules in cliff.toml (R9). With no release tag, git-cliff returns the
// initial tag v1.0.0 (R8). When the bumped version is the same as the latest
// release tag, only skipped commits follow that tag, and no release is due.

import { writeFileSync } from "node:fs";
import { join } from "node:path";

const RELEASE_TAG = /^v(\d+\.\d+\.\d+)$/;

/// The commands that the script runs. Tests replace them with a fake.
export type Tools = {
  /// All tags that are reachable from HEAD.
  tags(): string[];
  /// Runs git-cliff with the arguments and returns its standard output.
  cliff(args: string[]): string;
};

export type ReleaseFacts = { due: boolean; version: string; notes: string | null };

/// The highest `v<major>.<minor>.<patch>` tag, or null when there is none.
export function latestReleaseTag(tags: string[]): string | null {
  const versions = tags.flatMap((tag) => tag.match(RELEASE_TAG)?.[1] ?? []);
  if (versions.length === 0) return null;
  versions.sort((a, b) => Bun.semver.order(a, b));
  return `v${versions[versions.length - 1]}`;
}

/// A release is due when no release tag exists, or when the bumped version
/// is not the latest release tag.
export function decide(latestTag: string | null, bumped: string): { due: boolean; version: string } {
  const version = bumped.trim().match(RELEASE_TAG)?.[1];
  if (!version) throw new Error(`git-cliff output is not a version: "${bumped.trim()}"`);
  return { due: latestTag === null || `v${version}` !== latestTag, version };
}

export function releaseFacts(tools: Tools): ReleaseFacts {
  const latest = latestReleaseTag(tools.tags());
  const { due, version } = decide(latest, tools.cliff(["--bumped-version"]));
  const notes = due ? tools.cliff(["--unreleased", "--tag", `v${version}`, "--strip", "all"]).trim() + "\n" : null;
  return { due, version, notes };
}

function run(command: string[], cwd: string): string {
  let result;
  try {
    result = Bun.spawnSync(command, { cwd });
  } catch (error) {
    throw new Error(`cannot run ${command[0]}: ${(error as Error).message}`);
  }
  if (result.exitCode !== 0) {
    throw new Error(`${command.join(" ")} failed with exit code ${result.exitCode}: ${result.stderr.toString()}`);
  }
  return result.stdout.toString();
}

/// The real git and git-cliff in `repository`, with the configuration file `config`.
export function systemTools(repository: string, config: string): Tools {
  return {
    tags: () => run(["git", "tag", "--merged", "HEAD"], repository).split("\n").filter(Boolean),
    cliff: (args) => run(["git-cliff", "--config", config, "--repository", repository, ...args], repository),
  };
}

if (import.meta.main) {
  const [notesFile, ...rest] = process.argv.slice(2);
  if (!notesFile || rest.length > 0) {
    console.error("usage: release-version.ts <notes file>");
    process.exit(2);
  }
  const facts = releaseFacts(systemTools(process.cwd(), join(import.meta.dir, "..", "cliff.toml")));
  if (facts.notes !== null) writeFileSync(notesFile, facts.notes);
  console.log(`due=${facts.due}`);
  console.log(`version=${facts.version}`);
}
