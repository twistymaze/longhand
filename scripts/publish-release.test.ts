import { expect, test } from "bun:test";
import {
  attach,
  buildManifest,
  githubApi,
  isNewer,
  main,
  publish,
  type Asset,
  type Manifest,
  type Release,
  type ReleasesApi,
} from "./publish-release";

const REPO = "twistymaze/longhand";
const SHA = "0123456789abcdef0123456789abcdef01234567";

test("the manifest names the version, the notes, the date, and the darwin-aarch64 archive and signature", () => {
  expect(
    buildManifest({
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix\n",
      pubDate: "2026-10-11T15:00:00.000Z",
      url: "https://example.test/Longhand.app.tar.gz",
      signature: "c2lnbmF0dXJl\n",
    }),
  ).toEqual({
    version: "1.0.4",
    notes: "### Bug Fixes\n- one fix",
    pub_date: "2026-10-11T15:00:00.000Z",
    platforms: { "darwin-aarch64": { signature: "c2lnbmF0dXJl", url: "https://example.test/Longhand.app.tar.gz" } },
  });
});

test("only a higher version is newer, and no release means newer", () => {
  expect(isNewer("1.0.1", "1.0.0")).toBe(true);
  expect(isNewer("1.1.0", "1.0.9")).toBe(true);
  expect(isNewer("1.0.0", "1.0.0")).toBe(false);
  expect(isNewer("1.0.0", "1.0.1")).toBe(false);
  expect(isNewer("1.0.0", null)).toBe(true);
});

type FakeRelease = Release & {
  target_commitish: string;
  name: string;
  body: string;
  prerelease: boolean;
  latest: boolean;
  files: Record<string, string>;
};

/// A releases page in memory.
function fakeRepo() {
  let nextId = 1;
  const state = {
    releases: [] as FakeRelease[],
    // Runs before each upload. A test can make it throw.
    beforeUpload: (_name: string) => {},
    // True when a newer CI run on main is queued or running.
    newerRun: false,
  };
  const view = (r: FakeRelease): Release => ({
    id: r.id,
    tag_name: r.tag_name,
    draft: r.draft,
    assets: r.assets.map((a) => ({ ...a })),
  });
  const find = (id: number) => state.releases.find((r) => r.id === id)!;
  const api: ReleasesApi = {
    async latestRelease() {
      const latest = state.releases.find((r) => r.latest);
      return latest ? view(latest) : null;
    },
    async releaseByTag(tag) {
      const found = state.releases.find((r) => !r.draft && r.tag_name === tag);
      return found ? view(found) : null;
    },
    async listReleases() {
      return state.releases.map(view);
    },
    async createDraft(draft) {
      const release: FakeRelease = {
        id: nextId++,
        tag_name: draft.tag,
        target_commitish: draft.target,
        name: draft.name,
        body: draft.body,
        prerelease: false,
        draft: true,
        latest: false,
        assets: [],
        files: {},
      };
      state.releases.push(release);
      return view(release);
    },
    async uploadAsset(release, name, content) {
      state.beforeUpload(name);
      const found = find(release.id);
      const asset = { id: nextId++, name };
      found.assets.push(asset);
      found.files[name] = typeof content === "string" ? content : await content.text();
      return { ...asset };
    },
    async deleteAsset(release, asset) {
      const found = find(release.id);
      found.assets = found.assets.filter((a) => a.id !== asset.id);
      delete found.files[asset.name];
    },
    async renameAsset(release, asset, name) {
      const found = find(release.id);
      const stored = found.assets.find((a) => a.id === asset.id)!;
      found.files[name] = found.files[stored.name]!;
      delete found.files[stored.name];
      stored.name = name;
    },
    async publishRelease(release) {
      for (const r of state.releases) r.latest = false;
      Object.assign(find(release.id), { draft: false, latest: true });
    },
    async deleteRelease(release) {
      state.releases = state.releases.filter((r) => r.id !== release.id);
    },
    async newerRunWaiting() {
      return state.newerRun;
    },
  };
  return { api, state };
}

/// Adds a published release, as an earlier run would.
async function seedRelease(api: ReleasesApi, version: string) {
  const release = await api.createDraft({ tag: `v${version}`, target: SHA, name: `Longhand ${version}`, body: "" });
  await api.publishRelease(release);
}

const args = (version: string) => ({
  repository: REPO,
  version,
  commit: SHA,
  dmg: new Blob(["dmg"]),
  dmgName: "Longhand_1.0.0_aarch64.dmg",
  archive: new Blob(["archive"]),
  archiveName: "Longhand.app.tar.gz",
  signature: "c2ln\n",
  notes: "### Features\n- one feature\n",
  now: () => new Date("2026-10-11T15:00:00Z"),
});
const quiet = () => {};

