import MarkdownIt from "markdown-it";
import type StateInline from "markdown-it/lib/rules_inline/state_inline.mjs";
import type Token from "markdown-it/lib/token.mjs";
import { MarkdownParser } from "prosemirror-markdown";

import { markdownSchema } from "./schema";

// The markdown parser: CommonMark plus strikethrough, highlight, `<br>` hard
// breaks, task lists and empty paragraphs. See PROVENANCE.md.

// Parse `open` + inner text + `close` as one mark. The inner text is parsed
// again, so that marks inside the mark (for example `==**bold**==`) stay.
function wrappedMarkRule(
  md: MarkdownIt,
  name: string,
  tag: string,
  markerCode: number,
  markup: string,
) {
  const pattern = new RegExp(`^${markup.replace(/./g, "\\$&")}([\\s\\S]+?)${markup.replace(/./g, "\\$&")}`);
  md.inline.ruler.before(
    "emphasis",
    name,
    (state: StateInline, silent: boolean) => {
      const start = state.pos;
      if (
        state.src.charCodeAt(start) !== markerCode ||
        state.src.charCodeAt(start + 1) !== markerCode
      ) {
        return false;
      }

      const match = state.src.slice(start).match(pattern);
      if (!match) return false;

      if (!silent) {
        const open = state.push(`${name}_open`, tag, 1);
        open.markup = markup;

        const oldMax = state.posMax;
        state.pos = start + markup.length;
        state.posMax = start + markup.length + match[1].length;
        state.md.inline.tokenize(state);
        state.posMax = oldMax;

        const close = state.push(`${name}_close`, tag, -1);
        close.markup = markup;
      }

      state.pos = start + match[0].length;
      return true;
    },
  );
}

export function strikethroughPlugin(md: MarkdownIt) {
  wrappedMarkRule(md, "s", "s", 0x7e /* ~ */, "~~");
}

export function highlightPlugin(md: MarkdownIt) {
  wrappedMarkRule(md, "highlight", "mark", 0x3d /* = */, "==");
}

function hardBreakTagPlugin(md: MarkdownIt) {
  md.inline.ruler.before(
    "html_inline",
    "hardbreak_tag",
    (state: StateInline, silent: boolean) => {
      const match = state.src.slice(state.pos).match(/^<br\s*\/?>/i);
      if (!match) return false;

      if (!silent) {
        const token = state.push("hardbreak", "br", 0);
        token.markup = match[0];
      }

      state.pos += match[0].length;
      return true;
    },
  );
}

function taskListPlugin(md: MarkdownIt) {
  md.core.ruler.after("inline", "task_lists", (state) => {
    const tokens = state.tokens;
    for (let i = 0; i < tokens.length; i++) {
      if (tokens[i].type !== "bullet_list_open") continue;

      const closeIdx = findMatchingClose(
        tokens,
        i,
        "bullet_list_open",
        "bullet_list_close",
      );
      if (closeIdx === -1) continue;

      // Only the direct items of this list count. A nested list gets its own
      // pass, so a task in it does not change the items of this list.
      const itemLevel = tokens[i].level + 1;
      const isItem = (k: number) =>
        tokens[k].type === "list_item_open" && tokens[k].level === itemLevel;

      let hasTask = false;
      for (let j = i + 1; j < closeIdx && !hasTask; j++) {
        if (!isItem(j)) continue;
        const inlineIdx = findInlineToken(tokens, j);
        hasTask =
          inlineIdx !== -1 && TASK_PREFIX.test(tokens[inlineIdx].content);
      }
      if (!hasTask) continue;

      tokens[i].type = "task_list_open";
      tokens[i].tag = "ul";
      tokens[closeIdx].type = "task_list_close";
      tokens[closeIdx].tag = "ul";

      for (let k = i + 1; k < closeIdx; k++) {
        if (isItem(k)) {
          const inlineIdx = findInlineToken(tokens, k);
          if (inlineIdx !== -1) {
            const content = tokens[inlineIdx].content;
            const taskMatch = content.match(TASK_PREFIX);
            if (taskMatch) {
              const checked = taskMatch[1].toLowerCase() === "x";
              tokens[k].type = "task_item_open";
              tokens[k].attrSet("checked", checked ? "true" : "false");

              tokens[inlineIdx].content = content.slice(taskMatch[0].length);
              if (tokens[inlineIdx].children) {
                stripTaskPrefix(
                  tokens[inlineIdx].children!,
                  taskMatch[0].length,
                );
              }
            } else {
              tokens[k].type = "task_item_open";
              tokens[k].attrSet("checked", "false");
            }
          }

          const itemCloseIdx = findMatchingClose(
            tokens,
            k,
            "list_item_open",
            "list_item_close",
          );
          if (
            itemCloseIdx !== -1 &&
            tokens[itemCloseIdx].type === "list_item_close"
          ) {
            tokens[itemCloseIdx].type = "task_item_close";
          }
        }
      }
    }
  });
}

function findInlineToken(tokens: Token[], fromIdx: number): number {
  for (let i = fromIdx + 1; i < tokens.length; i++) {
    if (tokens[i].type === "inline") return i;
    if (
      tokens[i].type === "list_item_close" ||
      tokens[i].type === "task_item_close" ||
      tokens[i].type === "bullet_list_close"
    ) {
      break;
    }
  }
  return -1;
}

