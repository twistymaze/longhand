// Publishes one Longhand release for macOS and Windows to the GitHub Releases
// page of the source repository (KTD6).
//
//   node scripts/publish-release.ts publish <version> <dmg> <archive> <archive.sig> <installer> <installer.sig> <notes file> <changelog.json> <CHANGELOG.md>
//     Publishes the release v<version> on the built commit. It uploads these
//     assets to a draft in this order: the DMG, the macOS updater archive, its
//     signature, latest.json, the Windows installer, its signature,
//     latest-windows.json, changelog.json and CHANGELOG.md. Then it publishes
//     the draft as the latest release. The DMG and the installer must have the
//     fixed names Longhand_aarch64.dmg and Longhand_x64-setup.exe, because the
//     site links to these names (R6). The changelog assets always have these
//     names, so the app can find changelog.json from the version alone. The
//     command prints `published=<true|false>` for $GITHUB_OUTPUT. It writes
//     its other lines to stderr, so that they do not go into GITHUB_OUTPUT.
//
// The script reads the repository from GITHUB_REPOSITORY and the token from
// GITHUB_TOKEN. The publish command reads the built commit from GITHUB_SHA.
// The release job runs in the same workflow run as the build, so GITHUB_SHA
// is the built commit.
//
// The publish order is check, draft, upload, read back, publish. A job that
// stops early leaves at most a draft, which no user sees and which has no
// tag. Each check that can fail runs while the release is a draft: the names
// of the installers, and a read-back of the two manifests from the draft.
// No step after the publication can fail the command, so `published=true`
// always means a published release. A version that is not higher than the
// latest release publishes nothing and ends without an error. The
// concurrency group of the CI workflow keeps two publishes from running at
// the same time. The script uses only Node built-ins and scripts/lib/,
// because the publish job runs no package install.
//
// When a newer CI run on main is queued or running, the publish command
// publishes nothing and ends without an error. That run computes its version
// from all commits since the latest release, so it publishes these commits
// too. If a newer commit changed a workflow file, GitHub also refuses to let
// the workflow token create a tag on this older commit (403, "Resource not
// accessible by integration").

import { openAsBlob, readFileSync } from "node:fs";
import { basename } from "node:path";
import { compareVersions } from "./lib/version.ts";

export const MANIFEST_NAME = "latest.json";
export const WINDOWS_MANIFEST_NAME = "latest-windows.json";
export const CHANGELOG_JSON_NAME = "changelog.json";
export const CHANGELOG_MARKDOWN_NAME = "CHANGELOG.md";
/// The fixed installer names that the site links to (R6).
export const DMG_NAME = "Longhand_aarch64.dmg";
export const INSTALLER_NAME = "Longhand_x64-setup.exe";

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

type ManifestArgs = {
  version: string;
  notes: string;
  pubDate: string;
  url: string;
  signature: string;
};

function manifestFor(platform: string, args: ManifestArgs): Manifest {
  return {
    version: args.version,
    notes: args.notes.trim(),
    pub_date: args.pubDate,
    platforms: { [platform]: { signature: args.signature.trim(), url: args.url } },
  };
}

/// The static update manifest that tauri-plugin-updater reads on macOS.
export function buildManifest(args: ManifestArgs): Manifest {
  return manifestFor("darwin-aarch64", args);
}

/// The update manifest that the Windows app reads.
export function buildWindowsManifest(args: ManifestArgs): Manifest {
  return manifestFor("windows-x86_64", args);
}

/// True only when `version` is higher than the latest release's version. No
/// release yet means that any version is higher.
export function isNewer(version: string, latestVersion: string | null): boolean {
  if (latestVersion === null) return true;
  return compareVersions(version, latestVersion) === 1;
}

export type Asset = { id: number; name: string };
/// `body` is the release notes. GitHub sends null when they are empty.
export type Release = { id: number; tag_name: string; draft: boolean; body?: string | null; assets: Asset[] };
export type Draft = { tag: string; target: string; name: string; body: string };

