// The summary's markdown, parsed into blocks rendered as React elements.
// The body comes from a model, so nothing is ever injected as HTML; only
// headings, lists, paragraphs, and bold, italic and code runs are kept.

export type Inline = { kind: "text" | "strong" | "em" | "code"; text: string };

/// A list item. Its child lists hold the items that are indented below it.
export type ListItem = { content: Inline[]; children: ListBlock[] };

export type ListBlock = { kind: "list"; ordered: boolean; items: ListItem[] };

export type Block =
  | { kind: "heading"; level: 1 | 2 | 3; content: Inline[] }
  | ListBlock
  | { kind: "paragraph"; content: Inline[] };

type OpenItem = { text: string; children: OpenList[] };
type OpenList = { ordered: boolean; items: OpenItem[] };
/// One open list, with the indent of its markers and the item that holds it.
type Level = { indent: number; list: OpenList; parent: OpenItem | null };

/// The width of the leading white space. A tab counts as four spaces.
function indentOf(raw: string): number {
  let width = 0;
  for (const c of raw) {
    if (c === " ") width += 1;
    else if (c === "\t") width += 4;
    else break;
  }
  return width;
}

function closeList(list: OpenList): ListBlock {
  return {
    kind: "list",
    ordered: list.ordered,
    items: list.items.map((item) => ({ content: parseInline(item.text), children: item.children.map(closeList) })),
  };
}

export function parseMarkdown(text: string): Block[] {
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  // The open lists, from the top-level list to the deepest child list.
  let levels: Level[] = [];

  const flushParagraph = () => {
    if (paragraph.length) blocks.push({ kind: "paragraph", content: parseInline(paragraph.join(" ")) });
    paragraph = [];
  };
  const flushList = () => {
    if (levels.length) blocks.push(closeList(levels[0].list));
    levels = [];
  };
  const lastItem = (level: Level) => level.list.items[level.list.items.length - 1];

  const addItem = (indent: number, ordered: boolean, itemText: string) => {
    // A shallower marker closes the deeper lists. The top-level list stays open.
    while (levels.length > 1 && levels[levels.length - 1].indent > indent) levels.pop();
    const top: Level | undefined = levels[levels.length - 1];
    if (top && indent > top.indent) {
      // A marker indented deeper than its parent item's marker starts a child list.
      const parent = lastItem(top);
      const list: OpenList = { ordered, items: [] };
      parent.children.push(list);
      levels.push({ indent, list, parent });
    } else if (top && top.list.ordered !== ordered) {
      // The other kind of marker at the same depth starts a new list.
      if (top.parent) {
        const list: OpenList = { ordered, items: [] };
        top.parent.children.push(list);
        levels[levels.length - 1] = { indent: top.indent, list, parent: top.parent };
      } else {
        flushList();
      }
    }
    if (!levels.length) levels = [{ indent, list: { ordered, items: [] }, parent: null }];
    levels[levels.length - 1].list.items.push({ text: itemText, children: [] });
  };

  // A wrapped line joins the deepest item whose marker is less indented.
  const addContinuation = (indent: number, line: string) => {
    while (levels.length > 1 && levels[levels.length - 1].indent >= indent) levels.pop();
    lastItem(levels[levels.length - 1]).text += ` ${line}`;
  };

  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const indent = indentOf(raw);
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    const bullet = /^[-*+]\s+(.*)$/.exec(line);
    const numbered = /^\d+[.)]\s+(.*)$/.exec(line);
    if (!line) {
      // A blank line ends a paragraph. A list stays open until a line that
      // is not part of it.
      flushParagraph();
    } else if (heading) {
      flushParagraph();
      flushList();
      const level = Math.min(3, heading[1].length) as 1 | 2 | 3;
      blocks.push({ kind: "heading", level, content: parseInline(heading[2]) });
    } else if (bullet || numbered) {
      flushParagraph();
      addItem(indent, !bullet, (bullet ?? numbered)![1]);
    } else if (levels.length && indent >= 2) {
      addContinuation(indent, line);
    } else {
      flushList();
      paragraph.push(line);
    }
  }
  flushParagraph();
  flushList();
  return blocks;
}

// Groups: 1 and 2 strong, 3 and 4 em, 5 code. An `_` or `__` inside a word
// is text: it opens only after a character that is not a letter or number,
// and closes only before one.
const INLINE_PATTERN = new RegExp(
  [
    String.raw`\*\*(.+?)\*\*`,
    String.raw`(?<![\p{L}\p{N}])__(.+?)__(?![\p{L}\p{N}])`,
    String.raw`\*(?!\s)(.+?)(?<!\s)\*`,
    String.raw`(?<![\p{L}\p{N}_])_(?!\s)(.+?)(?<!\s)_(?![\p{L}\p{N}_])`,
    "`([^`]+)`",
  ].join("|"),
  "gu",
);

export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  for (const m of text.matchAll(INLINE_PATTERN)) {
    const at = m.index ?? 0;
    if (at > last) out.push({ kind: "text", text: text.slice(last, at) });
    const strong = m[1] ?? m[2];
    const em = m[3] ?? m[4];
    if (strong !== undefined) out.push({ kind: "strong", text: strong });
    else if (em !== undefined) out.push({ kind: "em", text: em });
    else out.push({ kind: "code", text: m[5] });
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

/// The first paragraph as plain text: the overview in Home's preview (R6).
export function firstParagraph(text: string): string | null {
  const block = parseMarkdown(text).find((b) => b.kind === "paragraph");
  return block && block.kind === "paragraph" ? block.content.map((i) => i.text).join("") : null;
}
