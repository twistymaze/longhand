import {
  MarkdownSerializer,
  type MarkdownSerializerState,
} from "prosemirror-markdown";

// The markdown serializer. Lists are tight, and text is escaped only where
// the parser would otherwise read a character as syntax, so that the file
// keeps the text as the user typed it. See PROVENANCE.md.

// Escape `text` so that `getParser()` reads it back as the same plain text.
// Each rule matches one syntax that the parser knows (see `parser.ts`); a
// character that cannot start that syntax stays as it is.
export function escapeText(text: string, atBlockStart: boolean): string {
  let out = text
    // A backslash escapes only ASCII punctuation.
    .replace(/\\(?=[!-/:-@[-`{-~])/g, "\\\\")
    // Emphasis: `*` or `_` that touches a non-space character. An `_` inside
    // a word never starts emphasis.
    .replace(/[*_]/g, (m, i: number) => {
      const before = i > 0 ? text[i - 1] : "";
      const after = i + 1 < text.length ? text[i + 1] : "";
      if (m === "_" && /\w/.test(before) && /\w/.test(after)) return m;
      if (/\s/.test(before) && /\s/.test(after)) return m;
      return "\\" + m;
    })
    // Strikethrough `~~` and highlight `==`.
    .replace(/~(?=~)|(?<=~)~/g, "\\~")
    .replace(/=(?==)|(?<==)=/g, "\\=")
    // A link: `](` or a reference definition `]:`.
    .replace(/\](?=[(:])/g, "\\]")
    // An entity such as `&amp;`.
    .replace(/&(?=#?[a-zA-Z0-9]+;)/g, "\\&");

  if (atBlockStart) {
    out = out
      .replace(/^(\s*)(#{1,6})(?=\s|$)/, "$1\\$2")
      .replace(/^([-+])(?=\s|$)/, "\\$1")
      .replace(/^-(?=(?:\s*-){2,}\s*$)/, "\\-")
      .replace(/^(\s*\d+)([.)])(?=\s|$)/, "$1\\$2")
      .replace(/^>/, "\\>")
      .replace(/^`{3}/, "\\`")
      .replace(/^\[(?=[ xX]\]\s)/, "\\[");
  }
  return out;
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

      bulletList(state, node) {
        state.renderList(node, "  ", () => "- ");
      },

      orderedList(state, node) {
        const start = node.attrs.start || 1;
        const maxW = String(start + node.childCount - 1).length;
        const space = state.repeat(" ", maxW + 2);
        state.renderList(node, space, (i) => {
          const nStr = String(start + i);
          return state.repeat(" ", maxW - nStr.length) + nStr + ". ";
        });
      },

      listItem(state, node) {
        state.renderContent(node);
      },

      taskList(state, node) {
        state.renderList(node, "  ", () => "- ");
      },

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

      text(state, node) {
        // Link text keeps the full escape: brackets inside it end the link.
        if (node.marks.some((mark) => mark.type.name === "link")) {
          state.text(node.text!, true);
          return;
        }
        state.write();
        const atBlockStart = (state as unknown as { atBlockStart: boolean })
          .atBlockStart;
        state.text(escapeText(node.text!, atBlockStart), false);
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
          const href = mark.attrs.href
            ? mark.attrs.href.replace(/[()]/g, "\\$&")
            : "";
          return `](${href})`;
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
