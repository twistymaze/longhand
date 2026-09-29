import { expect, test } from "bun:test";
import {
  buildManifest,
  checkCommitTime,
  githubApi,
  isNewer,
  previewVersion,
  publish,
  releasesToPrune,
  tagFor,
  type BuildsApi,
  type Manifest,
  type Release,
} from "./publish-preview";

const REPO = "twistymaze/longhand-builds";

test("the version is the config version with the UTC committer time", () => {
  expect(previewVersion("0.1.0", "2026-09-28T09:30:12-05:00")).toBe("0.1.0-preview.20260928143012");
  expect(tagFor("0.1.0-preview.20260928143012")).toBe("preview-20260928143012");
});

test("a config version that is not X.Y.Z or a time that is not a date fails", () => {
  expect(() => previewVersion("0.1.0-beta", "2026-09-28T09:30:12Z")).toThrow();
  expect(() => previewVersion("0.1.0", "yesterday")).toThrow();
});

test("a later commit time gives a higher version", () => {
  expect(isNewer(previewVersion("0.1.0", "2026-09-28T14:30:13Z"), previewVersion("0.1.0", "2026-09-28T14:30:12Z"))).toBe(true);
  expect(isNewer("0.2.0", "0.2.0-preview.20260928143012")).toBe(true);
});

test("a commit time more than 10 minutes after the job start fails", () => {
  expect(() => checkCommitTime("2026-09-28T14:40:01Z", "2026-09-28T14:30:00Z")).toThrow(/10 minutes/);
  expect(() => checkCommitTime("2026-09-28T14:40:00Z", "2026-09-28T14:30:00Z")).not.toThrow();
  expect(() => checkCommitTime("2026-09-01T00:00:00Z", "2026-09-28T14:30:00Z")).not.toThrow();
});

test("the manifest names the version, the date, and the darwin-aarch64 archive and signature", () => {
  expect(
    buildManifest({
      version: "0.1.0-preview.20260928143012",
      pubDate: "2026-09-28T15:00:00.000Z",
      url: "https://example.test/Longhand.app.tar.gz",
      signature: "c2lnbmF0dXJl\n",
    }),
  ).toEqual({
    version: "0.1.0-preview.20260928143012",
    notes: "Preview build 0.1.0-preview.20260928143012",
    pub_date: "2026-09-28T15:00:00.000Z",
    platforms: { "darwin-aarch64": { signature: "c2lnbmF0dXJl", url: "https://example.test/Longhand.app.tar.gz" } },
  });
});

test("an equal or lower version is not newer, and no manifest means newer", () => {
  expect(isNewer("0.1.0-preview.20260928143012", "0.1.0-preview.20260928143012")).toBe(false);
  expect(isNewer("0.1.0-preview.20260928143011", "0.1.0-preview.20260928143012")).toBe(false);
  expect(isNewer("0.1.0-preview.20260928143012", null)).toBe(true);
});

test("pruning keeps the 20 newest preview releases and never selects another release", () => {
  const published: Release[] = Array.from({ length: 23 }, (_, i) => ({
    id: i,
    tag_name: `preview-202609281430${String(i).padStart(2, "0")}`,
    draft: false,
  }));
  const other: Release[] = [
    { id: 100, tag_name: "v0.1.0", draft: false },
    { id: 101, tag_name: "stable-1", draft: true },
  ];
  const drafts: Release[] = [
    { id: 200, tag_name: "preview-20260928143099", draft: true },
    { id: 201, tag_name: "preview-20260928143098", draft: true },
  ];
  const pruned = releasesToPrune([...other, ...published.slice().reverse(), ...drafts], "preview-20260928143099");
  expect(pruned.map((r) => r.id).sort((a, b) => a - b)).toEqual([0, 1, 2, 201]);
});

