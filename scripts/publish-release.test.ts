import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { test } from "node:test";
import {
  buildManifest,
  buildWindowsManifest,
  githubApi,
  isNewer,
  main,
  publish,
  type Manifest,
  type Release,
  type ReleasesApi,
} from "./publish-release.ts";

const REPO = "twistymaze/longhand";
const SHA = "0123456789abcdef0123456789abcdef01234567";
const DMG = "Longhand_aarch64.dmg";
const INSTALLER = "Longhand_x64-setup.exe";

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

test("the Windows manifest names the version, the notes, the date, and the windows-x86_64 installer and signature", () => {
  assert.deepEqual(
    buildWindowsManifest({
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix\n",
      pubDate: "2026-10-11T15:00:00.000Z",
      url: `https://example.test/${INSTALLER}`,
      signature: "c2lnbmF0dXJl\n",
    }),
    {
      version: "1.0.4",
      notes: "### Bug Fixes\n- one fix",
      pub_date: "2026-10-11T15:00:00.000Z",
      platforms: { "windows-x86_64": { signature: "c2lnbmF0dXJl", url: `https://example.test/${INSTALLER}` } },
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
    // Records each upload, draft and publish, in order.
    events: [] as string[],
    // Runs before each upload. A test can make it throw.
    beforeUpload: (_name: string) => {},
    // True when a newer CI run on main is queued or running.
    newerRun: false,
    // The content that an authenticated read of an asset gives. A test can
    // replace it.
    readContent: (release: FakeRelease, name: string): string => release.files[name]!,
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
    async listReleases() {
      return state.releases.map(view);
    },
    async createDraft(draft) {
      state.events.push(`draft ${draft.tag}`);
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
      // GitHub refuses a second asset with the same name (422).
      if (found.assets.some((a) => a.name === name)) throw new Error(`422: ${name} already exists`);
      const asset = { id: nextId++, name };
      found.assets.push(asset);
      found.files[name] = typeof content === "string" ? content : await content.text();
      state.events.push(`upload ${name}${found.draft ? " to draft" : ""}`);
      return { ...asset };
    },
    async readAsset(release, asset) {
      const found = find(release.id);
      state.events.push(`read ${asset.name}`);
      return state.readContent(found, found.assets.find((a) => a.id === asset.id)!.name);
    },
    async publishRelease(release) {
      for (const r of state.releases) r.latest = false;
      Object.assign(find(release.id), { draft: false, latest: true });
      state.events.push(`publish ${find(release.id).tag_name}`);
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
async function seedRelease(api: ReleasesApi, state: { events: string[] }, version: string) {
  const release = await api.createDraft({ tag: `v${version}`, target: SHA, name: `Longhand ${version}`, body: "" });
  await api.publishRelease(release);
  state.events.length = 0;
}

const args = (version: string) => ({
  repository: REPO,
  version,
  commit: SHA,
  dmg: new Blob(["dmg"]),
  dmgName: DMG,
  archive: new Blob(["archive"]),
  archiveName: "Longhand.app.tar.gz",
  signature: "c2ln\n",
  installer: new Blob(["exe"]),
  installerName: INSTALLER,
  installerSignature: "d2lu\n",
  notes: "### Features\n- one feature\n",
  changelogJson: new Blob(['{"format":1}\n']),
  changelogMarkdown: new Blob(["# Changelog\n"]),
  now: () => new Date("2026-10-11T15:00:00Z"),
});
const quiet = () => {};

const ALL_ASSETS = [
  DMG,
  "Longhand.app.tar.gz",
  "Longhand.app.tar.gz.sig",
  "latest.json",
  INSTALLER,
  `${INSTALLER}.sig`,
  "latest-windows.json",
  "changelog.json",
  "CHANGELOG.md",
];

test("with no release yet, publish creates v1.0.0 with the 9 assets on the built commit as the latest release", async () => {
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
  assert.deepEqual(release.assets.map((a) => a.name), ALL_ASSETS);
  assert.equal(release.files[DMG], "dmg");
  assert.equal(release.files["Longhand.app.tar.gz.sig"], "c2ln\n");
  assert.equal(release.files[INSTALLER], "exe");
  assert.equal(release.files[`${INSTALLER}.sig`], "d2lu\n");
  assert.equal(release.files["changelog.json"], '{"format":1}\n');
  assert.equal(release.files["CHANGELOG.md"], "# Changelog\n");
});

test("publish uploads the 9 assets to the draft in order, and the release becomes the latest only after the last upload", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.0");
  const latestAtUpload: (string | undefined)[] = [];
  state.beforeUpload = () => latestAtUpload.push(state.releases.find((r) => r.latest)?.tag_name);
  assert.equal(await publish(api, args("1.0.1"), quiet), "published");
  const uploads = state.events.filter((e) => e.startsWith("upload "));
  assert.deepEqual(uploads, ALL_ASSETS.map((name) => `upload ${name} to draft`));
  assert.equal(state.events.at(-1), "publish v1.0.1");
  assert.ok(state.events.indexOf("publish v1.0.1") > state.events.indexOf("upload CHANGELOG.md to draft"));
  assert.deepEqual(latestAtUpload, ALL_ASSETS.map(() => "v1.0.0"));
  assert.equal(state.releases.find((r) => r.latest)?.tag_name, "v1.0.1");
});

test("latest.json names the archive and latest-windows.json names the fixed installer, each with its trimmed signature", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.3");
  assert.equal(await publish(api, args("1.0.4"), quiet), "published");
  const release = state.releases.find((r) => r.tag_name === "v1.0.4")!;
  const mac = JSON.parse(release.files["latest.json"]!) as Manifest;
  assert.equal(mac.version, "1.0.4");
  assert.deepEqual(mac.platforms, {
    "darwin-aarch64": {
      signature: "c2ln",
      url: `https://github.com/${REPO}/releases/download/v1.0.4/Longhand.app.tar.gz`,
    },
  });
  assert.deepEqual(JSON.parse(release.files["latest-windows.json"]!) as Manifest, {
    version: "1.0.4",
    notes: "### Features\n- one feature",
    pub_date: "2026-10-11T15:00:00.000Z",
    platforms: {
      "windows-x86_64": {
        signature: "d2lu",
        url: `https://github.com/${REPO}/releases/download/v1.0.4/${INSTALLER}`,
      },
    },
  });
});

test("a DMG or an installer with a versioned name fails the publish before any draft", async () => {
  for (const bad of [{ dmgName: "Longhand_1.0.0_aarch64.dmg" }, { installerName: "Longhand_1.0.0_x64-setup.exe" }]) {
    const { api, state } = fakeRepo();
    await seedRelease(api, state, "0.9.0");
    const name = Object.values(bad)[0]!;
    await assert.rejects(publish(api, { ...args("1.0.0"), ...bad }, quiet), new RegExp(name.replaceAll(".", "\\.")));
    assert.deepEqual(state.events, []);
    assert.deepEqual(state.releases.map((r) => r.tag_name), ["v0.9.0"]);
  }
});

test("a re-run whose version the latest release already has publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.4");
  assert.equal(await publish(api, args("1.0.4"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.0.4"]);
  assert.deepEqual(state.events, []);
});

test("an older run that finishes after a newer one publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.1.0");
  assert.equal(await publish(api, args("1.0.4"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.1.0"]);
});

test("a run that a newer CI run on main follows publishes nothing", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.0");
  state.newerRun = true;
  assert.equal(await publish(api, args("1.0.1"), quiet), "skipped");
  assert.deepEqual(state.releases.map((r) => r.tag_name), ["v1.0.0"]);
  assert.deepEqual(state.events, []);
});

test("a stale draft with the same tag is deleted before the new draft", async () => {
  const { api, state } = fakeRepo();
  await api.createDraft({ tag: "v1.0.0", target: SHA, name: "Longhand 1.0.0", body: "" });
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  assert.deepEqual(state.releases.map((r) => [r.id, r.draft]), [[2, false]]);
});

test("an upload that fails part-way leaves a draft that is not published and keeps the latest release", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.0");
  state.beforeUpload = (name) => {
    if (name === INSTALLER) throw new Error(`upload ${INSTALLER}: 502`);
  };
  await assert.rejects(publish(api, args("1.0.1"), quiet), /502/);
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.draft, r.latest]), [
    ["v1.0.0", false, true],
    ["v1.0.1", true, false],
  ]);
  assert.equal(state.events.some((e) => e.startsWith("publish ")), false);
});

