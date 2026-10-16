import {
  MarkdownSerializer,
  type MarkdownSerializerState,
} from "prosemirror-markdown";
import type { Node as PMNode } from "prosemirror-model";

// The markdown serializer. Lists are tight, and text is escaped only where
// the parser would otherwise read a character as syntax, so that the file
// keeps the text as the user typed it. See PROVENANCE.md.

// Escape `text` so that `getParser()` reads it back as the same plain text.
// Each rule matches one syntax that the parser knows (see `parser.ts`); a
// character that cannot start that syntax stays as it is.
// `lineContinues` tells that more inline content follows the text on the
// same line of the file.
export function escapeText(
  text: string,
  atBlockStart: boolean,
  lineContinues = false,
): string {
  let out = text
    // A backslash escapes only ASCII punctuation. A backslash at the end of
    // the text comes before the next mark or line break, which starts with
    // punctuation.
    .replace(/\\(?=[!-/:-@[-`{-~]|$)/g, "\\\\")
    // Emphasis: `*` or `_` that touches a non-space character. An `_` inside
    // a word never starts emphasis. The neighbours come from the string that
    // this pass changes, because the pass before made it longer.
    .replace(/[*_]/g, (m, i: number, s: string) => {
      const before = i > 0 ? s[i - 1] : "";
      const after = i + 1 < s.length ? s[i + 1] : "";
      if (m === "_" && /\w/.test(before) && /\w/.test(after)) return m;
      if (/\s/.test(before) && /\s/.test(after)) return m;
      return "\\" + m;
    })
    // A `<br>` tag, with the pattern of the parser's hard break rule.
    .replace(/<(?=br\s*\/?>)/gi, "\\<")
    // Strikethrough `~~` and highlight `==`.
    .replace(/~(?=~)|(?<=~)~/g, "\\~")
    .replace(/=(?==)|(?<==)=/g, "\\=")
    // A link: `](` or a reference definition `]:`.
    .replace(/\](?=[(:])/g, "\\]")
    // An entity such as `&amp;`.
    .replace(/&(?=#?[a-zA-Z0-9]+;)/g, "\\&");

  if (atBlockStart) {
    // The parser removes up to three leading spaces and reads four as an
    // indented code block. An entity for the first one keeps them all as
    // text, and no line-start rule then applies. A line of only spaces is
    // a blank line in any form, so it stays as it is.
    if (lineContinues || /\S/.test(text)) {
      out = out.replace(/^[ \t]/, (c) => (c === " " ? "&#32;" : "&#9;"));
    }
    out = out
      .replace(/^(\s*)(#{1,6})(?=\s|$)/, "$1\\$2")
      .replace(/^([-+])(?=\s|$)/, "\\$1")
      .replace(/^-(?=(?:\s*-){2,}\s*$)/, "\\-")
      .replace(/^(\s*\d+)([.)])(?=\s|$)/, "$1\\$2")
      .replace(/^>/, "\\>")
      .replace(/^`{3}/, "\\$&")
      .replace(/^\[(?=[ xX]\](?:\s|$))/, "\\[");
  }
  return out;
}

type ListGroup = "bullet" | "ordered";

function listGroup(node: PMNode): ListGroup | null {
  const name = node.type.name;
  if (name === "bulletList" || name === "taskList") return "bullet";
  if (name === "orderedList") return "ordered";
  return null;
}

// The number of lists of the same group right before child `index` of
// `parent`, with empty paragraphs between them. CommonMark reads such lists
// as one list unless the marker changes, so each one takes the other marker.
function listsBefore(parent: PMNode, index: number): number {
  const group = listGroup(parent.child(index));
  let count = 0;
  for (let i = index - 1; i >= 0; i--) {
    const child = parent.child(i);
    if (child.type.name === "paragraph" && child.childCount === 0) continue;
    if (listGroup(child) !== group) break;
    count++;
  }
  return count;
}

// Write the list, with one blank line after a list of the same type before
// it. The base serializer writes two, which the parser reads as an empty
// paragraph.
function renderList(
  state: MarkdownSerializerState,
  node: PMNode,
  delim: string,
  firstDelim: (index: number) => string,
) {
  const s = state as unknown as {
    closed: PMNode | null;
    flushClose: (size: number) => void;
  };
  if (s.closed && s.closed.type === node.type) s.flushClose(2);
  state.renderList(node, delim, firstDelim);
}

function renderBulletList(
  state: MarkdownSerializerState,
  node: PMNode,
  parent: PMNode,
  index: number,
) {
  const marker = listsBefore(parent, index) % 2 === 0 ? "- " : "* ";
  renderList(state, node, "  ", () => marker);
}

let _serializer: MarkdownSerializer | null = null;

export function getSerializer(): MarkdownSerializer {
  if (_serializer) return _serializer;

  _serializer = new MarkdownSerializer(
    {
      blockquote(state, node) {
        state.wrapBlock("> ", null, node, () => state.renderContent(node));
      },

      codeBlock(state, node) {
        const backticks = node.textContent.match(/`{3,}/gm);
        const fence = backticks ? backticks.sort().slice(-1)[0] + "`" : "```";
        state.write(fence + (node.attrs.language || "") + "\n");
        state.text(node.textContent, false);
        state.write("\n");
        state.write(fence);
        state.closeBlock(node);
      },

      heading(state, node) {
        state.write(state.repeat("#", node.attrs.level) + " ");
        state.renderInline(node);
        state.closeBlock(node);
      },

      horizontalRule(state, node) {
        state.write("---");
        state.closeBlock(node);
      },

      bulletList: renderBulletList,

      orderedList(state, node, parent, index) {
        const start = node.attrs.start || 1;
        const maxW = String(start + node.childCount - 1).length;
        const space = state.repeat(" ", maxW + 2);
        const delim = listsBefore(parent, index) % 2 === 0 ? ". " : ") ";
        renderList(state, node, space, (i) => {
          const nStr = String(start + i);
          return state.repeat(" ", maxW - nStr.length) + nStr + delim;
        });
      },

      listItem(state, node) {
        state.renderContent(node);
      },

      taskList: renderBulletList,

      taskItem(state, node) {
        const checkbox = node.attrs.checked ? "[x] " : "[ ] ";
        state.write(checkbox);
        state.renderContent(node);
      },

      paragraph(state, node) {
        if (node.childCount === 0) {
          // Force flushClose of the previous block before marking ourselves
          // closed — produces an extra blank line per empty paragraph.
          state.write("");
        } else {
          state.renderInline(node);
        }
        state.closeBlock(node);
      },

      hardBreak(state, node, parent, index) {
        for (let i = index + 1; i < parent.childCount; i++) {
          if (parent.child(i).type !== node.type) {
            state.write("\\\n");
            return;
          }
        }
      },

      text(state, node, parent, index) {
        // Link text keeps the full escape: brackets inside it end the link.
        if (node.marks.some((mark) => mark.type.name === "link")) {
          state.text(node.text!, true);
          return;
        }
        state.write();
        // Text after a line break starts a new line of the file, where
        // block syntax applies as at the start of the block.
        const atBlockStart =
          (state as unknown as { atBlockStart: boolean }).atBlockStart ||
          (index > 0 && parent.child(index - 1).type.name === "hardBreak");
        const next = index + 1 < parent.childCount ? parent.child(index + 1) : null;
        const lineContinues = next !== null && next.type.name !== "hardBreak";
        state.text(escapeText(node.text!, atBlockStart, lineContinues), false);
      },
    },
    {
      bold: {
        open: "**",
        close: "**",
        mixable: true,
        expelEnclosingWhitespace: true,
      },
      italic: {
        open: "*",
        close: "*",
        mixable: true,
        expelEnclosingWhitespace: true,
      },
      strike: {
        open: "~~",
        close: "~~",
        mixable: true,
        expelEnclosingWhitespace: true,
      },
      link: {
        open: "[",
        close(_state: MarkdownSerializerState, mark) {
          const href: string = mark.attrs.href ?? "";
          // A destination with whitespace is valid only in angle brackets.
          if (/[\s<>]/.test(href)) {
            return `](<${href.replace(/[<>]/g, "\\$&")}>)`;
          }
          return `](${href.replace(/[()]/g, "\\$&")})`;
        },
        mixable: true,
      },
      highlight: { open: "==", close: "==" },
    },
    // `tightLists` is a runtime option that the type declarations omit.
    {
      hardBreakNodeName: "hardBreak",
      strict: false,
      tightLists: true,
    } as ConstructorParameters<typeof MarkdownSerializer>[2],
  );

  return _serializer;
}