/// A builds repository in memory.
function fakeRepo(manifestVersion: string | null) {
  let nextId = 1;
  const state = {
    manifest: manifestVersion === null ? null : ({ version: manifestVersion } as Manifest),
    sha: manifestVersion === null ? null : "sha-0",
    releases: [] as (Release & { assets: string[] })[],
    writes: [] as Manifest[],
    // Runs before the check that follows the uploads, as a later job would.
    onUploaded: () => {},
    // Runs before each manifest write.
    beforeWrite: () => {},
  };
  const api: BuildsApi = {
    async readManifest() {
      return state.manifest && state.sha ? { manifest: state.manifest, sha: state.sha } : null;
    },
    async writeManifest(manifest, sha) {
      state.beforeWrite();
      if (sha !== state.sha) return false;
      state.manifest = manifest;
      state.sha = `sha-${state.writes.push(manifest)}`;
      return true;
    },
    async listReleases() {
      return state.releases.map(({ id, tag_name, draft }) => ({ id, tag_name, draft }));
    },
    async createDraft(tag) {
      const release = { id: nextId++, tag_name: tag, draft: true, assets: [] };
      state.releases.push(release);
      return release;
    },
    async uploadAsset(release, path) {
      const found = state.releases.find((r) => r.id === release.id)!;
      found.assets.push(path);
      if (found.assets.length === 3) state.onUploaded();
    },
    async publishRelease(release) {
      state.releases.find((r) => r.id === release.id)!.draft = false;
    },
    async deleteRelease(release) {
      state.releases = state.releases.filter((r) => r.id !== release.id);
    },
  };
  return { api, state };
}

const args = (version: string) => ({
  repository: REPO,
  version,
  dmg: "/b/Longhand.dmg",
  archive: "/b/Longhand.app.tar.gz",
  signature: "sig",
  now: () => new Date("2026-09-28T15:00:00Z"),
});
const quiet = () => {};

test("a newer build publishes its release and writes the manifest", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("published");
  expect(state.releases).toEqual([
    {
      id: 1,
      tag_name: "preview-20260928143012",
      draft: false,
      assets: ["/b/Longhand.dmg", "/b/Longhand.app.tar.gz", "/b/Longhand.app.tar.gz.sig"],
    },
  ]);
  expect(state.manifest?.version).toBe("0.1.0-preview.20260928143012");
  expect(state.manifest?.platforms["darwin-aarch64"]?.url).toBe(
    `https://github.com/${REPO}/releases/download/preview-20260928143012/Longhand.app.tar.gz`,
  );
});

test("the first build publishes when no manifest exists", async () => {
  const { api, state } = fakeRepo(null);
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("published");
  expect(state.writes).toHaveLength(1);
});

test("a re-run of the build that the manifest names publishes nothing", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928143012");
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("skipped");
  expect(state.releases).toEqual([]);
  expect(state.writes).toEqual([]);
});

test("an older build that finishes after a newer one publishes nothing", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928150000");
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("skipped");
  expect(state.releases).toEqual([]);
  expect(state.manifest?.version).toBe("0.1.0-preview.20260928150000");
});

test("a newer manifest written during the upload deletes the draft", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  state.onUploaded = () => {
    state.manifest = { version: "0.1.0-preview.20260928150000" } as Manifest;
    state.sha = "sha-other";
  };
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("skipped");
  expect(state.releases).toEqual([]);
  expect(state.manifest?.version).toBe("0.1.0-preview.20260928150000");
});

test("a lost manifest write reads again, and a newer manifest stays", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  let first = true;
  state.beforeWrite = () => {
    if (!first) return;
    first = false;
    state.manifest = { version: "0.1.0-preview.20260928150000" } as Manifest;
    state.sha = "sha-other";
  };
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("skipped");
  expect(state.manifest?.version).toBe("0.1.0-preview.20260928150000");
});

test("a lost manifest write reads again, and an older manifest is replaced", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  let first = true;
  state.beforeWrite = () => {
    if (!first) return;
    first = false;
    state.manifest = { version: "0.1.0-preview.20260928130000" } as Manifest;
    state.sha = "sha-other";
  };
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("published");
  expect(state.manifest?.version).toBe("0.1.0-preview.20260928143012");
});

test("a stale draft with the same tag is deleted before the new draft", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  await api.createDraft("preview-20260928143012", "0.1.0-preview.20260928143012");
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("published");
  expect(state.releases.map((r) => r.id)).toEqual([2]);
});

