// List, move and check the feature ideas in docs/ideas/.
//
// The folder of an idea is its status. The frontmatter holds a one-line
// description, so this script reads only the frontmatter. docs/ideas/README.md
// describes the rules.
//
//   bun scripts/ideas.ts [folder...] [--area <area>]   list ideas (Done shows only a count unless you name it)
//   bun scripts/ideas.ts move <slug> <folder>           git mv the idea and change every link to it
//   bun scripts/ideas.ts check                          find frontmatter errors and broken links
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, normalize, relative, resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
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
  const meta = match ? ((Bun.YAML.parse(match[1]) as Record<string, unknown>) ?? {}) : {};
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
  const git = Bun.spawnSync(["git", "mv", idea.path, to], { cwd: root, stderr: "pipe" });
  if (git.exitCode !== 0) fail(`git mv failed: ${git.stderr.toString()}`);
  let changed = 0;
  for (const file of linkingFiles()) {
    const before = file === to ? idea.path : file;
    const text = readFileSync(file, "utf8");
    const next = rewriteLinks(text, file, before, idea.path, to);
    if (next !== text) {
      writeFileSync(file, next);
      changed++;
    }
  }
  console.log(`Moved ${slug} to ${folder}/. Changed links in ${changed} file(s).`);
  if (folder === "parked" && !/^## Why parked/m.test(readFileSync(to, "utf8")))
    console.log(`Add a "## Why parked" section to ${relative(root, to)}.`);
}

// The branch names of the PRs that merged into main. A merge commit subject
// is "Merge pull request #N from <owner>/<branch>".
function mergedBranches(): Set<string> {
  const ref = Bun.spawnSync(["git", "rev-parse", "--verify", "--quiet", "origin/main"], { cwd: root }).exitCode === 0 ? "origin/main" : "HEAD";
  const git = Bun.spawnSync(["git", "log", ref, "--merges", "--format=%s"], { cwd: root, stderr: "pipe" });
  const branches = new Set<string>();
  for (const line of git.stdout.toString().split("\n")) {
    const m = /^Merge pull request #\d+ from [^/]+\/(.+)$/.exec(line.trim());
    if (m) branches.add(m[1]);
  }
  return branches;
}

// An idea in next/ whose plan topic has a merged branch has shipped and must
// move to done/. Plan branches are named worktree-<topic> or codex/<topic>.
function shippedButOpen(ideas: Idea[]): string[] {
  const open = ideas.filter((i) => i.folder !== "done" && i.meta.plan);
  if (open.length === 0) return [];
  const branches = mergedBranches();
  const errors: string[] = [];
  for (const i of open) {
    const planPath = join(root, String(i.meta.plan));
    if (!existsSync(planPath)) continue;
    const topic = /^topic:\s*(\S+)\s*$/m.exec(readFileSync(planPath, "utf8"))?.[1];
    if (!topic) continue;
    const branch = [`worktree-${topic}`, `codex/${topic}`].find((b) => branches.has(b));
    if (branch)
      errors.push(`${relative(root, i.path)}: branch ${branch} of its plan merged; move it with "bun scripts/ideas.ts move ${i.slug} done" and write the PR in its file`);
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
    console.log(`\n${label} (${group.length})${show ? "" : ": bun scripts/ideas.ts done"}`);
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
  if (rest.length !== 2) fail("Usage: bun scripts/ideas.ts move <slug> <folder>");
  move(rest[0], rest[1]);
} else if (command === "check") {
  process.exit(check());
} else {
  list(command ? [command, ...rest] : []);
}
