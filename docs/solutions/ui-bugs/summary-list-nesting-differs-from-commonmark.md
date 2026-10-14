---
title: The Summary tab nests lists in a different way from CommonMark, so other summary renderers must use its parser
date: 2026-10-14
category: ui-bugs
module: Summary markdown (src/session/markdown.ts parseMarkdown)
problem_type: best_practice
component: frontend
severity: medium
applies_when:
  - "Code renders a summary body for a new surface: copied HTML, an export, a preview or a print view"
  - "Code changes how the Summary tab parses the summary markdown"
symptoms:
  - "A summary like '1. **Budget:**' followed by '  - detail' shows a nested list in the Summary tab, but markdown-it closes the ordered list and starts a separate bullet list"
tags: [summary, markdown, list-nesting, commonmark, markdown-it, copy, clipboard]
related_components: [notes]
---

# The Summary tab nests lists in a different way from CommonMark, so other summary renderers must use its parser

## Context

The Summary tab does not render the summary with a CommonMark library. It uses its own parser, `parseMarkdown` in `src/session/markdown.ts:41`. That parser nests a list marker when its indent is more than the indent of the parent marker (`src/session/markdown.ts:59-62`). CommonMark, and so markdown-it, nests a list item only when its indent reaches the content column of the parent item. For `1. ` that column is 3.

Models often write a bullet with two spaces of indent under a numbered item:

```markdown
1. **Budget:**
  - detail
```

The Summary tab shows `detail` as a child of `Budget:`. markdown-it closes the ordered list and starts a separate bullet list. The two outputs have a different structure from the same text.

The plan for formatted copy (PR #18) first rendered all copied HTML with markdown-it. The document review of the plan found the difference before the code was written. With markdown-it, a copied summary would paste with a list structure that the user did not see in the app.

## Guidance

To render a summary body anywhere other than the Summary tab, build the output from the blocks of `parseMarkdown`, not from a CommonMark renderer. The copy module does this: `tabHtml` uses `parseMarkdown` for the summary and markdown-it only for the transcript and My Notes (`src/session/copy.ts:56-57`).

The same rule applies to the other summary inline marks. `parseMarkdown` keeps only bold, italic and code. A link, strikethrough or highlight in a summary is text in the tab, so it is text in the copied HTML too. If a new surface needs links in summaries, extend `parseMarkdown` and the Summary tab together, so that the two stay the same.

## Why This Matters

A user compares the paste or the export with what the app showed. If a nested item becomes a separate list, the meaning of the summary changes ("detail" no longer belongs to "Budget"). The difference shows only for one indent shape, so a test with `- one\n  - two` passes with both parsers and hides it.

## When to Apply

- Before you add a renderer for summary text.
- When a test of summary output uses lists: include a numbered item with a two-space bullet under it. `src/session/copy.test.ts` has this case ("the summary keeps the nesting that the Summary tab shows").

## Examples

```ts
// Wrong for a summary: the CommonMark structure differs from the tab.
markdown.render(summaryBody);

// Right: the same blocks that the Summary tab renders.
parseMarkdown(summaryBody).map(blockHtml).join("");
```
