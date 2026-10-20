// List, move and check the feature ideas in docs/ideas/.
//
// The folder of an idea is its status. The frontmatter holds a one-line
// description, so this script reads only the frontmatter. docs/ideas/README.md
// describes the rules.
//
//   node scripts/ideas.ts [folder...] [--area <area>]   list ideas (Done shows only a count unless you name it)
//   node scripts/ideas.ts move <slug> <folder>           git mv the idea and change every link to it
//   node scripts/ideas.ts check                          find frontmatter errors and broken links
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, normalize, relative, resolve } from "node:path";
import { parseFrontmatter } from "./lib/frontmatter.ts";

const root = resolve(import.meta.dirname, "..");
const ideasDir = join(root, "docs/ideas");
const folders = ["next", "someday", "parked", "done"] as const;
type Folder = (typeof folders)[number];
const areas = ["capture", "transcript-and-summary", "processing", "library-and-sharing", "interface", "distribution"];
const required = ["title", "description", "area", "added", "source"];

type Idea = { slug: string; folder: Folder; path: string; meta: Record<string, unknown>; body: string };

function readIdea(folder: Folder, file: string): Idea {
  const path = join(ideasDir, folder, file);
  const text = readFileSync(path, "utf8");
  const match = /^---\n([\s\S]*?)\n---\n?([\s\S]*)$/.exec(text);
  const meta: Record<string, unknown> = match ? parseFrontmatter(match[1]) : {};
  return { slug: file.replace(/\.md$/, ""), folder, path, meta, body: match ? match[2] : text };
}

function allIdeas(): Idea[] {
  return folders.flatMap((folder) => {
    const dir = join(ideasDir, folder);
    if (!existsSync(dir)) return [];
    return readdirSync(dir)
      .filter((f) => f.endsWith(".md"))
      .sort()
      .map((f) => readIdea(folder, f));
  });
}

// Markdown files that can link to an idea.
function linkingFiles(): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (name.endsWith(".md")) out.push(p);
    }
  };
  walk(join(root, "docs"));
  for (const name of ["CLAUDE.md", "CONCEPTS.md", "STRATEGY.md", "PROVENANCE.md"]) {
    const p = join(root, name);
    if (existsSync(p)) out.push(p);
  }
  return out;
}

