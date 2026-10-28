// Computes whether a release is due, its version and its notes (KTD1, KTD10).
//
//   node scripts/release-version.ts <notes file>
//     Run from the repository root. Prints `due=<true|false>` and
//     `version=<X.Y.Z>` for $GITHUB_OUTPUT. When a release is due, it writes
//     the release notes to <notes file>.
//
// git-cliff finds whether a release is due and gives the next patch version
// with the rules in cliff.toml. git-cliff gives the commits of the release as
// JSON, and scripts/lib/changelog.ts writes the notes from it, as it writes
// the changelog. When the bumped version is the same as the
// latest release tag, only skipped commits follow that tag, and no release is
// due. With no release tag, git-cliff returns the initial tag v1.0.0, and the
// script uses it as it is. Otherwise the script reads the original commit
// subjects since the latest release tag and raises the version: a `!` after
// the type or scope of a release type gives a major, and a `+` after the type
// or scope of `feat` gives a minor (docs/releases.md).

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { releaseNotes } from "./lib/changelog.ts";
import { run } from "./lib/spawn.ts";
import { compareVersions } from "./lib/version.ts";

const RELEASE_TAG = /^v(\d+\.\d+\.\d+)$/;
// The marker grammar is also in cliff.toml (commit_preprocessors),
// scripts/check-pr-title.sh and .github/workflows/pr-title.yml. Change them
// together.
/// A subject that asks for a minor release: `feat+:` or `feat(scope)+:`.
const MINOR_MARKER = /^feat(\([^)]*\))?\+:/;
/// A subject that asks for a major release: a release type with `!`.
const MAJOR_MARKER = /^(feat|fix|perf|refactor|revert|build)(\([^)]*\))?!:/;

/// The commands that the script runs. Tests replace them with a fake.
export type Tools = {
  /// All tags that are reachable from HEAD.
  tags(): string[];
  /// Runs git-cliff with the arguments and returns its standard output.
  cliff(args: string[]): string;
  /// The subjects of the commits after the tag `since`, up to HEAD.
  subjects(since: string): string[];
};

export type ReleaseFacts = { due: boolean; version: string; notes: string | null };

/// The highest `v<major>.<minor>.<patch>` tag, or null when there is none.
export function latestReleaseTag(tags: string[]): string | null {
  const versions = tags.flatMap((tag) => tag.match(RELEASE_TAG)?.[1] ?? []);
  if (versions.length === 0) return null;
  versions.sort(compareVersions);
  return `v${versions[versions.length - 1]}`;
}

/// A release is due when no release tag exists, or when the bumped version
/// is not the latest release tag.
export function decide(latestTag: string | null, bumped: string): { due: boolean; version: string } {
  const version = bumped.trim().match(RELEASE_TAG)?.[1];
  if (!version) throw new Error(`git-cliff output is not a version: "${bumped.trim()}"`);
  return { due: latestTag === null || `v${version}` !== latestTag, version };
}

/// The version of a due release after the latest tag `latestTag`. git-cliff
/// must give the next patch. A marker in a subject raises it from the tag.
function raise(latestTag: string, bumped: string, subjects: string[]): string {
  const [major, minor, patch] = latestTag.slice(1).split(".").map(Number);
  const nextPatch = `${major}.${minor}.${patch + 1}`;
  if (bumped !== nextPatch) {
    throw new Error(`git-cliff gave ${bumped} after ${latestTag}, not the next patch ${nextPatch}; check bump_type in cliff.toml`);
  }
  if (subjects.some((subject) => MAJOR_MARKER.test(subject))) return `${major + 1}.0.0`;
  if (subjects.some((subject) => MINOR_MARKER.test(subject))) return `${major}.${minor + 1}.0`;
  return nextPatch;
}

export function releaseFacts(tools: Tools): ReleaseFacts {
  const latest = latestReleaseTag(tools.tags());
  const { due, version: bumped } = decide(latest, tools.cliff(["--bumped-version"]));
  const version = due && latest !== null ? raise(latest, bumped, tools.subjects(latest)) : bumped;
  const notes = due ? releaseNotes(JSON.parse(tools.cliff(["--context", "--unreleased", "--tag", `v${version}`]))) : null;
  return { due, version, notes };
}

/// The real git and git-cliff in `repository`, with the configuration file `config`.
export function systemTools(repository: string, config: string): Tools {
  return {
    tags: () => run(["git", "tag", "--merged", "HEAD"], repository).split("\n").filter(Boolean),
    cliff: (args) => run(["git-cliff", "--config", config, "--repository", repository, ...args], repository),
    subjects: (since) => run(["git", "log", "--format=%s", `${since}..HEAD`], repository).split("\n").filter(Boolean),
  };
}

if (import.meta.main) {
  const [notesFile, ...rest] = process.argv.slice(2);
  if (!notesFile || rest.length > 0) {
    console.error("usage: release-version.ts <notes file>");
    process.exit(2);
  }
  const facts = releaseFacts(systemTools(process.cwd(), join(import.meta.dirname, "..", "cliff.toml")));
  if (facts.notes !== null) writeFileSync(notesFile, facts.notes);
  console.log(`due=${facts.due}`);
  console.log(`version=${facts.version}`);
}
