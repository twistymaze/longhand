// Find short real-time limits in tests.
//
// A test must wait for the event that it checks. A time limit only stops a
// test that hangs, and a hang limit is 60 s. A shorter limit makes the test
// fail on a loaded runner. The scan finds five forms of a short limit:
//
// - Rust: an `elapsed()` upper bound under 60 s, for example
//   `assert!(t.elapsed() < Duration::from_secs(2))`.
// - Rust: a `recv_timeout` under 60 s that is not inside a loop. A loop that
//   polls with a short wait and has its own outer limit is correct.
// - Rust: a `HANG_LIMIT` constant other than 60 s.
// - TypeScript: a `waitFor` (or `findBy`) timeout under 60 000 ms.
// - TypeScript: a real `setTimeout(resolve, n)` sleep.
//
// Rust test code is a file in a `tests` folder, a file named `tests.rs`, and
// the body of a `mod` that comes directly after a `#[cfg(test)]` line. Other
// `#[cfg(test)]` items (a field, a helper function) are not test code. The scan
// removes comments and the text of string literals before it reads Rust code.
// A duration is a literal `Duration::from_*` call, or a `const` of the same
// file that holds one. The scan does not check other durations.
//
// The exception list below holds the known hits. Each entry names a file, a
// form, a count and a reason. The list can only get shorter: the scan fails on
// a hit that has no entry, and on an entry that the scan no longer finds.
//
//   node scripts/check-timing.ts    scan the tests and compare with the exception list
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";

const root = resolve(import.meta.dirname, "..");
const HANG_MS = 60_000;

type Form = "elapsed" | "recv_timeout" | "HANG_LIMIT" | "waitFor" | "setTimeout";
type Hit = { file: string; line: number; form: Form; code: string };
type Exception = { file: string; form: Form; count: number; reason: string };

const labels: Record<Form, string> = {
  elapsed: "an elapsed() upper bound under 60 s",
  recv_timeout: "a recv_timeout under 60 s outside a loop",
  HANG_LIMIT: "a HANG_LIMIT other than 60 s",
  waitFor: "a waitFor timeout under 60000 ms",
  setTimeout: "a real setTimeout sleep",
};

const exceptions: Exception[] = [
  {
    file: "src-tauri/src/managers/capture.rs",
    form: "elapsed",
    count: 1,
    reason: "known defect: a_stop_cuts_the_backoff_short requires that a stop ends the wait in under 1 s",
  },
  {
    file: "crates/summarize/tests/models.rs",
    form: "elapsed",
    count: 1,
    reason: "the 30 s bound is below the 60 s deadlines of the client, so only the 300 ms response deadline can end the request in time",
  },
  {
    file: "crates/pipeline/tests/type_move.rs",
    form: "recv_timeout",
    count: 1,
    reason: "a hang limit of 30 s: the move never ends while it waits on the blocked read, so only a failure reaches the limit",
  },
  {
    file: "src-tauri/src/managers/settings.rs",
    form: "recv_timeout",
    count: 1,
    reason: "a hang limit of 30 s: the local change never ends while it waits for the held shared save, so only a failure reaches the limit",
  },
  {
    file: "src/update/UpdateWindow.test.tsx",
    form: "setTimeout",
    count: 2,
    reason: "a 0 ms timer lets the queued promise callbacks run before the check; it does not wait for a time",
  },
  {
    file: "src/session/SessionPage.test.tsx",
    form: "setTimeout",
    count: 1,
    reason: "the 1300 ms timer runs on the fake clock (vi.useFakeTimers), so it does not wait real time",
  },
];

const skipDirs = new Set(["target", "node_modules", "gen", "dist"]);

function walk(dir: string, keep: (path: string) => boolean, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir).sort()) {
    if (name.startsWith(".") || skipDirs.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, keep, out);
    else if (keep(p)) out.push(p);
  }
  return out;
}

