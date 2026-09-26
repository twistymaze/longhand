// Pure grouping and formatting for the full licence list (KTD5).
//
// Each input is one licence text of one component. The output prints each
// distinct licence body once, after the lines of the components that use it.
// Each component line has the name, the version and the component's own
// copyright lines. The output does not depend on the input order.

export type Part = "rust" | "interface" | "native";

/** The titles of the three parts, in output order. The full list window reads them as headings. */
export const PART_TITLES = ["Rust crates", "Interface packages", "Native libraries"] as const;

const PARTS: Part[] = ["rust", "interface", "native"];

export interface LicensedComponent {
  part: Part;
  name: string;
  version: string;
  /** A short note after the version, for example the native code that a crate includes. */
  note?: string;
  text: string;
}

/** One entry of licences/native/manifest.json. */
export interface NativeEntry {
  name: string;
  version: string;
  part: Part;
  platforms: string[];
  /** Licence and notice files, relative to the repository root. */
  files: string[];
  source: string;
  note?: string;
  /** Copyright lines for a licence file that has none, such as a plain Apache-2.0 text. */
  copyright?: string[];
}

const RULE = "-".repeat(72);

/** Turns CRLF into LF, removes trailing spaces and the blank lines at each end. */
function normalizeLines(text: string): string[] {
  const lines = text
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.replace(/\s+$/, ""));
  while (lines.length > 0 && lines[0] === "") lines.shift();
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

function isCopyright(line: string): boolean {
  return /^(copyright\b|\(c\)|©)/i.test(line);
}

// Template placeholders, as in the canonical licence texts: "<year>",
// "<copyright holders>", "[yyyy]".
function isPlaceholder(line: string): boolean {
  return /<[^>]*>|\[yyyy\]|\[name of copyright owner\]|\{yyyy\}/i.test(line);
}

// A short line at the top of a licence, such as "MIT License" or
// "Version 2.0, January 2004".
function isTitle(line: string): boolean {
  return line.length <= 60 && !/[.:;,]$/.test(line);
}

interface Analysis {
  copyright: string[];
  body: string;
  /** The body without its title lines and with all whitespace collapsed. */
  key: string;
}

function analyse(text: string): Analysis {
  const lines = normalizeLines(text);
  const copyright: string[] = [];
  const kept: string[] = [];
  const titles = new Set<number>();
  let titleParagraphs = 0;
  let i = 0;
  // Read the header paragraph by paragraph. It ends at the first paragraph
  // that is neither copyright lines nor title lines.
  header: while (i < lines.length) {
    if (lines[i] === "") {
      kept.push("");
      i++;
      continue;
    }
    let end = i;
    while (end < lines.length && lines[end] !== "") end++;
    const paragraph = lines.slice(i, end).map((line) => line.trim());
    const first = paragraph[0];
    if (isCopyright(first) || (copyright.length > 0 && /^all rights reserved\.?$/i.test(first))) {
      while (i < end) {
        const line = lines[i].trim();
        if (isCopyright(line)) {
          let joined = line;
          i++;
          while (i < end && /^\s+\S/.test(lines[i]) && !isCopyright(lines[i].trim())) {
            joined += " " + lines[i].trim();
            i++;
          }
          if (!isPlaceholder(joined)) copyright.push(joined.replace(/\s+/g, " "));
        } else if (/^all rights reserved\.?$/i.test(line) && copyright.length > 0) {
          copyright[copyright.length - 1] += " " + line;
          i++;
        } else {
          // Body text directly under the copyright lines, with no blank line.
          break header;
        }
      }
    } else if (titleParagraphs < 3 && paragraph.every(isTitle)) {
      titleParagraphs++;
      for (; i < end; i++) {
        titles.add(kept.length);
        kept.push(lines[i]);
      }
    } else {
      break;
    }
  }
  const bodyLines = [...kept, ...lines.slice(i)];
  const collapsed: string[] = [];
  const keyLines: string[] = [];
  bodyLines.forEach((line, index) => {
    if (line === "" && (collapsed.length === 0 || collapsed[collapsed.length - 1] === "")) return;
    collapsed.push(line);
    if (!titles.has(index)) keyLines.push(line);
  });
  while (collapsed.length > 0 && collapsed[collapsed.length - 1] === "") collapsed.pop();
  const body = collapsed.join("\n");
  const key = keyLines.join(" ").replace(/\s+/g, " ").trim() || body.replace(/\s+/g, " ").trim();
  return { copyright, body, key };
}

/**
 * Gives the copyright lines for a licence text. When the text has none, as the
 * standard text has that cargo-about uses when it does not read a crate's own
 * file (for example Tauri's `LICENSE_MIT`), the lines come from the crate's
 * licence files that have the same body.
 */