function findMatchingClose(
  tokens: Token[],
  openIdx: number,
  openType: string,
  closeType: string,
): number {
  let depth = 1;
  for (let i = openIdx + 1; i < tokens.length; i++) {
    if (tokens[i].type === openType) depth++;
    if (tokens[i].type === closeType) {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

// `[ ]`, `[x]` or `[X]`, then whitespace or the end of the item text.
// markdown-it removes the whitespace at the end of a paragraph, so an empty
// task is only `[ ]`.
const TASK_PREFIX = /^\[([ xX])\](?:\s+|$)/;

function stripTaskPrefix(children: Token[], prefixLen: number) {
  let remaining = prefixLen;
  for (let i = 0; i < children.length && remaining > 0; i++) {
    const child = children[i];
    if (child.type === "text" && child.content) {
      if (child.content.length <= remaining) {
        remaining -= child.content.length;
        child.content = "";
      } else {
        child.content = child.content.slice(remaining);
        remaining = 0;
      }
    }
  }
}

// Markdown collapses consecutive blank lines, so empty paragraphs would be lost
// on roundtrip. We use the line maps that markdown-it attaches to top-level
// block tokens to detect blank lines (leading, trailing, and between blocks)
// and emit explicit empty paragraph tokens.
function emptyParagraphsPlugin(md: MarkdownIt) {
  md.core.ruler.after("block", "empty_paragraphs", (state) => {
    const tokens = state.tokens;
    const out: Token[] = [];
    const totalLines = countLines(state.src);
    const lines = state.src.split("\n");
    // The map of a list includes the blank lines after it. A list ends at
    // its last line that is not blank, so those lines count as empty
    // paragraphs too.
    const listEnd = (map: [number, number]) => {
      let end = map[1];
      while (end > map[0] + 1 && (lines[end - 1] ?? "").trim() === "") end--;
      return end;
    };

    const pushEmpty = () => {
      out.push(
        new state.Token("paragraph_open", "p", 1),
        new state.Token("paragraph_close", "p", -1),
      );
    };

    let prevEndLine = -1;
    for (const token of tokens) {
      const isTopLevelBlockOpen =
        token.level === 0 && token.nesting >= 0 && token.map !== null;

      if (isTopLevelBlockOpen) {
        // Leading: the entire gap before the first block is empty paragraphs.
        // Between blocks: one blank line is normal separation; the rest are
        // empty paragraphs.
        const extras =
          prevEndLine === -1 ? token.map![0] : token.map![0] - prevEndLine - 1;
        for (let i = 0; i < extras; i++) pushEmpty();
      }

      out.push(token);

      if (isTopLevelBlockOpen) {
        const isList =
          token.type === "bullet_list_open" ||
          token.type === "ordered_list_open";
        prevEndLine = isList ? listEnd(token.map!) : token.map![1];
      }
    }

    if (prevEndLine === -1) {
      // No blocks at all — every line plus the implicit "current" line is an
      // empty paragraph. Empty input still produces one empty paragraph, which
      // matches the editor's default "blank document" state.
      for (let i = 0; i <= totalLines; i++) pushEmpty();
    } else if (totalLines > prevEndLine) {
      const trailing = totalLines - prevEndLine;
      for (let i = 0; i < trailing; i++) pushEmpty();
    }

    state.tokens = out;
  });
}

function countLines(src: string): number {
  if (src === "") return 0;
  const newlines = (src.match(/\n/g) || []).length;
  return newlines + (src.endsWith("\n") ? 0 : 1);
}

let _parser: MarkdownParser | null = null;

export function getParser(): MarkdownParser {
  if (_parser) return _parser;

  const md = MarkdownIt("commonmark", { html: false });

  // The schema has no inline code and no image, and `<url>` text stays text.
  // Without these rules the characters stay as the user typed them.
  md.disable(["backticks", "image", "autolink"]);

  md.use(strikethroughPlugin);
  md.use(highlightPlugin);
  md.use(hardBreakTagPlugin);
  md.use(taskListPlugin);
  md.use(emptyParagraphsPlugin);

  _parser = new MarkdownParser(markdownSchema, md, {
    blockquote: { block: "blockquote" },
    paragraph: { block: "paragraph" },
    list_item: { block: "listItem" },
    bullet_list: { block: "bulletList" },
    ordered_list: {
      block: "orderedList",
      getAttrs: (tok) => ({ start: +tok.attrGet("start")! || 1 }),
    },
    heading: {
      block: "heading",
      getAttrs: (tok) => ({ level: +tok.tag.slice(1) }),
    },
    code_block: { block: "codeBlock", noCloseToken: true },
    fence: {
      block: "codeBlock",
      getAttrs: (tok) => ({ language: tok.info || "" }),
      noCloseToken: true,
    },
    hr: { node: "horizontalRule" },
    hardbreak: { node: "hardBreak" },

    em: { mark: "italic" },
    strong: { mark: "bold" },
    s: { mark: "strike" },
    link: {
      mark: "link",
      getAttrs: (tok) => ({
        href: tok.attrGet("href"),
        target: tok.attrGet("target"),
      }),
    },

    highlight: { mark: "highlight" },
    task_list: { block: "taskList" },
    task_item: {
      block: "taskItem",
      getAttrs: (tok) => ({
        checked: tok.attrGet("checked") === "true",
      }),
    },
  });

  return _parser;
}
