// Publishes one preview build to the builds repository (KTD1, KTD2).
//
//   bun scripts/publish-preview.ts version <config version> <committer time> <job start>
//     Prints the preview version (KTD5). Fails when the committer time is more
//     than 10 minutes after the job start.
//   bun scripts/publish-preview.ts publish <version> <dmg> <archive> <signature>
//     Publishes the build and writes preview/latest.json. Reads the token from
//     BUILDS_TOKEN and the repository from BUILDS_REPOSITORY.
//
// The order is check, draft, check again, publish, manifest, pruning. A job
// that stops early leaves at most a draft, which no user sees. A build whose
// version is not higher than the manifest's version publishes nothing and
// ends without an error, because the manifest already names this build or a
// newer one.

import { readFileSync } from "node:fs";
import { basename } from "node:path";

export const TAG_PREFIX = "preview-";
export const MANIFEST_PATH = "preview/latest.json";
export const KEEP_RELEASES = 20;
const MAX_CLOCK_SKEW_MS = 10 * 60 * 1000;

/// `<config version>-preview.<UTC committer time as YYYYMMDDHHMMSS>` (R6).
export function previewVersion(configVersion: string, committerTime: string): string {
  if (!/^\d+\.\d+\.\d+$/.test(configVersion)) throw new Error(`config version is not X.Y.Z: ${configVersion}`);
  const time = new Date(committerTime);
  if (Number.isNaN(time.getTime())) throw new Error(`committer time is not a date: ${committerTime}`);
  const stamp = time.toISOString().slice(0, 19).replace(/[-T:]/g, "");
  return `${configVersion}-preview.${stamp}`;
}

/// A commit time in the future would block all later builds until that time
/// (KTD5), so a time more than 10 minutes after the job start fails the job.
export function checkCommitTime(committerTime: string, jobStart: string): void {
  const commit = new Date(committerTime).getTime();
  const start = new Date(jobStart).getTime();
  if (Number.isNaN(commit) || Number.isNaN(start)) throw new Error("committer time or job start is not a date");
  if (commit - start > MAX_CLOCK_SKEW_MS) {
    throw new Error(`committer time ${committerTime} is more than 10 minutes after the job start ${jobStart}`);
  }
}

export function tagFor(version: string): string {
  const stamp = version.split("-preview.")[1];
  if (!stamp) throw new Error(`not a preview version: ${version}`);
  return `${TAG_PREFIX}${stamp}`;
}

export function assetUrl(repository: string, tag: string, name: string): string {
  return `https://github.com/${repository}/releases/download/${tag}/${name}`;
}

export type Manifest = {
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, { signature: string; url: string }>;
};

/// The static update manifest that tauri-plugin-updater reads.
export function buildManifest(args: { version: string; pubDate: string; url: string; signature: string }): Manifest {
  return {
    version: args.version,
    notes: `Preview build ${args.version}`,
    pub_date: args.pubDate,
    platforms: { "darwin-aarch64": { signature: args.signature.trim(), url: args.url } },
  };
}

/// True only when `version` is higher than the manifest's version. No
/// manifest yet means that any build is higher.
export function isNewer(version: string, manifestVersion: string | null): boolean {
  if (manifestVersion === null) return true;
  return Bun.semver.order(version, manifestVersion) === 1;
}

export type Release = { id: number; tag_name: string; draft: boolean };

/// The releases to delete: each preview draft except the current one, and
/// each published preview release after the 20 newest. The tag holds the UTC
/// timestamp, so text order is time order. A release without the prefix is
/// never selected.
export function releasesToPrune(releases: Release[], currentTag: string, keep = KEEP_RELEASES): Release[] {
  const preview = releases.filter((r) => r.tag_name.startsWith(TAG_PREFIX));
  const drafts = preview.filter((r) => r.draft && r.tag_name !== currentTag);
  const published = preview.filter((r) => !r.draft).sort((a, b) => (a.tag_name < b.tag_name ? 1 : -1));
  return [...drafts, ...published.slice(keep)];
}

/// The GitHub calls that the publish flow makes. Tests replace them.
export type BuildsApi = {
  readManifest(): Promise<{ manifest: Manifest; sha: string } | null>;
  /// Returns false when the SHA no longer matches the file (another write won).
  writeManifest(manifest: Manifest, sha: string | null): Promise<boolean>;
  listReleases(): Promise<Release[]>;
  createDraft(tag: string, version: string): Promise<Release>;
  uploadAsset(release: Release, path: string): Promise<void>;
  publishRelease(release: Release): Promise<void>;
  deleteRelease(release: Release): Promise<void>;
};

export type PublishResult = "published" | "skipped";