const linkPattern = /(\]\()([^)\s#]+?\.md)(#[^)]*)?\)/g;
const rootPathPattern = /(?<![\w/.])docs\/ideas\/[\w/-]+\.md/g;

// A pattern for the path of the folder `abs` from the root, as text in a
// document. A dot after the path that starts a file extension is part of
// another name, for example `<slug>-screenshots.png`.
function rootPathOf(abs: string): RegExp {
  const path = relative(root, abs).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?<![\\w/.])${path}(?![\\w-]|\\.\\w)`, "g");
}

// Change the links in one file after `from` moved to `to`. When the file itself
// moved, `fileBefore` is its old path, so that its relative links resolve from
// the folder where they were written.
function rewriteLinks(text: string, fileNow: string, fileBefore: string, from: string, to: string): string {
  text = text.replace(linkPattern, (whole, open: string, target: string, anchor: string | undefined) => {
    if (target.includes("://")) return whole;
    let abs = normalize(resolve(dirname(fileBefore), target));
    if (abs === from) abs = to;
    else if (fileNow === fileBefore) return whole;
    return `${open}${relative(dirname(fileNow), abs)}${anchor ?? ""})`;
  });
  return text.replace(rootPathPattern, (p) => (join(root, p) === from ? relative(root, to) : p));
}

function move(slug: string, folder: string) {
  if (!folders.includes(folder as Folder)) fail(`Unknown folder "${folder}". Use one of: ${folders.join(", ")}.`);
  const idea = allIdeas().find((i) => i.slug === slug);
  if (!idea) fail(`No idea "${slug}" in docs/ideas/.`);
  const to = join(ideasDir, folder, `${slug}.md`);
  if (idea.path === to) fail(`${slug} is already in ${folder}/.`);
  // The private screenshots of an idea are in a gitignored folder beside it,
  // so git mv does not move them.
  const shotsFrom = join(dirname(idea.path), `${slug}-screenshots`);
  const shotsTo = join(dirname(to), `${slug}-screenshots`);
  if (existsSync(shotsTo)) fail(`${relative(root, shotsTo)} is already there. Move or remove it first.`);
  const shots = existsSync(shotsFrom);
  const shotsPath = rootPathOf(shotsFrom);
  const git = spawnSync("git", ["mv", idea.path, to], { cwd: root, encoding: "utf8" });
  if (git.status !== 0) fail(`git mv failed: ${git.error?.message ?? git.stderr}`);
  if (shots) renameSync(shotsFrom, shotsTo);
  let changed = 0;
  for (const file of linkingFiles()) {
    const before = file === to ? idea.path : file;
    const text = readFileSync(file, "utf8");
    let next = rewriteLinks(text, file, before, idea.path, to);
    if (shots) next = next.replace(shotsPath, relative(root, shotsTo));
    if (next !== text) {
      writeFileSync(file, next);
      changed++;
    }
  }
  console.log(`Moved ${slug}${shots ? ` and ${slug}-screenshots/` : ""} to ${folder}/. Changed links in ${changed} file(s).`);
  if (folder === "parked" && !/^## Why parked/m.test(readFileSync(to, "utf8")))
    console.log(`Add a "## Why parked" section to ${relative(root, to)}.`);
}

// In CI the check reports a shipped idea as a warning: the idea moves in a
// docs commit after the merge, and that commit runs no CI, so a failure on
// the push of the merged PR would stop its release.
const inCi = Boolean(process.env.CI);

// The branch names of the PRs that merged into main. A merge commit subject
// is "Merge pull request #N from <owner>/<branch>". A squash merge names no
// branch, so GitHub gives the rest. `unread` says why GitHub gave nothing.
// `gh pr list` gives at most this number of PRs. When it gives that many, an
// older PR can be missing, so the check says that it did not read them all.
const prLimit = 1000;

function mergedBranches(): { branches: Set<string>; unread?: string } {
  const ref = spawnSync("git", ["rev-parse", "--verify", "--quiet", "origin/main"], { cwd: root }).status === 0 ? "origin/main" : "HEAD";
  const git = spawnSync("git", ["log", ref, "--merges", "--format=%s"], { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  const branches = new Set<string>();
  for (const line of (git.stdout ?? "").split("\n")) {
    const m = /^Merge pull request #\d+ from [^/]+\/(.+)$/.exec(line.trim());
    if (m) branches.add(m[1]);
  }
  const gh = spawnSync("gh", ["pr", "list", "--state", "merged", "--base", "main", "--limit", String(prLimit), "--json", "headRefName"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
  // When no `gh` is on PATH, spawnSync does not throw: it sets `error` to ENOENT.
  if ((gh.error as NodeJS.ErrnoException | undefined)?.code === "ENOENT") return { branches, unread: "gh is not installed" };
  if (gh.status !== 0) return { branches, unread: gh.stderr?.trim().split("\n")[0] || gh.error?.message || `gh exited ${gh.status}` };
  let prs: { headRefName: string }[];
  try {
    prs = JSON.parse(gh.stdout);
  } catch {
    return { branches, unread: "gh gave output that is not JSON" };
  }
  for (const pr of prs) branches.add(pr.headRefName);
  if (prs.length >= prLimit) return { branches, unread: `more than ${prLimit} merged PRs` };
  return { branches };
}

// An idea in next/ whose plan topic has a merged branch has shipped and must
// move to done/. Plan branches are named worktree-<topic> or codex/<topic>.
// It returns the problems and, in CI, prints the shipped ideas as warnings.
function shippedButOpen(ideas: Idea[]): string[] {
  const open = ideas.filter((i) => i.folder !== "done" && i.meta.plan);
  // CI reads the merged PRs also when no plan is open, so that a missing
  // token or `gh` shows at once and not when the next plan starts.
  if (open.length === 0 && !inCi) return [];
  const { branches, unread } = mergedBranches();
  const errors: string[] = [];
  if (unread) {
    const line = `Shipped-plan check: merged PRs not read (${unread}); merge commits only.`;
    if (inCi) errors.push(line);
    else console.error(line);
  }
  for (const i of open) {
    const planPath = join(root, String(i.meta.plan));
    if (!existsSync(planPath)) continue;
    const topic = /^topic:\s*(\S+)\s*$/m.exec(readFileSync(planPath, "utf8"))?.[1];
    if (!topic) continue;
    const branch = [`worktree-${topic}`, `codex/${topic}`].find((b) => branches.has(b));
    if (!branch) continue;
    const where = relative(root, i.path);
    const text = `branch ${branch} of its plan merged; move it with "node scripts/ideas.ts move ${i.slug} done" and write the PR in its file`;
    if (inCi) console.log(`::warning file=${where}::${text}`);
    else errors.push(`${where}: ${text}`);
  }
  return errors;
}

function check(): number {
  const ideas = allIdeas();
  const slugs = new Set<string>();
  const errors: string[] = [];
  for (const i of ideas) {
    const where = relative(root, i.path);
    if (slugs.has(i.slug)) errors.push(`${where}: another folder has the same slug`);
    slugs.add(i.slug);
    for (const key of required) if (!i.meta[key]) errors.push(`${where}: no "${key}" in the frontmatter`);
    if ("status" in i.meta) errors.push(`${where}: remove "status"; the folder is the status`);
    if (i.meta.area && !areas.includes(String(i.meta.area))) errors.push(`${where}: unknown area "${i.meta.area}"`);
    if (i.meta.plan && !existsSync(join(root, String(i.meta.plan)))) errors.push(`${where}: plan ${i.meta.plan} does not exist`);
    if (i.folder === "parked" && !/^## Why parked/m.test(i.body)) errors.push(`${where}: a parked idea needs a "## Why parked" section`);
  }
  for (const i of ideas)
    for (const r of (i.meta.related as string[] | undefined) ?? [])
      if (!slugs.has(r)) errors.push(`${relative(root, i.path)}: related idea "${r}" does not exist`);
  errors.push(...shippedButOpen(ideas));
  for (const name of readdirSync(ideasDir))
    if (name.endsWith(".md") && name !== "README.md") errors.push(`docs/ideas/${name}: put the idea in a folder (${folders.join(", ")})`);
  for (const file of linkingFiles()) {
    const text = readFileSync(file, "utf8");
    for (const m of text.matchAll(linkPattern)) {
      const abs = resolve(dirname(file), m[2]);
      if (!m[2].includes("://") && abs.startsWith(ideasDir) && !existsSync(abs))
        errors.push(`${relative(root, file)}: broken link to ${m[2]}`);
    }
    for (const m of text.matchAll(rootPathPattern))
      if (!existsSync(join(root, m[0]))) errors.push(`${relative(root, file)}: broken path ${m[0]}`);
  }
  for (const e of errors) console.error(e);
  console.log(errors.length ? `${errors.length} problem(s).` : `${ideas.length} ideas, no problems.`);
  return errors.length ? 1 : 0;
}

function list(args: string[]) {
  const areaIndex = args.indexOf("--area");
  const area = areaIndex >= 0 ? args[areaIndex + 1] : undefined;
  const named = args.filter((_, n) => areaIndex < 0 || (n !== areaIndex && n !== areaIndex + 1));
  const ideas = allIdeas().filter((i) => !area || i.meta.area === area);
  const groups: [string, Idea[], boolean][] = [
    ["In progress", ideas.filter((i) => i.folder === "next" && i.meta.plan), true],
    ["Next", ideas.filter((i) => i.folder === "next" && !i.meta.plan), true],
    ["Someday", ideas.filter((i) => i.folder === "someday"), true],
    ["Parked", ideas.filter((i) => i.folder === "parked"), true],
    ["Done", ideas.filter((i) => i.folder === "done"), false],
  ];
  const wanted = (label: string) => {
    if (named.length === 0) return true;
    const folder = label === "In progress" ? "next" : label.toLowerCase();
    return named.includes(folder) || named.includes(label.toLowerCase().replace(" ", "-"));
  };
  for (const [label, group, openByDefault] of groups) {
    if (!wanted(label)) continue;
    const show = openByDefault || named.length > 0;
    console.log(`\n${label} (${group.length})${show ? "" : ": node scripts/ideas.ts done"}`);
    if (!show) continue;
    for (const i of group) {
      console.log(`  ${i.slug}  [${i.meta.area}]`);
      console.log(`      ${i.meta.description}`);
      if (i.meta.plan) console.log(`      plan: ${i.meta.plan}`);
    }
  }
}

function fail(message: string): never {
  console.error(message);
  process.exit(2);
}

const [command, ...rest] = process.argv.slice(2);
if (command === "move") {
  if (rest.length !== 2) fail("Usage: node scripts/ideas.ts move <slug> <folder>");
  move(rest[0], rest[1]);
} else if (command === "check") {
  process.exit(check());
} else {
  list(command ? [command, ...rest] : []);
}
