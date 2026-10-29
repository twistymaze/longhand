import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { test } from "node:test";
import {
  attach,
  buildManifest,
  buildWindowsManifest,
  githubApi,
  isNewer,
  main,
  publish,
  type Asset,
  type Manifest,
  type Release,
  type ReleasesApi,
} from "./publish-release.ts";

const REPO = "twistymaze/longhand";
const SHA = "0123456789abcdef0123456789abcdef01234567";

test("the manifest names the version, the notes, the date, and the darwin-aarch64 archive and signature", () => {
  assert.deepEqual(
    buildManifest({
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix\n",
      pubDate: "2026-10-11T15:00:00.000Z",
      url: "https://example.test/Longhand.app.tar.gz",
      signature: "c2lnbmF0dXJl\n",
    }),
    {
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix",
      pub_date: "2026-10-11T15:00:00.000Z",
      platforms: { "darwin-aarch64": { signature: "c2lnbmF0dXJl", url: "https://example.test/Longhand.app.tar.gz" } },
    },
  );
});

test("only a higher version is newer, and no release means newer", () => {
  assert.equal(isNewer("1.0.1", "1.0.0"), true);
  assert.equal(isNewer("1.1.0", "1.0.9"), true);
  assert.equal(isNewer("1.0.0", "1.0.0"), false);
  assert.equal(isNewer("1.0.0", "1.0.1"), false);
  assert.equal(isNewer("1.0.0", null), true);
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
    // The public download of an asset of a published release, or null when
    // there is none. A test can replace it.
    publicContent: (tag: string, name: string): string | null =>
      state.releases.find((r) => !r.draft && r.tag_name === tag)?.files[name] ?? null,
  };
  const view = (r: FakeRelease): Release => ({
    id: r.id,
    tag_name: r.tag_name,
    draft: r.draft,
    body: r.body,
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
    async readPublicAsset(tag, name) {
      return state.publicContent(tag, name);
    },
  };
  return { api, state };
}

/// Adds a published release, as an earlier run would.
async function seedRelease(api: ReleasesApi, version: string, body = "") {
  const release = await api.createDraft({ tag: `v${version}`, target: SHA, name: `Longhand ${version}`, body });
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
  changelogJson: new Blob(['{"format":1}\n']),
  changelogMarkdown: new Blob(["# Changelog\n"]),
  now: () => new Date("2026-10-11T15:00:00Z"),
});
const quiet = () => {};

test("with no release yet, publish creates v1.0.0 with the 6 assets on the built commit as the latest release", async () => {
  const { api, state } = fakeRepo();
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  assert.equal(state.releases.length, 1);
  const release = state.releases[0]!;
  assert.partialDeepStrictEqual(release, {
    tag_name: "v1.0.0",
    target_commitish: SHA,
    name: "Longhand 1.0.0",
    body: "### Features\n- one feature\n",
    prerelease: false,
    draft: false,
    latest: true,
  });
  assert.deepEqual(release.assets.map((a) => a.name), [
    "Longhand_1.0.0_aarch64.dmg",
    "Longhand.app.tar.gz",
    "Longhand.app.tar.gz.sig",
    "latest.json",
    "changelog.json",
    "CHANGELOG.md",
  ]);
  assert.equal(release.files["Longhand.app.tar.gz.sig"], "c2ln\n");
  assert.equal(release.files["changelog.json"], '{"format":1}\n');
  assert.equal(release.files["CHANGELOG.md"], "# Changelog\n");
});

test("publish uploads every asset to the draft before it publishes the release", async () => {
  const { api, state } = fakeRepo();
  const draftAtUpload: boolean[] = [];
  state.beforeUpload = () => draftAtUpload.push(state.releases[0]!.draft);
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  assert.deepEqual(draftAtUpload, [true, true, true, true, true, true]);
  assert.equal(state.releases[0]!.draft, false);
});