// Replace comments and the text in string and char literals with spaces. Keep
// each newline, so that line numbers do not change.
function stripRust(text: string): string {
  let out = "";
  let i = 0;
  const blank = (s: string) => s.replace(/[^\n]/g, " ");
  const rawString = /b?r(#*)"/y;
  const charLiteral = /'(?:\\u\{[0-9a-fA-F]+\}|\\.|[^\\'\n])'/y;
  const at = (re: RegExp) => ((re.lastIndex = i), re.exec(text));
  while (i < text.length) {
    let m: RegExpExecArray | null;
    if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      const stop = end === -1 ? text.length : end;
      out += blank(text.slice(i, stop));
      i = stop;
    } else if (text.startsWith("/*", i)) {
      let depth = 0;
      let j = i;
      while (j < text.length) {
        if (text.startsWith("/*", j)) (depth++, (j += 2));
        else if (text.startsWith("*/", j)) {
          depth--;
          j += 2;
          if (depth === 0) break;
        } else j++;
      }
      out += blank(text.slice(i, j));
      i = j;
    } else if ((text[i] === "r" || text[i] === "b") && (m = at(rawString)) && !/\w/.test(text[i - 1] ?? "")) {
      const close = `"${m[1]}`;
      const end = text.indexOf(close, i + m[0].length);
      const stop = end === -1 ? text.length : end + close.length;
      out += `""${blank(text.slice(i + 2, stop))}`;
      i = stop;
    } else if (text[i] === '"' || (text[i] === "b" && text[i + 1] === '"' && !/\w/.test(text[i - 1] ?? ""))) {
      let j = text.indexOf('"', i) + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      out += `""${blank(text.slice(i + 2, j + 1))}`;
      i = j + 1;
    } else if (text[i] === "'" && (m = at(charLiteral))) {
      out += `''${blank(m[0].slice(2))}`;
      i += m[0].length;
    } else {
      out += text[i];
      i++;
    }
  }
  return out;
}

const durationPattern = /^(?:std::time::)?Duration::from_(secs|millis|micros|nanos|mins|secs_f32|secs_f64)\(\s*([\d_]+(?:\.[\d_]+)?)\s*\)/;
const unitMs: Record<string, number> = { secs: 1000, millis: 1, micros: 0.001, nanos: 0.000001, mins: 60_000, secs_f32: 1000, secs_f64: 1000 };

