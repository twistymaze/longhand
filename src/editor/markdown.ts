import { Node as PMNode } from "prosemirror-model";

import { getParser } from "./markdown/parser";
import { markdownSchema } from "./markdown/schema";
import { getSerializer } from "./markdown/serializer";

// Convert between markdown text and the editor's JSON document. See
// PROVENANCE.md.

export { markdownSchema } from "./markdown/schema";

export interface JSONContent {
  type?: string;
  attrs?: Record<string, any>;
  content?: JSONContent[];
  marks?: { type: string; attrs?: Record<string, any> }[];
  text?: string;
}

export function md2json(markdown: string): JSONContent {
  try {
    const doc = getParser().parse(markdown);
    const json = doc.toJSON() as JSONContent;
    if (!json.content || json.content.length === 0) {
      return { type: "doc", content: [{ type: "paragraph" }] };
    }
    return json;
  } catch (error) {
    console.error(error);
    return {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [{ type: "text", text: markdown }],
        },
      ],
    };
  }
}

export function json2md(jsonContent: JSONContent): string {
  try {
    const doc = PMNode.fromJSON(markdownSchema, jsonContent);
    return getSerializer().serialize(doc);
  } catch (error) {
    console.error(error);
    return "";
  }
}