test("latest.json names the version, the trimmed signature and the archive of its own release", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.3");
  assert.equal(await publish(api, args("1.0.4"), quiet), "published");
  const release = state.releases.find((r) => r.tag_name === "v1.0.4")!;
  const manifest = JSON.parse(release.files["latest.json"]!) as Manifest;
  assert.equal(manifest.version, "1.0.4");
  assert.deepEqual(manifest.platforms["darwin-aarch64"], {
    signature: "c2ln",
    url: "https://github.com/twistymaze/longhand/releases/download/v1.0.4/Longhand.app.tar.gz",
  });
});

test("a re-run whose version the latest release already has publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.4");
  assert.equal(await publish(api, args("1.0.4"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.0.4"]);
});

test("an older run that finishes after a newer one publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.1.0");
  assert.equal(await publish(api, args("1.0.4"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.1.0"]);
});

test("a run that a newer CI run on main follows publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  state.newerRun = true;
  assert.equal(await publish(api, args("1.0.1"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.0.0"]);
});

test("a stale draft with the same tag is deleted before the new draft", async () => {
  const { api, state } = fakeRepo();
  await api.createDraft({ tag: "v1.0.0", target: SHA, name: "Longhand 1.0.0", body: "" });
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  assert.deepEqual(state.releases.map((r) => [r.id, r.draft]), [[2, false]]);
});

test("a failed upload leaves only a draft and fails the publish", async () => {
  const { api, state } = fakeRepo();
  state.beforeUpload = (name) => {
    if (name === "Longhand.app.tar.gz") throw new Error("upload Longhand.app.tar.gz: 502");
  };
  await assert.rejects(publish(api, args("1.0.0"), quiet), /502/);
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.draft]), [["v1.0.0", true]]);
});

test("the Windows manifest names the version, the notes, the date, and the windows-x86_64 installer and signature", () => {
  assert.deepEqual(
    buildWindowsManifest({
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix\n",
      pubDate: "2026-10-11T15:00:00.000Z",
      url: "https://example.test/Longhand_1.0.4_x64-setup.exe",
      signature: "c2lnbmF0dXJl\n",
    }),
    {
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix",
      pub_date: "2026-10-11T15:00:00.000Z",
      platforms: { "windows-x86_64": { signature: "c2lnbmF0dXJl", url: "https://example.test/Longhand_1.0.4_x64-setup.exe" } },
    },
  );
});

const INSTALLER = "Longhand_1.0.0_x64-setup.exe";
const attachArgs = (installer: string, signature = "d2lu\n") => ({
  repository: REPO,
  tag: "v1.0.0",
  installer: new Blob([installer]),
  installerName: INSTALLER,
  signature,
  now: () => new Date("2026-10-12T09:00:00Z"),
});

/// Records each upload and delete of the fake repository, in order.
function recordChanges(api: ReleasesApi): string[] {
  const changes: string[] = [];
  const upload = api.uploadAsset.bind(api);
  const remove = api.deleteAsset.bind(api);
  api.uploadAsset = (release, name, content) => {
    changes.push(`upload ${name}`);
    return upload(release, name, content);
  };
  api.deleteAsset = (release, asset) => {
    changes.push(`delete ${asset.name}`);
    return remove(release, asset);
  };
  return changes;
}

test("attach uploads the installer, its signature, then latest-windows.json for the release with the tag", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0", "### Features\n- one feature\n");
  const changes = recordChanges(api);
  await attach(api, attachArgs("exe"), quiet);
  assert.deepEqual(changes, [`upload ${INSTALLER}`, `upload ${INSTALLER}.sig`, "upload latest-windows.json"]);
  const files = state.releases[0]!.files;
  assert.equal(files[INSTALLER], "exe");
  assert.equal(files[`${INSTALLER}.sig`], "d2lu\n");
  assert.deepEqual(JSON.parse(files["latest-windows.json"]!) as Manifest, {
    version: "1.0.0",
    notes: "### Features\n- one feature",
    pub_date: "2026-10-12T09:00:00.000Z",
    platforms: {
      "windows-x86_64": {
        signature: "d2lu",
        url: `https://github.com/${REPO}/releases/download/v1.0.0/${INSTALLER}`,
      },
    },
  });
});

