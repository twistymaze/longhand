import { type MarkSpec, type NodeSpec, Schema } from "prosemirror-model";

import { taskItemNodeSpec, taskListNodeSpec } from "../node-views/task-item-view";

// The editor schema. Node and mark names, attributes and mark order match
// `markdown/schema.ts`, so that a document moves between the two through JSON.
// See PROVENANCE.md.
const nodes: Record<string, NodeSpec> = {
  doc: { content: "block+" },

  paragraph: {
    content: "inline*",
    group: "block",
    parseDOM: [{ tag: "p" }],
    toDOM() {
      return ["p", 0];
    },
  },

  text: { group: "inline" },

  heading: {
    content: "inline*",
    group: "block",
    attrs: { level: { default: 1 } },
    defining: true,
    parseDOM: [1, 2, 3, 4, 5, 6].map((level) => ({
      tag: `h${level}`,
      attrs: { level },
    })),
    toDOM(node) {
      return [`h${node.attrs.level}`, 0];
    },
  },

  blockquote: {
    content: "block+",
    group: "block",
    defining: true,
    parseDOM: [{ tag: "blockquote" }],
    toDOM() {
      return ["blockquote", 0];
    },
  },

  codeBlock: {
    content: "text*",
    marks: "",
    group: "block",
    code: true,
    defining: true,
    attrs: { language: { default: "" } },
    parseDOM: [{ tag: "pre", preserveWhitespace: "full" }],
    toDOM() {
      return ["pre", ["code", 0]];
    },
  },

  horizontalRule: {
    group: "block",
    parseDOM: [{ tag: "hr" }],
    toDOM() {
      return ["hr"];
    },
  },

  hardBreak: {
    inline: true,
    group: "inline",
    selectable: false,
    parseDOM: [{ tag: "br" }],
    toDOM() {
      return ["br"];
    },
  },

  bulletList: {
    content: "listItem+",
    group: "block",
    parseDOM: [{ tag: "ul:not([data-type])" }],
    toDOM() {
      return ["ul", 0];
    },
  },

  orderedList: {
    content: "listItem+",
    group: "block",
    attrs: { start: { default: 1 } },
    parseDOM: [
      {
        tag: "ol",
        getAttrs(dom) {
          const el = dom as HTMLElement;
          return {
            start: el.hasAttribute("start") ? +el.getAttribute("start")! : 1,
          };
        },
      },
    ],
    toDOM(node) {
      // Markers are drawn from a CSS counter, which cannot read `start`; seed
      // it inline so a non-1 start is visible and not just serialized.
      return node.attrs.start === 1
        ? ["ol", 0]
        : [
            "ol",
            {
              start: node.attrs.start,
              style: `counter-reset: ol-counter ${node.attrs.start - 1}`,
            },
            0,
          ];
    },
  },

  listItem: {
    content: "paragraph block*",
    defining: true,
    parseDOM: [{ tag: "li:not([data-type])" }],
    toDOM() {
      return ["li", 0];
    },
  },

  taskList: taskListNodeSpec,
  taskItem: taskItemNodeSpec,
};

const marks: Record<string, MarkSpec> = {
  highlight: {
    parseDOM: [{ tag: "mark" }],
    toDOM() {
      return ["mark", 0];
    },
  },

  strike: {
    parseDOM: [
      { tag: "s" },
      { tag: "del" },
      {
        style: "text-decoration",
        getAttrs: (value) => (value as string).includes("line-through") && null,
      },
    ],
    toDOM() {
      return ["s", 0];
    },
  },

  bold: {
    parseDOM: [
      { tag: "strong" },
      {
        tag: "b",
        getAttrs: (node) =>
          (node as HTMLElement).style.fontWeight !== "normal" && null,
      },
      {
        style: "font-weight=400",
        clearMark: (m) => m.type.name === "bold",
      },
      {
        style: "font-weight",
        getAttrs: (value) =>
          /^(bold(er)?|[5-9]\d{2,})$/.test(value as string) && null,
      },
    ],
    toDOM() {
      return ["strong", 0];
    },
  },

  italic: {
    parseDOM: [
      { tag: "em" },
      {
        tag: "i",
        getAttrs: (node) =>
          (node as HTMLElement).style.fontStyle !== "normal" && null,
      },
      { style: "font-style=italic" },
    ],
    toDOM() {
      return ["em", 0];
    },
  },

  link: {
    attrs: {
      href: {},
      target: { default: null },
    },
    inclusive: false,
    parseDOM: [
      {
        tag: "a[href]",
        getAttrs(dom) {
          return {
            href: (dom as HTMLElement).getAttribute("href"),
            target: (dom as HTMLElement).getAttribute("target"),
          };
        },
      },
    ],
    toDOM(node) {
      return [
        "a",
        {
          href: node.attrs.href,
          target: node.attrs.target,
          rel: "noopener noreferrer nofollow",
        },
        0,
      ];
    },
  },
};

export const schema = new Schema({ nodes, marks });