test("with no release yet, publish creates v1.0.0 with the 4 assets on the built commit as the latest release", async () => {
  const { api, state } = fakeRepo();
  expect(await publish(api, args("1.0.0"), quiet)).toBe("published");
  expect(state.releases).toHaveLength(1);
  const release = state.releases[0]!;
  expect(release).toMatchObject({
    tag_name: "v1.0.0",
    target_commitish: SHA,
    name: "Longhand 1.0.0",
    body: "### Features\n- one feature\n",
    prerelease: false,
    draft: false,
    latest: true,
  });
  expect(release.assets.map((a) => a.name)).toEqual([
    "Longhand_1.0.0_aarch64.dmg",
    "Longhand.app.tar.gz",
    "Longhand.app.tar.gz.sig",
    "latest.json",
  ]);
  expect(release.files["Longhand.app.tar.gz.sig"]).toBe("c2ln\n");
});

test("latest.json names the version, the trimmed signature and the archive of its own release", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.3");
  expect(await publish(api, args("1.0.4"), quiet)).toBe("published");
  const release = state.releases.find((r) => r.tag_name === "v1.0.4")!;
  const manifest = JSON.parse(release.files["latest.json"]!) as Manifest;
  expect(manifest.version).toBe("1.0.4");
  expect(manifest.platforms["darwin-aarch64"]).toEqual({
    signature: "c2ln",
    url: "https://github.com/twistymaze/longhand/releases/download/v1.0.4/Longhand.app.tar.gz",
  });
});

test("a re-run whose version the latest release already has publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.4");
  expect(await publish(api, args("1.0.4"), quiet)).toBe("skipped");
  expect(state.releases.map((r) => r.tag_name)).toEqual(["v1.0.4"]);
});

test("an older run that finishes after a newer one publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.1.0");
  expect(await publish(api, args("1.0.4"), quiet)).toBe("skipped");
  expect(state.releases.map((r) => r.tag_name)).toEqual(["v1.1.0"]);
});

test("a run that a newer CI run on main follows publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  state.newerRun = true;
  expect(await publish(api, args("1.0.1"), quiet)).toBe("skipped");
  expect(state.releases.map((r) => r.tag_name)).toEqual(["v1.0.0"]);
});

test("a stale draft with the same tag is deleted before the new draft", async () => {
  const { api, state } = fakeRepo();
  await api.createDraft({ tag: "v1.0.0", target: SHA, name: "Longhand 1.0.0", body: "" });
  expect(await publish(api, args("1.0.0"), quiet)).toBe("published");
  expect(state.releases.map((r) => [r.id, r.draft])).toEqual([[2, false]]);
});

test("a failed upload leaves only a draft and fails the publish", async () => {
  const { api, state } = fakeRepo();
  state.beforeUpload = (name) => {
    if (name === "Longhand.app.tar.gz") throw new Error("upload Longhand.app.tar.gz: 502");
  };
  await expect(publish(api, args("1.0.0"), quiet)).rejects.toThrow(/502/);
  expect(state.releases.map((r) => [r.tag_name, r.draft])).toEqual([["v1.0.0", true]]);
});

test("attach uploads the installer to the release with the tag", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  await attach(api, { tag: "v1.0.0", installer: new Blob(["exe"]), installerName: "Longhand_1.0.0_x64-setup.exe" }, quiet);
  expect(state.releases[0]!.files).toEqual({ "Longhand_1.0.0_x64-setup.exe": "exe" });
});

test("attach replaces an asset with the same name", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  const installer = { tag: "v1.0.0", installerName: "Longhand_1.0.0_x64-setup.exe" };
  await attach(api, { ...installer, installer: new Blob(["first"]) }, quiet);
  await attach(api, { ...installer, installer: new Blob(["second"]) }, quiet);
  expect(state.releases[0]!.assets.map((a) => a.name)).toEqual(["Longhand_1.0.0_x64-setup.exe"]);
  expect(state.releases[0]!.files["Longhand_1.0.0_x64-setup.exe"]).toBe("second");
});