test("attach never uploads or deletes latest.json", async () => {
  const { api, state } = fakeRepo();
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  const macManifest = state.releases[0]!.files["latest.json"];
  const changes = recordChanges(api);
  await attach(api, attachArgs("first"), quiet);
  await attach(api, attachArgs("second"), quiet);
  assert.equal(changes.filter((c) => c.endsWith(" latest.json")).length, 0);
  assert.equal(state.releases[0]!.files["latest.json"], macManifest);
});

test("a re-run deletes the old manifest first, replaces the installer and the signature, and uploads the new manifest last", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  await attach(api, attachArgs("first", "Zmlyc3Q=\n"), quiet);
  const changes = recordChanges(api);
  await attach(api, attachArgs("second", "c2Vjb25k\n"), quiet);
  assert.deepEqual(changes, [
    "delete latest-windows.json",
    `upload ${INSTALLER}.new`,
    `delete ${INSTALLER}`,
    `upload ${INSTALLER}.sig.new`,
    `delete ${INSTALLER}.sig`,
    "upload latest-windows.json",
  ]);
  const release = state.releases[0]!;
  assert.deepEqual(release.assets.map((a) => a.name).sort(), [INSTALLER, `${INSTALLER}.sig`, "latest-windows.json"].sort());
  assert.equal(release.files[INSTALLER], "second");
  assert.equal(release.files[`${INSTALLER}.sig`], "c2Vjb25k\n");
  assert.equal((JSON.parse(release.files["latest-windows.json"]!) as Manifest).platforms["windows-x86_64"]!.signature, "c2Vjb25k");
});

test("a failed re-upload keeps the earlier installer and leaves no Windows manifest", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  await attach(api, attachArgs("first"), quiet);
  state.beforeUpload = () => {
    throw new Error("upload failed");
  };
  await assert.rejects(attach(api, attachArgs("second"), quiet), /upload failed/);
  assert.equal(state.releases[0]!.files[INSTALLER], "first");
  assert.equal(state.releases[0]!.files["latest-windows.json"], undefined);
});

test("attach fails without uploads when no release has the tag", async () => {
  const { api } = fakeRepo();
  const changes = recordChanges(api);
  await assert.rejects(attach(api, attachArgs("exe"), quiet), /no release has the tag v1\.0\.0/);
  assert.deepEqual(changes, []);
});

test("attach fails when the public latest-windows.json differs from the uploaded manifest", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  state.publicContent = () => "{}\n";
  await assert.rejects(attach(api, attachArgs("exe"), quiet), /latest-windows\.json of v1\.0\.0 differs/);
});

test("attach reads back the public latest-windows.json of the tag", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  const reads: string[] = [];
  state.publicContent = (tag, name) => {
    reads.push(`${tag} ${name}`);
    return state.releases[0]!.files[name] ?? null;
  };
  await attach(api, attachArgs("exe"), quiet);
  assert.deepEqual(reads, ["v1.0.0 latest-windows.json"]);
});

/// Writes the release files of a CLI publish and returns its arguments.
function cliFiles(version: string): string[] {
  const dir = `${process.env.TMPDIR ?? "/tmp"}/publish-release-test-${process.pid}`;
  mkdirSync(`${dir}/changelog`, { recursive: true });
  writeFileSync(`${dir}/Longhand_1.0.0_aarch64.dmg`, "dmg");
  writeFileSync(`${dir}/Longhand.app.tar.gz`, "archive");
  writeFileSync(`${dir}/Longhand.app.tar.gz.sig`, "c2ln\n");
  writeFileSync(`${dir}/notes.md`, "notes\n");
  writeFileSync(`${dir}/changelog/changelog.json`, '{"format":1}\n');
  writeFileSync(`${dir}/changelog/CHANGELOG.md`, "# Changelog\n");
  return [
    "publish",
    version,
    `${dir}/Longhand_1.0.0_aarch64.dmg`,
    `${dir}/Longhand.app.tar.gz`,
    `${dir}/Longhand.app.tar.gz.sig`,
    `${dir}/notes.md`,
    `${dir}/changelog/changelog.json`,
    `${dir}/changelog/CHANGELOG.md`,
  ];
}