test("a publish deletes the published preview releases after the 20 newest", async () => {
  const { api, state } = fakeRepo("0.1.0-preview.20260928120000");
  for (let i = 0; i < 20; i++) {
    const release = await api.createDraft(`preview-202609281100${String(i).padStart(2, "0")}`, "");
    await api.publishRelease(release);
  }
  expect(await publish(api, args("0.1.0-preview.20260928143012"), quiet)).toBe("published");
  expect(state.releases).toHaveLength(20);
  expect(state.releases.some((r) => r.tag_name === "preview-20260928110000")).toBe(false);
  expect(state.releases.some((r) => r.tag_name === "preview-20260928143012")).toBe(true);
});

test("a failed delete after the publish does not fail the publish", async () => {
  const { api } = fakeRepo("0.1.0-preview.20260928120000");
  for (let i = 0; i < 21; i++) {
    const release = await api.createDraft(`preview-202609281100${String(i).padStart(2, "0")}`, "");
    await api.publishRelease(release);
  }
  api.deleteRelease = async () => {
    throw new Error("503");
  };
  const lines: string[] = [];
  expect(await publish(api, args("0.1.0-preview.20260928143012"), (l) => lines.push(l))).toBe("published");
  expect(lines.some((l) => l.startsWith("could not delete preview-20260928110000"))).toBe(true);
});

/// Replaces fetch with `respond` while `run` runs, and records each request.
async function withFetch(respond: (url: string) => Response, run: () => Promise<void>): Promise<string[]> {
  const requests: string[] = [];
  const real = globalThis.fetch;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    requests.push(`${init?.method ?? "GET"} ${url}`);
    return respond(url);
  }) as typeof fetch;
  try {
    await run();
  } finally {
    globalThis.fetch = real;
  }
  return requests;
}

const API = `https://api.github.com/repos/${REPO}`;

test("the GitHub binding reads the manifest, and a missing manifest is null", async () => {
  const content = Buffer.from(JSON.stringify({ version: "0.1.0-preview.20260928143012" })).toString("base64");
  await withFetch(
    () => Response.json({ content, sha: "abc" }),
    async () => {
      expect(await githubApi(REPO, "t").readManifest()).toEqual({
        manifest: { version: "0.1.0-preview.20260928143012" } as Manifest,
        sha: "abc",
      });
    },
  );
  await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      expect(await githubApi(REPO, "t").readManifest()).toBeNull();
    },
  );
});

test("the GitHub binding reports a SHA conflict as a lost write and fails on other errors", async () => {
  const manifest = { version: "0.1.0-preview.20260928143012" } as Manifest;
  for (const status of [409, 422]) {
    await withFetch(
      () => new Response("conflict", { status }),
      async () => {
        expect(await githubApi(REPO, "t").writeManifest(manifest, "abc")).toBe(false);
      },
    );
  }
  await withFetch(
    () => new Response("server error", { status: 500 }),
    async () => {
      await expect(githubApi(REPO, "t").writeManifest(manifest, "abc")).rejects.toThrow(/500/);
    },
  );
});

test("the GitHub binding reads every page of releases", async () => {
  const page = (n: number) => Array.from({ length: n }, (_, i) => ({ id: i, tag_name: `preview-${i}`, draft: false }));
  const requests = await withFetch(
    (url) => Response.json(url.endsWith("page=1") ? page(100) : page(3)),
    async () => {
      expect(await githubApi(REPO, "t").listReleases()).toHaveLength(103);
    },
  );
  expect(requests).toEqual([`GET ${API}/releases?per_page=100&page=1`, `GET ${API}/releases?per_page=100&page=2`]);
});

test("the GitHub binding deletes the tag of a published release only", async () => {
  const requests = await withFetch(
    () => new Response(null, { status: 204 }),
    async () => {
      const api = githubApi(REPO, "t");
      await api.deleteRelease({ id: 7, tag_name: "preview-7", draft: true });
      await api.deleteRelease({ id: 8, tag_name: "preview-8", draft: false });
    },
  );
  expect(requests).toEqual([
    `DELETE ${API}/releases/7`,
    `DELETE ${API}/releases/8`,
    `DELETE ${API}/git/refs/tags/preview-8`,
  ]);
});