export function copyrightFromFiles(text: string, files: string[]): string[] {
  const own = analyse(text);
  if (own.copyright.length > 0) return own.copyright;
  const lines = new Set<string>();
  for (const file of files) {
    const other = analyse(file);
    if (other.key === own.key) for (const line of other.copyright) lines.add(line);
  }
  return [...lines].sort(compare);
}

/** Separates the leading copyright lines of a licence text from its body. */
export function splitCopyright(text: string): { copyright: string[]; body: string } {
  const { copyright, body } = analyse(text);
  return { copyright, body };
}

/** Code-point order, so the output is the same on every host and locale. */
export function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

interface Group {
  bodies: Map<string, number>;
  components: Map<string, { line: string; sortKey: string[]; copyright: Set<string> }>;
}

function componentLine(c: LicensedComponent): string {
  const label = c.version ? `${c.name} ${c.version}` : c.name;
  return c.note ? `${label} (${c.note})` : label;
}

/** The most frequent body text of a group; code-point order breaks a tie. */
function representative(group: Group): string {
  return [...group.bodies.entries()].sort((a, b) => b[1] - a[1] || compare(a[0], b[0]))[0][0];
}

function renderPart(components: LicensedComponent[]): string[] {
  const groups = new Map<string, Group>();
  for (const component of components) {
    const { copyright, body, key } = analyse(component.text);
    let group = groups.get(key);
    if (!group) {
      group = { bodies: new Map(), components: new Map() };
      groups.set(key, group);
    }
    group.bodies.set(body, (group.bodies.get(body) ?? 0) + 1);
    const line = componentLine(component);
    let entry = group.components.get(line);
    if (!entry) {
      entry = { line, sortKey: [component.name, component.version, component.note ?? ""], copyright: new Set() };
      group.components.set(line, entry);
    }
    for (const c of copyright) entry.copyright.add(c);
  }

  const rendered = [...groups.values()]
    .map((group) => ({ body: representative(group), group }))
    .sort((a, b) => compare(a.body, b.body));

  const out: string[] = [];
  for (const { body, group } of rendered) {
    out.push(RULE, "");
    const entries = [...group.components.values()].sort((a, b) => {
      for (let k = 0; k < a.sortKey.length; k++) {
        const order = compare(a.sortKey[k], b.sortKey[k]);
        if (order !== 0) return order;
      }
      return 0;
    });
    for (const entry of entries) {
      out.push(entry.line);
      for (const c of [...entry.copyright].sort(compare)) out.push(`    ${c}`);
    }
    out.push("", body, "");
  }
  return out;
}

/**
 * Renders the full list: an introduction line, then the three parts, each
 * with its title on a line of its own.
 */
export function renderList(product: string, components: LicensedComponent[]): string {
  const out: string[] = [
    `${product} includes the components below. Each licence text appears once, after the components that use it.`,
    "",
  ];
  PARTS.forEach((part, index) => {
    out.push(PART_TITLES[index], "");
    const inPart = components.filter((c) => c.part === part);
    if (inPart.length === 0) out.push("None.", "");
    else out.push(...renderPart(inPart));
  });
  while (out.length > 0 && out[out.length - 1] === "") out.pop();
  return out.join("\n") + "\n";
}

/** The hand-kept entries for one platform, with their texts read through `read`. */
export function selectNative(
  entries: NativeEntry[],
  platform: string,
  read: (path: string) => string,
): LicensedComponent[] {
  return entries
    .filter((entry) => entry.platforms.includes(platform))
    .flatMap((entry) =>
      entry.files.map((file) => ({
        part: entry.part,
        name: entry.name,
        version: entry.version,
        note: entry.note,
        text: [...(entry.copyright ?? []), "", read(file)].join("\n"),
      })),
    );
}

/**
 * Fails when ort-sys's dist.txt names an ONNX Runtime version other than the
 * one whose licence files the manifest records (KTD3).
 */
export function checkOnnxRuntimeVersion(distText: string, manifestVersion: string): void {
  const versions = [...new Set([...distText.matchAll(/ms@(\d+\.\d+\.\d+)/g)].map((m) => m[1]))].sort(compare);
  if (versions.length === 0) {
    throw new Error("ort-sys dist.txt names no ONNX Runtime version (expected 'ms@<version>').");
  }
  if (versions.length > 1 || versions[0] !== manifestVersion) {
    throw new Error(
      `ONNX Runtime version mismatch: licences/native/manifest.json names ${manifestVersion}, ` +
        `but ort-sys dist.txt names ${versions.join(", ")}. ` +
        "Fetch the LICENSE and ThirdPartyNotices.txt of that version into licences/native/ and update the manifest.",
    );
  }
}