const ENV = { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t", GITHUB_SHA: SHA };

test("the CLI reads the repository, the token and the built commit from the environment", async () => {
  const made: string[] = [];
  const { api, state } = fakeRepo();
  const makeApi = (repository: string, token: string) => {
    made.push(`${repository} ${token}`);
    return api;
  };
  const output: string[] = [];
  await main(cliFiles("1.0.0"), ENV, makeApi, quiet, (line) => output.push(line));
  assert.deepEqual(made, [`${REPO} t`]);
  assert.partialDeepStrictEqual(state.releases[0], { tag_name: "v1.0.0", target_commitish: SHA, body: "notes\n", draft: false });
  assert.equal(state.releases[0]!.files["Longhand_1.0.0_aarch64.dmg"], "dmg");
  assert.equal(state.releases[0]!.files["changelog.json"], '{"format":1}\n');
  assert.equal(state.releases[0]!.files["CHANGELOG.md"], "# Changelog\n");
  assert.deepEqual(output, ["published=true"]);
});

test("a CLI publish that a newer CI run follows uploads nothing and reports that it did not publish", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  state.newerRun = true;
  const output: string[] = [];
  await main(cliFiles("1.0.1"), ENV, () => api, quiet, (line) => output.push(line));
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.assets.length]), [["v1.0.0", 0]]);
  assert.deepEqual(output, ["published=false"]);
});

test("a CLI publish whose version is not newer reports that it did not publish", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.1");
  const output: string[] = [];
  await main(cliFiles("1.0.1"), ENV, () => api, quiet, (line) => output.push(line));
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.assets.length]), [["v1.0.1", 0]]);
  assert.deepEqual(output, ["published=false"]);
});

test("the CLI usage names the changelog files of a publish", async () => {
  const { api } = fakeRepo();
  await assert.rejects(
    main(["publish", "1.0.0", "a", "b", "c", "d"], ENV, () => api, quiet),
    /publish <version> <dmg> <archive> <signature file> <notes file> <changelog\.json> <CHANGELOG\.md>/,
  );
});

test("the CLI rejects the old two-argument attach with the usage text", async () => {
  const { api } = fakeRepo();
  await assert.rejects(
    main(["attach", "v1.0.0", "/b/setup.exe"], ENV, () => api, quiet),
    /attach <tag> <installer> <signature file>/,
  );
});