test("a failed re-upload keeps the earlier installer", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  const installer = { tag: "v1.0.0", installerName: "Longhand_1.0.0_x64-setup.exe" };
  await attach(api, { ...installer, installer: new Blob(["first"]) }, quiet);
  state.beforeUpload = () => {
    throw new Error("upload failed");
  };
  await expect(attach(api, { ...installer, installer: new Blob(["second"]) }, quiet)).rejects.toThrow("upload failed");
  expect(state.releases[0]!.files["Longhand_1.0.0_x64-setup.exe"]).toBe("first");
});

test("attach fails when no release has the tag", async () => {
  const { api } = fakeRepo();
  await expect(
    attach(api, { tag: "v1.0.0", installer: new Blob(["exe"]), installerName: "setup.exe" }, quiet),
  ).rejects.toThrow("no release has the tag v1.0.0");
});

test("the CLI reads the repository, the token and the built commit from the environment", async () => {
  const made: string[] = [];
  const { api, state } = fakeRepo();
  const dir = `${process.env.TMPDIR ?? "/tmp"}/publish-release-test-${process.pid}`;
  await Bun.write(`${dir}/Longhand_1.0.0_aarch64.dmg`, "dmg");
  await Bun.write(`${dir}/Longhand.app.tar.gz`, "archive");
  await Bun.write(`${dir}/Longhand.app.tar.gz.sig`, "c2ln\n");
  await Bun.write(`${dir}/notes.md`, "notes\n");
  const env = { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t", GITHUB_SHA: SHA };
  const makeApi = (repository: string, token: string) => {
    made.push(`${repository} ${token}`);
    return api;
  };
  const argv = [
    "publish",
    "1.0.0",
    `${dir}/Longhand_1.0.0_aarch64.dmg`,
    `${dir}/Longhand.app.tar.gz`,
    `${dir}/Longhand.app.tar.gz.sig`,
    `${dir}/notes.md`,
  ];
  await main(argv, env, makeApi, quiet);
  expect(made).toEqual([`${REPO} t`]);
  expect(state.releases[0]).toMatchObject({ tag_name: "v1.0.0", target_commitish: SHA, body: "notes\n", draft: false });
  expect(state.releases[0]!.files["Longhand_1.0.0_aarch64.dmg"]).toBe("dmg");
});

test("a CLI publish with a failed upload rejects, so the process exits with an error", async () => {
  const { api, state } = fakeRepo();
  state.beforeUpload = () => {
    throw new Error("upload: 502");
  };
  const dir = `${process.env.TMPDIR ?? "/tmp"}/publish-release-test-${process.pid}`;
  await Bun.write(`${dir}/a.dmg`, "dmg");
  await Bun.write(`${dir}/a.tar.gz`, "archive");
  await Bun.write(`${dir}/a.tar.gz.sig`, "sig");
  await Bun.write(`${dir}/notes.md`, "notes");
  const env = { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t", GITHUB_SHA: SHA };
  const argv = ["publish", "1.0.0", `${dir}/a.dmg`, `${dir}/a.tar.gz`, `${dir}/a.tar.gz.sig`, `${dir}/notes.md`];
  await expect(main(argv, env, () => api, quiet)).rejects.toThrow(/502/);
});

test("the CLI fails when the environment does not name the repository, the token and the commit", async () => {
  const { api } = fakeRepo();
  await expect(main(["attach", "v1.0.0", "/b/setup.exe"], {}, () => api, quiet)).rejects.toThrow(/GITHUB_REPOSITORY/);
  await expect(
    main(["publish", "1.0.0", "a", "b", "c", "d"], { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t" }, () => api, quiet),
  ).rejects.toThrow(/GITHUB_SHA/);
});

/// Replaces fetch with `respond` while `run` runs, and records each request.
async function withFetch(
  respond: (url: string, init?: RequestInit) => Response,
  run: () => Promise<void>,
): Promise<{ line: string; init?: RequestInit }[]> {
  const requests: { line: string; init?: RequestInit }[] = [];
  const real = globalThis.fetch;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    requests.push({ line: `${init?.method ?? "GET"} ${url}`, init });
    return respond(url, init);
  }) as typeof fetch;
  try {
    await run();
  } finally {
    globalThis.fetch = real;
  }
  return requests;
}

const API = `https://api.github.com/repos/${REPO}`;

test("the GitHub binding treats a 404 from the latest release as no release and sends the API version header", async () => {
  const requests = await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      expect(await githubApi(REPO, "t").latestRelease()).toBeNull();
    },
  );
  expect(requests.map((r) => r.line)).toEqual([`GET ${API}/releases/latest`]);
  const headers = requests[0]!.init!.headers as Record<string, string>;
  expect(headers["X-GitHub-Api-Version"]).toBe("2022-11-28");
  expect(headers.Authorization).toBe("Bearer t");
});

