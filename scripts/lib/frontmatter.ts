// A reader for the YAML frontmatter of docs/ideas, with no npm package.
//
// The reader accepts only the subset that the idea files use: one
// `key: value` line for each key, where the value is plain text, a
// double-quoted string, or a flow list of plain items such as `[a, b]`.
// Every value stays a string, so a date such as `added: 2026-10-18` is not
// changed to a date. A comment line and an empty line are accepted. Any other
// syntax, for example a nested mapping, a block list or a single-quoted
// string, is an error with the line number.

export type Frontmatter = Record<string, string | string[]>;

const LINE = /^([A-Za-z0-9_-]+):(?:[ \t]+(.*))?$/;
const ESCAPES: Record<string, string> = { '"': '"', "\\": "\\", "/": "/", n: "\n", t: "\t", r: "\r", "0": "\0" };
// A plain value cannot start with one of these characters in YAML.
const INDICATOR = /^[-?:,[\]{}#&*!|>'"%@`]/;

function plain(text: string, fail: (message: string) => never): string {
  if (text === "") fail("an empty value is not supported");
  if (INDICATOR.test(text)) fail(`the value "${text}" starts with a YAML indicator; put it in double quotes`);
  if (/: |:$/.test(text)) fail(`the value "${text}" contains ": "; put it in double quotes`);
  return text;
}

function quoted(text: string, fail: (message: string) => never): string {
  let out = "";
  let i = 1;
  for (;;) {
    if (i >= text.length) fail("the string has no closing quote");
    const ch = text[i];
    if (ch === '"') break;
    if (ch === "\\") {
      const next = text[i + 1];
      if (!(next in ESCAPES)) fail(`the escape \\${next ?? ""} is not supported`);
      out += ESCAPES[next];
      i += 2;
      continue;
    }
    out += ch;
    i++;
  }
  const after = text.slice(i + 1);
  if (after.trim() !== "" && !/^\s+#/.test(after)) fail(`unexpected text "${after}" after the string`);
  return out;
}

// Remove a comment: a # after a space.
function uncomment(text: string): string {
  return text.replace(/\s+#.*$/, "").trim();
}

/// The frontmatter text (the lines between the --- lines) as an object.
export function parseFrontmatter(text: string): Frontmatter {
  const meta: Frontmatter = {};
  text.split(/\r?\n/).forEach((line, index) => {
    const fail = (message: string): never => {
      throw new Error(`frontmatter line ${index + 1}: ${message}`);
    };
    if (line.trim() === "" || line.trimStart().startsWith("#")) return;
    if (/^\s/.test(line)) fail("an indented line (a nested value) is not supported");
    const match = LINE.exec(line.trimEnd());
    if (!match) return fail(`expected "key: value", found "${line}"`);
    const [, key, raw = ""] = match;
    if (Object.hasOwn(meta, key)) fail(`the key "${key}" is already defined`);
    if (raw === "" || raw.startsWith("#")) fail(`the key "${key}" has no value; a nested mapping or a block list is not supported`);
    let value: string | string[];
    if (raw.startsWith('"')) {
      value = quoted(raw, fail);
    } else if (raw.startsWith("[")) {
      const list = uncomment(raw);
      if (!list.endsWith("]")) fail(`the list "${raw}" has no closing ]`);
      const inner = list.slice(1, -1).trim();
      value = inner === "" ? [] : inner.split(",").map((item) => plain(item.trim(), fail));
    } else {
      value = plain(uncomment(raw), fail);
    }
    Object.defineProperty(meta, key, { value, enumerable: true, writable: true, configurable: true });
  });
  return meta;
}