function durationMs(expr: string, consts: Map<string, number>): number | undefined {
  const text = expr.trim();
  const m = durationPattern.exec(text);
  if (m) return Number(m[2].replace(/_/g, "")) * unitMs[m[1]];
  const name = /^([A-Za-z_]\w*)\b(?!\s*[(:.])/.exec(text);
  return name ? consts.get(name[1]) : undefined;
}

type Frame = { loop: boolean; test: boolean };

function scanRust(file: string, text: string, wholeFileIsTest: boolean): Hit[] {
  const lines = text.split("\n");
  const code = stripRust(text).split("\n");
  const consts = new Map<string, number>();
  for (const line of code) {
    const m = /\bconst\s+([A-Za-z_]\w*)\s*:\s*(?:std::time::)?Duration\s*=\s*(.+?);/.exec(line);
    if (!m) continue;
    const ms = durationMs(m[2], consts);
    if (ms !== undefined) consts.set(m[1], ms);
  }
  const hits: Hit[] = [];
  const stack: Frame[] = [];
  let statement = "";
  let testAttribute = false;
  for (let n = 0; n < code.length; n++) {
    const line = code[n];
    let inLoop = stack.some((f) => f.loop);
    let inTest = wholeFileIsTest || stack.some((f) => f.test);
    if (/#\[cfg\(test\)\]/.test(line)) testAttribute = true;
    for (const ch of line) {
      if (ch === "{") {
        const before = statement.trim();
        const frame = {
          loop: /(?:^|\W)loop$/.test(before) || /^(?:'\w+\s*:\s*)?(?:while|for)\b/.test(before),
          test: testAttribute && /\bmod\s+\w+$/.test(before),
        };
        stack.push(frame);
        inLoop ||= frame.loop;
        inTest ||= frame.test;
        statement = "";
        testAttribute = false;
      } else if (ch === "}" || ch === ";") {
        if (ch === "}") stack.pop();
        statement = "";
        testAttribute = false;
      } else statement += ch;
    }
    statement += " ";
    if (!inTest) continue;
    const hit = (form: Form) => hits.push({ file, line: n + 1, form, code: lines[n].trim() });
    const elapsed = /\.elapsed\(\)\s*<=?(?!<)(.*)$/.exec(line);
    if (elapsed && !/^\s*while\b/.test(line)) {
      const ms = durationMs(elapsed[1], consts);
      if (ms !== undefined && ms < HANG_MS) hit("elapsed");
    }
    const recv = /\.recv_timeout\((.*)$/.exec(line);
    if (recv && !inLoop) {
      const ms = durationMs(`${recv[1]} ${code[n + 1] ?? ""}`, consts);
      if (ms !== undefined && ms < HANG_MS) hit("recv_timeout");
    }
    const limit = /\bconst\s+HANG_LIMIT\s*:\s*(?:std::time::)?Duration\s*=\s*(.+?);/.exec(line);
    if (limit) {
      const ms = durationMs(limit[1], consts);
      if (ms !== undefined && ms !== HANG_MS) hit("HANG_LIMIT");
    }
  }
  return hits;
}

function lineOf(text: string, index: number): number {
  return text.slice(0, index).split("\n").length;
}

function scanFrontend(file: string, text: string): Hit[] {
  const lines = text.split("\n");
  const hits: Hit[] = [];
  const hit = (form: Form, index: number) => {
    const line = lineOf(text, index);
    hits.push({ file, line, form, code: lines[line - 1].trim() });
  };
  for (const call of text.matchAll(/\b(?:waitFor\w*|find(?:All)?By\w+)\(/g)) {
    let depth = 0;
    let end = call.index + call[0].length - 1;
    for (; end < text.length; end++) {
      if (text[end] === "(") depth++;
      else if (text[end] === ")" && --depth === 0) break;
    }
    const span = text.slice(call.index, end);
    for (const t of span.matchAll(/\btimeout\s*:\s*([\d_]+)\b/g)) {
      if (Number(t[1].replace(/_/g, "")) < HANG_MS) hit("waitFor", call.index + t.index);
    }
  }
  for (const sleep of text.matchAll(/\bsetTimeout\(\s*[A-Za-z_$][\w$]*\s*[,)]/g)) hit("setTimeout", sleep.index);
  return hits;
}

function scan(): Hit[] {
  const rel = (p: string) => relative(root, p).split(sep).join("/");
  const hits: Hit[] = [];
  const rust = [...walk(join(root, "crates"), (p) => p.endsWith(".rs")), ...walk(join(root, "src-tauri"), (p) => p.endsWith(".rs"))];
  for (const path of rust) {
    const file = rel(path);
    const parts = file.split("/");
    const wholeFileIsTest = parts.slice(0, -1).includes("tests") || parts[parts.length - 1] === "tests.rs";
    hits.push(...scanRust(file, readFileSync(path, "utf8"), wholeFileIsTest));
  }
  for (const path of walk(join(root, "src"), (p) => /\.test\.tsx?$/.test(p))) {
    hits.push(...scanFrontend(rel(path), readFileSync(path, "utf8")));
  }
  return hits;
}

function main() {
  const hits = scan();
  const problems: string[] = [];
  const groups = new Map<string, Hit[]>();
  for (const h of hits) {
    const key = `${h.file}\0${h.form}`;
    groups.set(key, [...(groups.get(key) ?? []), h]);
  }
  for (const [key, group] of groups) {
    const [file, form] = key.split("\0") as [string, Form];
    const listed = exceptions.find((e) => e.file === file && e.form === form)?.count ?? 0;
    if (group.length <= listed) continue;
    for (const h of group) problems.push(`${h.file}:${h.line}: short time limit in a test (${labels[h.form]}): ${h.code}`);
    if (listed > 0) problems.push(`${file}: the scan found ${group.length} of this form, and the exception list holds ${listed}.`);
  }
  for (const e of exceptions) {
    const found = groups.get(`${e.file}\0${e.form}`)?.length ?? 0;
    if (found === 0) problems.push(`${e.file}: the scan found no hit of this form (${labels[e.form]}): remove the exception.`);
    else if (found < e.count) problems.push(`${e.file}: the scan found ${found} hits of this form (${labels[e.form]}), and the exception lists ${e.count}: lower the count.`);
  }
  if (problems.length === 0) {
    console.log(`check-timing: no new short time limits in tests (${exceptions.length} listed exceptions).`);
    return;
  }
  for (const p of problems) console.error(p);
  console.error(`
A test must wait for the event that it checks, not for a delay, a poll count or a lucky order of threads (CLAUDE.md).
A time limit only stops a hang, and a hang limit is 60 s. See docs/solutions/test-failures/tests-with-speed-limits-failed-on-a-loaded-runner.md.
Change the test to wait for the event. The exception list in scripts/check-timing.ts can only get shorter.`);
  process.exit(1);
}

main();