test("publish reads back both manifests from the draft before it publishes", async () => {
  const { api, state } = fakeRepo();
  assert.equal(await publish(api, args("1.0.0"), quiet), "published");
  const reads = state.events.filter((e) => e.startsWith("read "));
  assert.deepEqual(reads.sort(), ["read latest-windows.json", "read latest.json"]);
  for (const read of reads) assert.ok(state.events.indexOf(read) < state.events.indexOf("publish v1.0.0"));
});

test("a draft manifest that differs from the upload fails the publish before the draft is published", async () => {
  for (const name of ["latest.json", "latest-windows.json"]) {
    const { api, state } = fakeRepo();
    await seedRelease(api, state, "1.0.0");
    state.readContent = (release, asset) => (asset === name ? "{}\n" : release.files[asset]!);
    await assert.rejects(publish(api, args("1.0.1"), quiet), new RegExp(`${name.replaceAll(".", "\\.")} of the draft v1\\.0\\.1 differs`));
    assert.equal(state.events.some((e) => e.startsWith("publish ")), false);
    assert.equal(state.releases.find((r) => r.latest)?.tag_name, "v1.0.0");
  }
});

/// Writes the release files of a CLI publish and returns its arguments.
function cliFiles(version: string, names: { dmg?: string; installer?: string } = {}): string[] {
  const dmg = names.dmg ?? DMG;
  const installer = names.installer ?? INSTALLER;
  const dir = `${process.env.TMPDIR ?? "/tmp"}/publish-release-test-${process.pid}`;
  mkdirSync(`${dir}/changelog`, { recursive: true });
  writeFileSync(`${dir}/${dmg}`, "dmg");
  writeFileSync(`${dir}/Longhand.app.tar.gz`, "archive");
  writeFileSync(`${dir}/Longhand.app.tar.gz.sig`, "c2ln\n");
  writeFileSync(`${dir}/${installer}`, "exe");
  writeFileSync(`${dir}/${installer}.sig`, "d2lu\n");
  writeFileSync(`${dir}/notes.md`, "notes\n");
  writeFileSync(`${dir}/changelog/changelog.json`, '{"format":1}\n');
  writeFileSync(`${dir}/changelog/CHANGELOG.md`, "# Changelog\n");
  return [
    "publish",
    version,
    `${dir}/${dmg}`,
    `${dir}/Longhand.app.tar.gz`,
    `${dir}/Longhand.app.tar.gz.sig`,
    `${dir}/${installer}`,
    `${dir}/${installer}.sig`,
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
  await main(cliFiles("1.0.0"), ENV, makeApi, quiet, quiet);
  assert.deepEqual(made, [`${REPO} t`]);
  assert.partialDeepStrictEqual(state.releases[0], { tag_name: "v1.0.0", target_commitish: SHA, body: "notes\n", draft: false });
  const files = state.releases[0]!.files;
  assert.equal(files[DMG], "dmg");
  assert.equal(files["Longhand.app.tar.gz.sig"], "c2ln\n");
  assert.equal(files[INSTALLER], "exe");
  assert.equal(files[`${INSTALLER}.sig`], "d2lu\n");
  assert.equal((JSON.parse(files["latest-windows.json"]!) as Manifest).platforms["windows-x86_64"]!.signature, "d2lu");
  assert.equal(files["changelog.json"], '{"format":1}\n');
  assert.equal(files["CHANGELOG.md"], "# Changelog\n");
});