export async function publish(
  api: BuildsApi,
  args: { repository: string; version: string; dmg: string; archive: string; signature: string; now: () => Date },
  log: (line: string) => void = console.log,
): Promise<PublishResult> {
  const tag = tagFor(args.version);
  let current = await api.readManifest();
  if (!isNewer(args.version, current?.manifest.version ?? null)) {
    log(`${args.version} is not higher than ${current?.manifest.version}; nothing to publish`);
    return "skipped";
  }

  // A job that stopped early can leave a draft with this tag.
  for (const stale of (await api.listReleases()).filter((r) => r.draft && r.tag_name === tag)) {
    await api.deleteRelease(stale);
  }
  const release = await api.createDraft(tag, args.version);
  for (const path of [args.dmg, args.archive, `${args.archive}.sig`]) await api.uploadAsset(release, path);

  // The uploads take time, so check the manifest again before the release shows.
  current = await api.readManifest();
  if (!isNewer(args.version, current?.manifest.version ?? null)) {
    log(`${current?.manifest.version} was published during the upload; deleting the draft`);
    await api.deleteRelease(release);
    return "skipped";
  }
  await api.publishRelease(release);

  const manifest = buildManifest({
    version: args.version,
    pubDate: args.now().toISOString(),
    url: assetUrl(args.repository, tag, basename(args.archive)),
    signature: args.signature,
  });
  for (;;) {
    if (await api.writeManifest(manifest, current?.sha ?? null)) break;
    current = await api.readManifest();
    if (!isNewer(args.version, current?.manifest.version ?? null)) {
      // The release stays until pruning. No manifest names it, so no user sees it.
      log(`${current?.manifest.version} was written first; the manifest stays`);
      return "skipped";
    }
  }
  log(`published ${args.version}`);

  for (const old of releasesToPrune(await api.listReleases(), tag)) {
    log(`deleting ${old.tag_name}${old.draft ? " (draft)" : ""}`);
    await api.deleteRelease(old);
  }
  return "published";
}

/// The builds repository through the GitHub REST API.
export function githubApi(repository: string, token: string): BuildsApi {
  const headers = {
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const api = `https://api.github.com/repos/${repository}`;
  const call = async (method: string, url: string, body?: unknown, allowed: number[] = []) => {
    const response = await fetch(url, {
      method,
      headers: body === undefined ? headers : { ...headers, "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok && !allowed.includes(response.status)) {
      throw new Error(`${method} ${url}: ${response.status} ${await response.text()}`);
    }
    return response;
  };

  return {
    async readManifest() {
      const response = await call("GET", `${api}/contents/${MANIFEST_PATH}?ref=main`, undefined, [404]);
      if (response.status === 404) return null;
      const file = (await response.json()) as { content: string; sha: string };
      return { manifest: JSON.parse(Buffer.from(file.content, "base64").toString("utf8")), sha: file.sha };
    },
    async writeManifest(manifest, sha) {
      const content = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`).toString("base64");
      const body = { message: `preview: ${manifest.version}`, content, branch: "main", ...(sha ? { sha } : {}) };
      // 409 and 422 mean that the SHA is not the file's current SHA.
      const response = await call("PUT", `${api}/contents/${MANIFEST_PATH}`, body, [409, 422]);
      return response.ok;
    },
    async listReleases() {
      const releases: Release[] = [];
      for (let page = 1; ; page++) {
        const batch = (await (await call("GET", `${api}/releases?per_page=100&page=${page}`)).json()) as Release[];
        releases.push(...batch);
        if (batch.length < 100) return releases;
      }
    },
    async createDraft(tag, version) {
      const body = { tag_name: tag, target_commitish: "main", name: `Longhand ${version}`, draft: true, prerelease: true };
      return (await (await call("POST", `${api}/releases`, body)).json()) as Release;
    },
    async uploadAsset(release, path) {
      const url = `https://uploads.github.com/repos/${repository}/releases/${release.id}/assets?name=${encodeURIComponent(basename(path))}`;
      const response = await fetch(url, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/octet-stream" },
        body: readFileSync(path),
      });
      if (!response.ok) throw new Error(`upload ${basename(path)}: ${response.status} ${await response.text()}`);
    },
    async publishRelease(release) {
      await call("PATCH", `${api}/releases/${release.id}`, { draft: false });
    },
    async deleteRelease(release) {
      await call("DELETE", `${api}/releases/${release.id}`, undefined, [404]);
      // A draft has no tag. A published release keeps its tag after the delete.
      if (!release.draft) await call("DELETE", `${api}/git/refs/tags/${release.tag_name}`, undefined, [404, 422]);
    },
  };
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  if (command === "version" && rest.length === 3) {
    const [configVersion, committerTime, jobStart] = rest as [string, string, string];
    checkCommitTime(committerTime, jobStart);
    console.log(previewVersion(configVersion, committerTime));
  } else if (command === "publish" && rest.length === 4) {
    const [version, dmg, archive, signature] = rest as [string, string, string, string];
    const repository = process.env.BUILDS_REPOSITORY;
    const token = process.env.BUILDS_TOKEN;
    if (!repository || !token) throw new Error("BUILDS_REPOSITORY and BUILDS_TOKEN must be set");
    await publish(githubApi(repository, token), {
      repository,
      version,
      dmg,
      archive,
      signature: readFileSync(signature, "utf8"),
      now: () => new Date(),
    });
  } else {
    console.error("usage: publish-preview.ts version <config version> <committer time> <job start>");
    console.error("       publish-preview.ts publish <version> <dmg> <archive> <signature file>");
    process.exit(2);
  }
}