/// The GitHub calls that the publish flow makes. Tests replace them.
export type ReleasesApi = {
  /// The latest published release, or null when there is none.
  latestRelease(): Promise<Release | null>;
  /// All releases, drafts included.
  listReleases(): Promise<Release[]>;
  createDraft(draft: Draft): Promise<Release>;
  uploadAsset(release: Release, name: string, content: Blob | string): Promise<Asset>;
  /// The text of an asset, also of a draft, through the authenticated API.
  readAsset(release: Release, asset: Asset): Promise<string>;
  /// Publishes the draft and marks it as the latest release.
  publishRelease(release: Release): Promise<void>;
  deleteRelease(release: Release): Promise<void>;
  /// True when a newer run of this CI workflow on main is queued or running.
  newerRunWaiting(): Promise<boolean>;
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
    /// The updater signature of the archive.
    signature: string;
    installer: Blob;
    installerName: string;
    /// The updater signature of the installer.
    installerSignature: string;
    notes: string;
    changelogJson: Blob;
    changelogMarkdown: Blob;
    now: () => Date;
  },
  log: (line: string) => void = console.log,
): Promise<PublishResult> {
  // A build change must not publish a name that the site does not link (R6).
  for (const [name, fixed] of [
    [args.dmgName, DMG_NAME],
    [args.installerName, INSTALLER_NAME],
  ]) {
    if (name !== fixed) throw new Error(`the installer ${name} must have the name ${fixed}`);
  }
  const tag = tagFor(args.version);
  const latest = await api.latestRelease();
  const latestVersion = latest ? latest.tag_name.replace(/^v/, "") : null;
  if (!isNewer(args.version, latestVersion)) {
    log(`${args.version} is not higher than ${latestVersion}; nothing to publish`);
    return "skipped";
  }
  if (await api.newerRunWaiting()) {
    log(`a newer CI run on main is queued or running; it publishes ${args.version} or higher`);
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

  const pubDate = args.now().toISOString();
  const text = (manifest: Manifest) => `${JSON.stringify(manifest, null, 2)}\n`;
  const macManifest = text(
    buildManifest({
      version: args.version,
      notes: args.notes,
      pubDate,
      url: assetUrl(args.repository, tag, args.archiveName),
      signature: args.signature,
    }),
  );
  const windowsManifest = text(
    buildWindowsManifest({
      version: args.version,
      notes: args.notes,
      pubDate,
      url: assetUrl(args.repository, tag, args.installerName),
      signature: args.installerSignature,
    }),
  );
  await api.uploadAsset(release, args.dmgName, args.dmg);
  await api.uploadAsset(release, args.archiveName, args.archive);
  await api.uploadAsset(release, `${args.archiveName}.sig`, args.signature);
  const macAsset = await api.uploadAsset(release, MANIFEST_NAME, macManifest);
  await api.uploadAsset(release, args.installerName, args.installer);
  await api.uploadAsset(release, `${args.installerName}.sig`, args.installerSignature);
  const windowsAsset = await api.uploadAsset(release, WINDOWS_MANIFEST_NAME, windowsManifest);
  await api.uploadAsset(release, CHANGELOG_JSON_NAME, args.changelogJson);
  await api.uploadAsset(release, CHANGELOG_MARKDOWN_NAME, args.changelogMarkdown);

  // Installed apps read the manifests. A manifest that GitHub stored with a
  // change stops the publish while the release is still a draft.
  for (const [asset, uploaded] of [
    [macAsset, macManifest],
    [windowsAsset, windowsManifest],
  ] as const) {
    if ((await api.readAsset(release, asset)) !== uploaded) {
      throw new Error(`the ${asset.name} of the draft ${tag} differs from the uploaded manifest`);
    }
  }

  await api.publishRelease(release);
  log(`published ${tag}`);
  return "published";
}

/// The releases of the repository through the GitHub REST API. `runId` is the
/// current workflow run (GITHUB_RUN_ID); without it, no newer run is found.
export function githubApi(repository: string, token: string, runId?: string): ReleasesApi {
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
    async readAsset(_release, asset) {
      // With this Accept header, the API sends the content of the asset, also
      // of a draft, through a redirect to a signed URL. fetch does not send
      // the Authorization header to the other origin of the redirect.
      const url = `${api}/releases/assets/${asset.id}`;
      const response = await fetch(url, { headers: { ...headers, Accept: "application/octet-stream" } });
      if (!response.ok) throw new Error(`GET ${url}: ${response.status} ${await response.text()}`);
      return await response.text();
    },
    async publishRelease(release) {
      // The API takes make_latest as a string.
      await call("PATCH", `${api}/releases/${release.id}`, { draft: false, make_latest: "true" });
    },
    async deleteRelease(release) {
      // The publish flow deletes only drafts, which have no tag.
      await call("DELETE", `${api}/releases/${release.id}`, undefined, [404]);
    },
    async newerRunWaiting() {
      if (!runId) return false;
      type Run = { run_number: number; status: string; workflow_id: number };
      const current = (await (await call("GET", `${api}/actions/runs/${runId}`)).json()) as Run;
      const recent = (await (
        await call("GET", `${api}/actions/workflows/${current.workflow_id}/runs?branch=main&event=push&per_page=20`)
      ).json()) as { workflow_runs: Run[] };
      return recent.workflow_runs.some((r) => r.run_number > current.run_number && r.status !== "completed");
    },
  };
}

const USAGE =
  "usage: publish-release.ts publish <version> <dmg> <archive> <archive.sig> <installer> <installer.sig> <notes file> <changelog.json> <CHANGELOG.md>";

/// Runs the CLI command. A failure rejects, so the process exits with an
/// error. `log` gets the progress lines. `output` gets the `published=` line,
/// for $GITHUB_OUTPUT. It gets `published=true` right after the publication,
/// and no step after it can fail.
export async function main(
  argv: string[],
  env: Record<string, string | undefined>,
  makeApi: (repository: string, token: string, runId?: string) => ReleasesApi = githubApi,
  log: (line: string) => void = console.error,
  output: (line: string) => void = console.log,
): Promise<void> {
  const [command, ...rest] = argv;
  if (command !== "publish" || rest.length !== 9) throw new Error(USAGE);
  const repository = env.GITHUB_REPOSITORY;
  const token = env.GITHUB_TOKEN;
  if (!repository || !token) throw new Error("GITHUB_REPOSITORY and GITHUB_TOKEN must be set");
  const commit = env.GITHUB_SHA;
  if (!commit) throw new Error("GITHUB_SHA must be set to the built commit");

  const [version, dmg, archive, signature, installer, installerSignature, notes, changelogJson, changelogMarkdown] =
    rest as [string, string, string, string, string, string, string, string, string];
  const result = await publish(
    makeApi(repository, token, env.GITHUB_RUN_ID),
    {
      repository,
      version,
      commit,
      dmg: await openAsBlob(dmg),
      dmgName: basename(dmg),
      archive: await openAsBlob(archive),
      archiveName: basename(archive),
      signature: readFileSync(signature, "utf8"),
      installer: await openAsBlob(installer),
      installerName: basename(installer),
      installerSignature: readFileSync(installerSignature, "utf8"),
      notes: readFileSync(notes, "utf8"),
      changelogJson: await openAsBlob(changelogJson),
      changelogMarkdown: await openAsBlob(changelogMarkdown),
      now: () => new Date(),
    },
    log,
  );
  output(`published=${result === "published"}`);
}

if (import.meta.main) {
  await main(process.argv.slice(2), process.env);
}