test("a CLI publish writes published=true right after the release is published and ends without an error", async () => {
  const { api, state } = fakeRepo();
  await main(cliFiles("1.0.0"), ENV, () => api, quiet, (line) => state.events.push(`output ${line}`));
  assert.deepEqual(state.events.slice(-2), ["publish v1.0.0", "output published=true"]);
});

test("a CLI publish with a versioned installer name fails before any draft", async () => {
  const { api, state } = fakeRepo();
  const output: string[] = [];
  await assert.rejects(
    main(cliFiles("1.0.0", { installer: "Longhand_1.0.0_x64-setup.exe" }), ENV, () => api, quiet, (line) => output.push(line)),
    /Longhand_1\.0\.0_x64-setup\.exe/,
  );
  assert.deepEqual(state.releases, []);
  assert.deepEqual(output, []);
});

test("a CLI publish that a newer CI run follows uploads nothing and reports that it did not publish", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.0");
  state.newerRun = true;
  const output: string[] = [];
  await main(cliFiles("1.0.1"), ENV, () => api, quiet, (line) => output.push(line));
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.assets.length]), [["v1.0.0", 0]]);
  assert.deepEqual(output, ["published=false"]);
});

test("a CLI publish whose version is not newer reports that it did not publish", async () => {
  const { api, state } = fakeRepo();
  await seedRelease(api, state, "1.0.1");
  const output: string[] = [];
  await main(cliFiles("1.0.1"), ENV, () => api, quiet, (line) => output.push(line));
  assert.deepEqual(state.releases.map((r) => [r.tag_name, r.assets.length]), [["v1.0.1", 0]]);
  assert.deepEqual(output, ["published=false"]);
});

