// A TOML reader for Cargo.lock and min-age-exceptions.toml, with no npm package.
//
// The reader accepts comments, `[table]`, `[[array-of-tables]]`, and
// `key = value` lines with a bare key. A value is a basic string, an integer,
// a boolean, or an array of basic strings. An array can continue on more
// lines. Any other syntax, for example an inline table, a multi-line string,
// a literal string or a dotted key, is an error with the line number.

export type TomlValue = string | number | boolean | string[] | TomlTable | TomlTable[];
export type TomlTable = { [key: string]: TomlValue };

const KEY = /^[A-Za-z0-9_-]+$/;
const ESCAPES: Record<string, string> = { '"': '"', "\\": "\\", b: "\b", t: "\t", n: "\n", f: "\f", r: "\r" };

class Cursor {
  lines: string[];
  line = 0;
  column = 0;

  constructor(text: string) {
    this.lines = text.replace(/\r\n/g, "\n").split("\n");
  }

  fail(message: string): never {
    throw new Error(`TOML line ${this.line + 1}: ${message}`);
  }

  rest(): string {
    return this.lines[this.line].slice(this.column);
  }

  skipSpace() {
    while (this.rest().startsWith(" ") || this.rest().startsWith("\t")) this.column++;
  }

  // Skip spaces, comments and line ends inside an array.
  skipSpaceAndLines() {
    for (;;) {
      this.skipSpace();
      const rest = this.rest();
      if (rest === "" || rest.startsWith("#")) {
        if (this.line + 1 >= this.lines.length) this.fail("the array has no closing ]");
        this.line++;
        this.column = 0;
      } else return;
    }
  }

  // The end of a line: only spaces and an optional comment.
  expectEnd() {
    this.skipSpace();
    const rest = this.rest();
    if (rest !== "" && !rest.startsWith("#")) this.fail(`unexpected text "${rest}"`);
  }
}

function readString(c: Cursor): string {
  const rest = c.rest();
  if (rest.startsWith('"""')) c.fail("a multi-line string is not supported");
  if (!rest.startsWith('"')) c.fail(`expected a basic string, found "${rest}"`);
  let out = "";
  let i = 1;
  for (;;) {
    if (i >= rest.length) c.fail("the string has no closing quote");
    const ch = rest[i];
    if (ch === '"') break;
    if (ch === "\\") {
      const next = rest[i + 1];
      if (next === "u" || next === "U") {
        const size = next === "u" ? 4 : 8;
        const hex = rest.slice(i + 2, i + 2 + size);
        if (!new RegExp(`^[0-9A-Fa-f]{${size}}$`).test(hex)) c.fail(`the escape \\${next}${hex} is not valid`);
        out += String.fromCodePoint(parseInt(hex, 16));
        i += 2 + size;
        continue;
      }
      if (!(next in ESCAPES)) c.fail(`the escape \\${next ?? ""} is not supported`);
      out += ESCAPES[next];
      i += 2;
      continue;
    }
    out += ch;
    i++;
  }
  c.column += i + 1;
  return out;
}

function readValue(c: Cursor): TomlValue {
  const rest = c.rest();
  if (rest.startsWith('"')) return readString(c);
  if (rest.startsWith("[")) {
    c.column++;
    const items: string[] = [];
    for (;;) {
      c.skipSpaceAndLines();
      if (c.rest().startsWith("]")) break;
      items.push(readString(c));
      c.skipSpaceAndLines();
      if (c.rest().startsWith(",")) c.column++;
      else if (!c.rest().startsWith("]")) c.fail(`expected , or ] in the array, found "${c.rest()}"`);
    }
    c.column++;
    return items;
  }
  const word = /^[^\s#]+/.exec(rest)?.[0] ?? "";
  if (word === "true" || word === "false") {
    c.column += word.length;
    return word === "true";
  }
  if (/^[+-]?(0|[1-9]\d*)$/.test(word)) {
    c.column += word.length;
    return Number(word);
  }
  if (rest.startsWith("{")) c.fail("an inline table is not supported");
  if (rest.startsWith("'")) c.fail("a literal string is not supported");
  return c.fail(`the value "${rest}" is not a basic string, an integer, a boolean or an array of basic strings`);
}

/// The TOML document as plain objects and arrays.
export function parseToml(text: string): TomlTable {
  const root: TomlTable = {};
  let table = root;
  const c = new Cursor(text);
  for (; c.line < c.lines.length; c.line++) {
    c.column = 0;
    c.skipSpace();
    const rest = c.rest();
    if (rest === "" || rest.startsWith("#")) continue;
    const header = /^(\[\[?)\s*([^\]\s]*)\s*(\]\]?)/.exec(rest);
    if (header) {
      const [all, open, name, close] = header;
      if (open.length !== close.length) c.fail(`the header "${all}" is not valid`);
      if (!KEY.test(name)) c.fail(`the table name "${name}" is not a bare key`);
      c.column += all.length;
      c.expectEnd();
      const existing = Object.hasOwn(root, name) ? root[name] : undefined;
      if (open === "[[") {
        if (existing !== undefined && !(Array.isArray(existing) && existing.length > 0 && existing.every((e) => typeof e === "object"))) {
          c.fail(`"${name}" is already defined`);
        }
        table = {};
        set(root, name, [...((existing as TomlTable[] | undefined) ?? []), table]);
      } else {
        if (existing !== undefined) c.fail(`"${name}" is already defined`);
        table = {};
        set(root, name, table);
      }
      continue;
    }
    const assignment = /^([^=\s]+)\s*=\s*/.exec(rest);
    if (!assignment) return c.fail(`expected "key = value", a [table] or an [[array-of-tables]], found "${rest}"`);
    const key = assignment[1];
    if (!KEY.test(key)) c.fail(`the key "${key}" is not a bare key`);
    if (Object.hasOwn(table, key)) c.fail(`the key "${key}" is already defined`);
    c.column += assignment[0].length;
    const value = readValue(c);
    c.expectEnd();
    set(table, key, value);
  }
  return root;
}

// An own property, also for the key "__proto__".
function set(table: TomlTable, key: string, value: TomlValue) {
  Object.defineProperty(table, key, { value, enumerable: true, writable: true, configurable: true });
}