test("a CLI attach uploads the installer, its signature file and the Windows manifest", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, "1.0.0");
  const dir = `${process.env.TMPDIR ?? "/tmp"}/publish-release-test-${process.pid}/installer`;
  mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/${INSTALLER}`, "exe");
  writeFileSync(`${dir}/${INSTALLER}.sig`, "d2lu\n");
  await main(["attach", "v1.0.0", `${dir}/${INSTALLER}`, `${dir}/${INSTALLER}.sig`], ENV, () => api, quiet);
  const files = state.releases[0]!.files;
  assert.equal(files[INSTALLER], "exe");
  assert.equal(files[`${INSTALLER}.sig`], "d2lu\n");
  assert.equal((JSON.parse(files["latest-windows.json"]!) as Manifest).platforms["windows-x86_64"]!.signature, "d2lu");
});

test("a CLI publish with a failed upload rejects, so the process exits with an error", async () => {
  const { api, state } = fakeRepo();
  state.beforeUpload = () => {
    throw new Error("upload: 502");
  };
  const output: string[] = [];
  await assert.rejects(main(cliFiles("1.0.0"), ENV, () => api, quiet, (line) => output.push(line)), /502/);
  assert.deepEqual(output, []);
});

test("the CLI fails when the environment does not name the repository, the token and the commit", async () => {
  const { api } = fakeRepo();
  await assert.rejects(main(["attach", "v1.0.0", "/b/setup.exe", "/b/setup.exe.sig"], {}, () => api, quiet), /GITHUB_REPOSITORY/);
  await assert.rejects(
    main(["publish", "1.0.0", "a", "b", "c", "d", "e", "f"], { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t" }, () => api, quiet),
    /GITHUB_SHA/,
  );
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
      assert.equal(await githubApi(REPO, "t").latestRelease(), null);
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [`GET ${API}/releases/latest`]);
  const headers = requests[0]!.init!.headers as Record<string, string>;
  assert.equal(headers["X-GitHub-Api-Version"], "2022-11-28");
  assert.equal(headers.Authorization, "Bearer t");
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
  assert.equal(queued.result, true);
  assert.deepEqual(queued.lines, [
    `GET ${API}/actions/runs/77`,
    `GET ${API}/actions/workflows/5/runs?branch=main&event=push&per_page=20`,
  ]);
  assert.equal((await check(10, [{ run_number: 11, status: "completed" }, { run_number: 9, status: "queued" }])).result, false);
  assert.equal(await githubApi(REPO, "t").newerRunWaiting(), false);
});

test("the GitHub binding treats a 404 from a tag as no release and fails on other errors", async () => {
  await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      assert.equal(await githubApi(REPO, "t").releaseByTag("v1.0.0"), null);
    },
  );
  await withFetch(
    () => new Response("server error", { status: 500 }),
    async () => {
      await assert.rejects(githubApi(REPO, "t").latestRelease(), /500/);
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
  assert.deepEqual(requests.map((r) => r.line), [`POST ${API}/releases`]);
  assert.deepEqual(JSON.parse(String(requests[0]!.init!.body)), {
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
  assert.deepEqual(requests.map((r) => r.line), [`PATCH ${API}/releases/9`]);
  assert.deepEqual(JSON.parse(String(requests[0]!.init!.body)), { draft: false, make_latest: "true" });
});

test("the GitHub binding uploads, deletes and renames an asset", async () => {
  const asset: Asset = { id: 5, name: "latest.json" };
  const requests = await withFetch(
    () => Response.json({ id: 7, name: "latest.json" }, { status: 201 }),
    async () => {
      const api = githubApi(REPO, "t");
      const release = { id: 9, tag_name: "v1.0.0", draft: true, assets: [] };
      assert.deepEqual(await api.uploadAsset(release, "latest.json", "{}"), { id: 7, name: "latest.json" });
      await api.deleteAsset(release, asset);
      await api.renameAsset(release, asset, "setup.exe");
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [
    `POST https://uploads.github.com/repos/${REPO}/releases/9/assets?name=latest.json`,
    `DELETE ${API}/releases/assets/5`,
    `PATCH ${API}/releases/assets/5`,
  ]);
  assert.deepEqual(JSON.parse(String(requests[2]!.init!.body)), { name: "setup.exe" });
});

test("the GitHub binding reads every page of releases", async () => {
  const page = (n: number) => Array.from({ length: n }, (_, i) => ({ id: i, tag_name: `v1.0.${i}`, draft: false, assets: [] }));
  const requests = await withFetch(
    (url) => Response.json(url.endsWith("page=1") ? page(100) : page(3)),
    async () => {
      assert.equal((await githubApi(REPO, "t").listReleases()).length, 103);
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [
    `GET ${API}/releases?per_page=100&page=1`,
    `GET ${API}/releases?per_page=100&page=2`,
  ]);
});

test("the GitHub binding reads a public asset without the token and treats a 404 as no asset", async () => {
  const url = `https://github.com/${REPO}/releases/download/v1.0.0/latest-windows.json`;
  const requests = await withFetch(
    () => new Response("{}\n"),
    async () => {
      assert.equal(await githubApi(REPO, "t").readPublicAsset("v1.0.0", "latest-windows.json"), "{}\n");
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [`GET ${url}`]);
  assert.equal(requests[0]!.init, undefined);
  await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      assert.equal(await githubApi(REPO, "t").readPublicAsset("v1.0.0", "latest-windows.json"), null);
    },
  );
});