test("the GitHub binding finds a newer queued or running CI run on main", async () => {
  const check = async (current: number, others: { run_number: number; status: string }[]) => {
    let result = false;
    const requests = await withFetch(
      (url) =>
        url.endsWith("/actions/runs/77")
          ? Response.json({ run_number: current, status: "in_progress", workflow_id: 5 })
          : Response.json({ workflow_runs: others.map((r) => ({ ...r, workflow_id: 5 })) }),
      async () => {
        result = await githubApi(REPO, "t", "77").newerRunWaiting();
      },
    );
    return { result, lines: requests.map((r) => r.line) };
  };
  const queued = await check(10, [{ run_number: 11, status: "queued" }, { run_number: 10, status: "in_progress" }]);
  expect(queued.result).toBe(true);
  expect(queued.lines).toEqual([
    `GET ${API}/actions/runs/77`,
    `GET ${API}/actions/workflows/5/runs?branch=main&event=push&per_page=20`,
  ]);
  expect((await check(10, [{ run_number: 11, status: "completed" }, { run_number: 9, status: "queued" }])).result).toBe(false);
  expect(await githubApi(REPO, "t").newerRunWaiting()).toBe(false);
});

test("the GitHub binding treats a 404 from a tag as no release and fails on other errors", async () => {
  await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      expect(await githubApi(REPO, "t").releaseByTag("v1.0.0")).toBeNull();
    },
  );
  await withFetch(
    () => new Response("server error", { status: 500 }),
    async () => {
      await expect(githubApi(REPO, "t").latestRelease()).rejects.toThrow(/500/);
    },
  );
});

test("the GitHub binding creates a draft on the built commit that is not a pre-release", async () => {
  const requests = await withFetch(
    () => Response.json({ id: 9, tag_name: "v1.0.0", draft: true, assets: [] }),
    async () => {
      await githubApi(REPO, "t").createDraft({ tag: "v1.0.0", target: SHA, name: "Longhand 1.0.0", body: "notes" });
    },
  );
  expect(requests.map((r) => r.line)).toEqual([`POST ${API}/releases`]);
  expect(JSON.parse(String(requests[0]!.init!.body))).toEqual({
    tag_name: "v1.0.0",
    target_commitish: SHA,
    name: "Longhand 1.0.0",
    body: "notes",
    draft: true,
    prerelease: false,
  });
});

test("the GitHub binding publishes a release as the latest release", async () => {
  const requests = await withFetch(
    () => Response.json({}),
    async () => {
      await githubApi(REPO, "t").publishRelease({ id: 9, tag_name: "v1.0.0", draft: true, assets: [] });
    },
  );
  expect(requests.map((r) => r.line)).toEqual([`PATCH ${API}/releases/9`]);
  expect(JSON.parse(String(requests[0]!.init!.body))).toEqual({ draft: false, make_latest: "true" });
});

test("the GitHub binding uploads, deletes and renames an asset", async () => {
  const asset: Asset = { id: 5, name: "latest.json" };
  const requests = await withFetch(
    () => Response.json({ id: 7, name: "latest.json" }, { status: 201 }),
    async () => {
      const api = githubApi(REPO, "t");
      const release = { id: 9, tag_name: "v1.0.0", draft: true, assets: [] };
      expect(await api.uploadAsset(release, "latest.json", "{}")).toEqual({ id: 7, name: "latest.json" });
      await api.deleteAsset(release, asset);
      await api.renameAsset(release, asset, "setup.exe");
    },
  );
  expect(requests.map((r) => r.line)).toEqual([
    `POST https://uploads.github.com/repos/${REPO}/releases/9/assets?name=latest.json`,
    `DELETE ${API}/releases/assets/5`,
    `PATCH ${API}/releases/assets/5`,
  ]);
  expect(JSON.parse(String(requests[2]!.init!.body))).toEqual({ name: "setup.exe" });
});

test("the GitHub binding reads every page of releases", async () => {
  const page = (n: number) => Array.from({ length: n }, (_, i) => ({ id: i, tag_name: `v1.0.${i}`, draft: false, assets: [] }));
  const requests = await withFetch(
    (url) => Response.json(url.endsWith("page=1") ? page(100) : page(3)),
    async () => {
      expect(await githubApi(REPO, "t").listReleases()).toHaveLength(103);
    },
  );
  expect(requests.map((r) => r.line)).toEqual([
    `GET ${API}/releases?per_page=100&page=1`,
    `GET ${API}/releases?per_page=100&page=2`,
  ]);
});
