// Publishes one Longhand release to the GitHub Releases page of the source
// repository, and attaches the Windows installer to it later (KTD4).
//
//   bun scripts/publish-release.ts publish <version> <dmg> <archive> <signature> <notes file>
//     Publishes the release v<version> on the built commit, with the DMG, the
//     updater archive, its signature and latest.json as assets.
//   bun scripts/publish-release.ts attach <tag> <installer>
//     Uploads the installer to the release with the tag. It replaces an asset
//     with the same name, so a re-run works. The old asset stays until the new
//     one is uploaded, so a failed re-run does not remove the installer.
//
// The script reads the repository from GITHUB_REPOSITORY and the token from
// GITHUB_TOKEN. The publish command reads the built commit from GITHUB_SHA.
// The release job runs in the same workflow run as the build, so GITHUB_SHA
// is the built commit.
//
// The publish order is check, draft, upload, publish. A job that stops early
// leaves at most a draft, which no user sees and which has no tag. A version
// that is not higher than the latest release publishes nothing and ends
// without an error. The concurrency group of the CI workflow keeps two
// publishes from running at the same time. The script uses only Bun built-ins, because the
// publish job runs no `bun install`.

import { basename } from "node:path";

export const MANIFEST_NAME = "latest.json";

export function tagFor(version: string): string {
  return `v${version}`;
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
export function buildManifest(args: {
  version: string;
  notes: string;
  pubDate: string;
  url: string;
  signature: string;
}): Manifest {
  return {
    version: args.version,
    notes: args.notes.trim(),
    pub_date: args.pubDate,
    platforms: { "darwin-aarch64": { signature: args.signature.trim(), url: args.url } },
  };
}

/// True only when `version` is higher than the latest release's version. No
/// release yet means that any version is higher.
export function isNewer(version: string, latestVersion: string | null): boolean {
  if (latestVersion === null) return true;
  return Bun.semver.order(version, latestVersion) === 1;
}

export type Asset = { id: number; name: string };
export type Release = { id: number; tag_name: string; draft: boolean; assets: Asset[] };
export type Draft = { tag: string; target: string; name: string; body: string };

/// The GitHub calls that the publish and attach flows make. Tests replace them.
export type ReleasesApi = {
  /// The latest published release, or null when there is none.
  latestRelease(): Promise<Release | null>;
  /// The published release with the tag, or null when there is none.
  releaseByTag(tag: string): Promise<Release | null>;
  /// All releases, drafts included.
  listReleases(): Promise<Release[]>;
  createDraft(draft: Draft): Promise<Release>;
  uploadAsset(release: Release, name: string, content: Blob | string): Promise<Asset>;
  deleteAsset(release: Release, asset: Asset): Promise<void>;
  renameAsset(release: Release, asset: Asset, name: string): Promise<void>;
  /// Publishes the draft and marks it as the latest release.
  publishRelease(release: Release): Promise<void>;
  deleteRelease(release: Release): Promise<void>;
};

export type PublishResult = "published" | "skipped";

export async function publish(
  api: ReleasesApi,
  args: {
    repository: string;
    version: string;
    commit: string;
    dmg: Blob;
    dmgName: string;
    archive: Blob;
    archiveName: string;
    signature: string;
    notes: string;
    now: () => Date;
  },
  log: (line: string) => void = console.log,
): Promise<PublishResult> {
  const tag = tagFor(args.version);
  const latest = await api.latestRelease();
  const latestVersion = latest ? latest.tag_name.replace(/^v/, "") : null;
  if (!isNewer(args.version, latestVersion)) {
    log(`${args.version} is not higher than ${latestVersion}; nothing to publish`);
    return "skipped";
  }

  // A job that stopped early can leave a draft with this tag.
  for (const stale of (await api.listReleases()).filter((r) => r.draft && r.tag_name === tag)) {
    log(`deleting the draft ${stale.tag_name} of an earlier job`);
    await api.deleteRelease(stale);
  }
  const release = await api.createDraft({
    tag,
    target: args.commit,
    name: `Longhand ${args.version}`,
    body: args.notes,
  });

  const manifest = buildManifest({
    version: args.version,
    notes: args.notes,
    pubDate: args.now().toISOString(),
    url: assetUrl(args.repository, tag, args.archiveName),
    signature: args.signature,
  });
  await api.uploadAsset(release, args.dmgName, args.dmg);
  await api.uploadAsset(release, args.archiveName, args.archive);
  await api.uploadAsset(release, `${args.archiveName}.sig`, args.signature);
  await api.uploadAsset(release, MANIFEST_NAME, `${JSON.stringify(manifest, null, 2)}\n`);

  await api.publishRelease(release);
  log(`published ${tag}`);
  return "published";
}

export async function attach(
  api: ReleasesApi,
  args: { tag: string; installer: Blob; installerName: string },
  log: (line: string) => void = console.log,
): Promise<void> {
  const release = await api.releaseByTag(args.tag);
  if (!release) throw new Error(`no release has the tag ${args.tag}`);
  const olds = release.assets.filter((a) => a.name === args.installerName);
  if (olds.length === 0) {
    await api.uploadAsset(release, args.installerName, args.installer);
  } else {
    // Upload under a temporary name first. Only a successful upload replaces
    // the earlier installer.
    const upload = await api.uploadAsset(release, `${args.installerName}.new`, args.installer);
    for (const old of olds) {
      log(`deleting the earlier ${old.name}`);
      await api.deleteAsset(release, old);
    }
    await api.renameAsset(release, upload, args.installerName);
  }
  log(`attached ${args.installerName} to ${args.tag}`);
}

/// The releases of the repository through the GitHub REST API.
export function githubApi(repository: string, token: string): ReleasesApi {
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
  const releaseOrNull = async (url: string) => {
    const response = await call("GET", url, undefined, [404]);
    return response.status === 404 ? null : ((await response.json()) as Release);
  };

  return {
    latestRelease() {
      return releaseOrNull(`${api}/releases/latest`);
    },
    releaseByTag(tag) {
      return releaseOrNull(`${api}/releases/tags/${encodeURIComponent(tag)}`);
    },
    async listReleases() {
      const releases: Release[] = [];
      for (let page = 1; ; page++) {
        const batch = (await (await call("GET", `${api}/releases?per_page=100&page=${page}`)).json()) as Release[];
        releases.push(...batch);
        if (batch.length < 100) return releases;
      }
    },
    async createDraft(draft) {
      const body = {
        tag_name: draft.tag,
        target_commitish: draft.target,
        name: draft.name,
        body: draft.body,
        draft: true,
        prerelease: false,
      };
      return (await (await call("POST", `${api}/releases`, body)).json()) as Release;
    },
    async uploadAsset(release, name, content) {
      const url = `https://uploads.github.com/repos/${repository}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`;
      const response = await fetch(url, {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/octet-stream" },
        body: content,
      });
      if (!response.ok) throw new Error(`upload ${name}: ${response.status} ${await response.text()}`);
      return (await response.json()) as Asset;
    },
    async deleteAsset(_release, asset) {
      await call("DELETE", `${api}/releases/assets/${asset.id}`, undefined, [404]);
    },
    async renameAsset(_release, asset, name) {
      await call("PATCH", `${api}/releases/assets/${asset.id}`, { name });
    },
    async publishRelease(release) {
      // The API takes make_latest as a string.
      await call("PATCH", `${api}/releases/${release.id}`, { draft: false, make_latest: "true" });
    },
    async deleteRelease(release) {
      // The publish flow deletes only drafts, which have no tag.
      await call("DELETE", `${api}/releases/${release.id}`, undefined, [404]);
    },
  };
}

const USAGE = [
  "usage: publish-release.ts publish <version> <dmg> <archive> <signature file> <notes file>",
  "       publish-release.ts attach <tag> <installer>",
].join("\n");

/// Runs one CLI command. A failure rejects, so the process exits with an error.
export async function main(
  argv: string[],
  env: Record<string, string | undefined>,
  makeApi: (repository: string, token: string) => ReleasesApi = githubApi,
  log: (line: string) => void = console.log,
): Promise<void> {
  const [command, ...rest] = argv;
  if (!((command === "publish" && rest.length === 5) || (command === "attach" && rest.length === 2))) {
    throw new Error(USAGE);
  }
  const repository = env.GITHUB_REPOSITORY;
  const token = env.GITHUB_TOKEN;
  if (!repository || !token) throw new Error("GITHUB_REPOSITORY and GITHUB_TOKEN must be set");

  if (command === "publish") {
    const [version, dmg, archive, signature, notes] = rest as [string, string, string, string, string];
    const commit = env.GITHUB_SHA;
    if (!commit) throw new Error("GITHUB_SHA must be set to the built commit");
    await publish(
      makeApi(repository, token),
      {
        repository,
        version,
        commit,
        dmg: Bun.file(dmg),
        dmgName: basename(dmg),
        archive: Bun.file(archive),
        archiveName: basename(archive),
        signature: await Bun.file(signature).text(),
        notes: await Bun.file(notes).text(),
        now: () => new Date(),
      },
      log,
    );
  } else {
    const [tag, installer] = rest as [string, string];
    await attach(makeApi(repository, token), { tag, installer: Bun.file(installer), installerName: basename(installer) }, log);
  }
}

if (import.meta.main) {
  await main(process.argv.slice(2), process.env);
}
