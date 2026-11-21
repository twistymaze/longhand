// A Claude Code PreToolUse hook that stops agent commands that break the
// shared rules of this repository. Each rule names the fix. rules/agent-*.toml
// records each rule as a mistake family with a negative example.
//
// Input: the hook JSON on stdin ({ tool_name, tool_input }).
// Deny: exit 2, the reason on stderr.
// Ask: exit 0, a permission decision "ask" on stdout.
// Each deny or ask adds one line to rules.log next to this file (gitignored),
// for the pilot review. An error in this script never blocks a command.
import { appendFileSync, existsSync, readFileSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const HOOKS = dirname(fileURLToPath(import.meta.url));
const LOG = join(HOOKS, "rules.log");

// The real path of a folder. A folder that is not there gets the real path
// of its nearest existing parent, with the rest of the path added. Symbolic
// links (macOS /tmp and /var) then compare equal.
function realFolder(path) {
  let head = path;
  const rest = [];
  while (!existsSync(head)) {
    const parent = dirname(head);
    if (parent === head) return path;
    rest.unshift(basename(head));
    head = parent;
  }
  return join(realpathSync(head), ...rest);
}

// The checkout root: two levels above .claude/hooks/.
const ROOT = realFolder(resolve(HOOKS, "..", ".."));

// True when the folder is in this checkout, or when the folder is not known.
function inCheckout(folder) {
  if (folder === undefined) return true;
  const rel = relative(ROOT, folder);
  return !(rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel));
}

// The folder after a `cd` segment, or undefined when it is not known: a
// variable, a command substitution or `cd -`.
function cdTarget(words, folder) {
  const arg = words[1];
  if (arg === undefined || arg === "~") return homedir();
  if (arg === "-" || arg.includes("$") || arg.includes("`")) return undefined;
  if (arg.startsWith("~/")) return realFolder(join(homedir(), arg.slice(2)));
  if (isAbsolute(arg)) return realFolder(arg);
  return folder === undefined ? undefined : realFolder(resolve(folder, arg));
}

// Split a shell command into segments at &&, ||, ;, | and line ends outside
// quotes, and each segment into words without their quotes. The body of a
// here-document is text, not commands, so it is not in a segment.
export function segments(command) {
  const result = [];
  let words = [];
  let word = null;
  let quote = null;
  const heredocs = [];
  const endWord = () => {
    if (word !== null) words.push(word);
    word = null;
  };
  const endSegment = () => {
    endWord();
    if (words.length > 0) result.push(words);
    words = [];
  };
  for (let i = 0; i < command.length; i++) {
    const c = command[i];
    if (quote) {
      if (c === quote) quote = null;
      else if (c === "\\" && quote === '"' && i + 1 < command.length) word += command[++i];
      else word += c;
    } else if (c === "'" || c === '"') {
      quote = c;
      word ??= "";
    } else if (c === "\\" && i + 1 < command.length) {
      word = (word ?? "") + command[++i];
    } else if (c === "<" && command[i + 1] === "<" && command[i + 2] !== "<") {
      // A here-document: read its delimiter. Its body starts on the next line.
      endWord();
      i += 2;
      const strip = command[i] === "-";
      if (strip) i++;
      while (command[i] === " " || command[i] === "\t") i++;
      let delimiter = "";
      while (i < command.length && !/[\s;&|<>()]/.test(command[i])) {
        const d = command[i];
        if (d === "'" || d === '"') {
          const end = command.indexOf(d, i + 1);
          delimiter += command.slice(i + 1, end === -1 ? command.length : end);
          i = end === -1 ? command.length : end + 1;
        } else {
          if (d !== "\\") delimiter += d;
          i++;
        }
      }
      i--;
      if (delimiter !== "") heredocs.push({ delimiter, strip });
    } else if (c === " " || c === "\t") endWord();
    else if (c === "\n" && heredocs.length > 0) {
      endSegment();
      // Skip each body up to the line that holds only its delimiter.
      for (const { delimiter, strip } of heredocs.splice(0)) {
        while (i < command.length) {
          const end = command.indexOf("\n", i + 1);
          const line = command.slice(i + 1, end === -1 ? command.length : end);
          i = end === -1 ? command.length : end;
          if ((strip ? line.replace(/^\t+/, "") : line) === delimiter) break;
        }
      }
    } else if (c === "\n" || c === ";" || c === "&" || c === "|") endSegment();
    else word = (word ?? "") + c;
  }
  endSegment();
  return result;
}

// The command word of a segment, after VAR=value words. For `npx <tool>`, the tool.
function commandWord(words) {
  let i = 0;
  while (i < words.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[i])) i++;
  const first = (words[i] ?? "").split("/").pop();
  if (first === "npx") return (words.slice(i + 1).find((w) => !w.startsWith("-")) ?? "").split("@")[0];
  return first;
}

// The value of an option such as --title, in "--title x" or "--title=x" form.
function option(words, ...names) {
  for (let i = 0; i < words.length; i++) {
    for (const name of names) {
      if (words[i] === name) return words[i + 1];
      if (words[i].startsWith(`${name}=`)) return words[i].slice(name.length + 1);
    }
  }
  return undefined;
}

const isGhPr = (w) => commandWord(w) === "gh" && w.includes("pr") && (w.includes("create") || w.includes("edit"));
const PRIVATE = /(^|\/)(sample\.m4a|sample\.txt|\.env)$|(^|\/)(fixtures|manual-tests)(\/|$)|-screenshots(\/|$)/;