const USAGE_TEXT =
  /publish <version> <dmg> <archive> <archive\.sig> <installer> <installer\.sig> <notes file> <changelog\.json> <CHANGELOG\.md>/;

test("the CLI rejects the old 7-argument publish with the usage text", async () => {
  const { api, state } = fakeRepo();
  await assert.rejects(main(["publish", "1.0.0", "a", "b", "c", "d", "e", "f"], ENV, () => api, quiet), USAGE_TEXT);
  assert.deepEqual(state.events, []);
});

test("the CLI rejects attach with the usage text", async () => {
  const { api } = fakeRepo();
  await assert.rejects(main(["attach", "v1.0.0", "/b/setup.exe", "/b/setup.exe.sig"], ENV, () => api, quiet), (error: Error) => {
    assert.match(error.message, USAGE_TEXT);
    assert.doesNotMatch(error.message, /attach/);
    return true;
  });
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
  const nine = ["publish", "1.0.0", "a", "b", "c", "d", "e", "f", "g", "h"];
  await assert.rejects(main(nine, {}, () => api, quiet), /GITHUB_REPOSITORY/);
  await assert.rejects(main(nine, { GITHUB_REPOSITORY: REPO, GITHUB_TOKEN: "t" }, () => api, quiet), /GITHUB_SHA/);
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

test("the GitHub binding fails on an error other than a 404", async () => {
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

test("the GitHub binding uploads an asset", async () => {
  const requests = await withFetch(
    () => Response.json({ id: 7, name: "latest.json" }, { status: 201 }),
    async () => {
      const release = { id: 9, tag_name: "v1.0.0", draft: true, assets: [] };
      assert.deepEqual(await githubApi(REPO, "t").uploadAsset(release, "latest.json", "{}"), { id: 7, name: "latest.json" });
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [
    `POST https://uploads.github.com/repos/${REPO}/releases/9/assets?name=latest.json`,
  ]);
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

test("the GitHub binding reads the content of a draft asset with the token and fails on an error", async () => {
  const release = { id: 9, tag_name: "v1.0.0", draft: true, assets: [] };
  const asset = { id: 7, name: "latest-windows.json" };
  const requests = await withFetch(
    () => new Response("{}\n"),
    async () => {
      assert.equal(await githubApi(REPO, "t").readAsset(release, asset), "{}\n");
    },
  );
  assert.deepEqual(requests.map((r) => r.line), [`GET ${API}/releases/assets/7`]);
  const headers = requests[0]!.init!.headers as Record<string, string>;
  assert.equal(headers.Authorization, "Bearer t");
  assert.equal(headers.Accept, "application/octet-stream");
  await withFetch(
    () => new Response("not found", { status: 404 }),
    async () => {
      await assert.rejects(githubApi(REPO, "t").readAsset(release, asset), /404/);
    },
  );
});