// Each rule: its family, its action, a test on one segment (Bash) or on the
// input (other tools), and the message that names the fix.
const RULES = [
  // The deny rule comes first: a feat title with ! is denied, not asked.
  {
    family: "agent-release-title",
    action: "deny",
    bash: (w) => isGhPr(w) && /^[a-z]+(\([^)]*\))?\+?!:|BREAKING CHANGE/.test(option(w, "--title", "-t") ?? ""),
    message: "`!` or BREAKING CHANGE in a PR title makes a major release. Use it only when the user asks for a major release.",
  },
  {
    family: "agent-release-title",
    action: "ask",
    bash: (w) => isGhPr(w) && /^feat/.test(option(w, "--title", "-t") ?? ""),
    message: "A feat PR title: ask the user \"patch or minor?\" first, and wait for the answer (CLAUDE.md, Each PR title is a Conventional Commit).",
  },
  {
    family: "agent-pr-branding",
    action: "deny",
    bash: (w) => isGhPr(w) && /compound.engineering/i.test(option(w, "--body", "-b") ?? ""),
    message: "Do not add Compound Engineering branding or badges to PR descriptions or commit messages.",
  },
  {
    family: "agent-ci-rerun",
    action: "ask",
    bash: (w) => commandWord(w) === "gh" && w.includes("run") && w.includes("rerun"),
    message: "Run a CI job again only for an infrastructure fault. A failure that depends on timing is a defect in the test: find the race and fix the test in this PR.",
  },
  {
    family: "agent-ideas-move",
    action: "deny",
    bash: (w) => (commandWord(w) === "mv" || (commandWord(w) === "git" && w.includes("mv"))) && w.some((a) => a.includes("docs/ideas/")),
    message: "Move an idea only with `node scripts/ideas.ts move <slug> <folder>`: it also changes every link to the idea.",
  },
  {
    family: "agent-reference-source",
    action: "deny",
    edit: (path) => /(^|\/)reference-source\//.test(path),
    message: "reference-source/ holds read-only clones. Do not edit them; record borrowed code in PROVENANCE.md.",
  },
  {
    family: "agent-private-paths",
    action: "deny",
    bash: (w) => commandWord(w) === "git" && w.includes("add") && (w.includes("-f") || w.includes("--force")) && w.some((a) => PRIVATE.test(a)),
    message: "This path is private and gitignored (sample files, fixtures/, manual-tests/, .env, screenshots). Never commit, print or upload it.",
  },
  {
    family: "agent-tool-choice",
    action: "deny",
    bash: (w) => ["pip", "pip3", "python", "python3"].includes(commandWord(w)),
    message: "Use Python only through uv: `uv run ...`.",
  },
  {
    family: "agent-tool-choice",
    action: "deny",
    // Only this checkout uses npm. A command that runs in another
    // repository is allowed; a command whose folder is not known is not.
    bash: (w, folder) => ["bun", "bunx", "yarn", "pnpm"].includes(commandWord(w)) && inCheckout(folder),
    message: "Use JavaScript packages only through npm in this checkout.",
  },
  {
    family: "agent-tool-choice",
    action: "deny",
    bash: (w) => commandWord(w) === "prettier",
    message: "Longhand has no JavaScript formatter, and prettier rewrites whole files. The checks are `npm run typecheck` and `npm test`.",
  },
  {
    family: "agent-test-build-location",
    action: "deny",
    bash: (w) => ["cp", "ditto", "unzip", "mv"].includes(commandWord(w)) && w.some((a) => a.startsWith("/Applications")),
    message: "Never put a test build in /Applications. Unzip it to ~/Downloads/longhand-test (docs/development-workflow.md, Manual tests).",
  },
];

// The first rule that matches the hook input, or undefined.
export function decide(input) {
  const tool = input?.tool_name;
  const toolInput = input?.tool_input ?? {};
  if (tool === "Bash" && typeof toolInput.command === "string") {
    // The folder that each segment runs in: the hook input's cwd, then each
    // cd segment.
    let folder = typeof input.cwd === "string" && isAbsolute(input.cwd) ? realFolder(input.cwd) : undefined;
    for (const words of segments(toolInput.command)) {
      const rule = RULES.find((r) => r.bash?.(words, folder));
      if (rule) return rule;
      if (commandWord(words) === "cd") folder = cdTarget(words, folder);
    }
  } else if ((tool === "Edit" || tool === "Write" || tool === "MultiEdit") && typeof toolInput.file_path === "string") {
    return RULES.find((r) => r.edit?.(toolInput.file_path));
  }
  return undefined;
}

function main() {
  let rule;
  try {
    rule = decide(JSON.parse(readFileSync(0, "utf8")));
  } catch (e) {
    process.stderr.write(`rules hook: ${e.message}; the command is not checked\n`);
    return 0;
  }
  if (!rule) return 0;
  try {
    appendFileSync(LOG, `${new Date().toISOString()}\t${rule.family}\t${rule.action}\n`);
  } catch {
    // The log is for the pilot review only.
  }
  if (rule.action === "deny") {
    process.stderr.write(`${rule.family}: ${rule.message}\n`);
    return 2;
  }
  process.stdout.write(
    JSON.stringify({
      hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "ask", permissionDecisionReason: `${rule.family}: ${rule.message}` },
    }),
  );
  return 0;
}

if (import.meta.main) process.exitCode = main();
