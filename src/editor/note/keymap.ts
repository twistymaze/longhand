import {
  chainCommands,
  createParagraphNear,
  deleteSelection,
  joinBackward,
  joinForward,
  liftEmptyBlock,
  newlineInCode,
  selectAll,
  selectNodeBackward,
  selectNodeForward,
  selectTextblockEnd,
  selectTextblockStart,
  setBlockType,
  splitBlock,
  toggleMark,
} from "prosemirror-commands";
import { redo, undo } from "prosemirror-history";
import {
  InputRule,
  inputRules,
  textblockTypeInputRule,
  undoInputRule,
  wrappingInputRule,
} from "prosemirror-inputrules";
import { keymap } from "prosemirror-keymap";
import {
  Fragment,
  type MarkType,
  type Node as PMNode,
  type NodeType,
} from "prosemirror-model";
import {
  liftListItem,
  sinkListItem,
  splitListItem,
} from "prosemirror-schema-list";
import {
  TextSelection,
  type Command,
  type EditorState,
  type Transaction,
} from "prosemirror-state";

import { schema } from "./schema";

function isInListItem(state: EditorState): string | null {
  const { $from } = state.selection;
  for (let depth = $from.depth; depth > 0; depth--) {
    const name = $from.node(depth).type.name;
    if (name === "listItem" || name === "taskItem") return name;
  }
  return null;
}

function moveListItem(direction: "up" | "down"): Command {
  return (state, dispatch) => {
    const { $from } = state.selection;

    let depth = -1;
    for (let d = $from.depth; d > 0; d--) {
      const name = $from.node(d).type.name;
      if (name === "listItem" || name === "taskItem") {
        depth = d;
        break;
      }
    }
    if (depth === -1) return false;

    const parent = $from.node(depth - 1);
    const index = $from.index(depth - 1);
    const atBoundary =
      direction === "up" ? index === 0 : index >= parent.childCount - 1;

    if (!atBoundary) {
      // Swap with adjacent sibling
      const siblingIndex = direction === "up" ? index - 1 : index + 1;
      const currentItem = parent.child(index);
      const siblingItem = parent.child(siblingIndex);

      if (dispatch) {
        const tr = state.tr;
        const currentStart = $from.before(depth);
        const currentEnd = $from.after(depth);

        if (direction === "up") {
          const prevStart = currentStart - siblingItem.nodeSize;
          tr.replaceWith(
            prevStart,
            currentEnd,
            Fragment.from([currentItem, siblingItem]),
          );
          const offset = prevStart - currentStart;
          tr.setSelection(
            TextSelection.create(
              tr.doc,
              state.selection.anchor + offset,
              state.selection.head + offset,
            ),
          );
        } else {
          const nextEnd = currentEnd + siblingItem.nodeSize;
          tr.replaceWith(
            currentStart,
            nextEnd,
            Fragment.from([siblingItem, currentItem]),
          );
          const offset = siblingItem.nodeSize;
          tr.setSelection(
            TextSelection.create(
              tr.doc,
              state.selection.anchor + offset,
              state.selection.head + offset,
            ),
          );
        }

        dispatch(tr.scrollIntoView());
      }
      return true;
    }

    // At boundary: lift item into the outer (parent) list
    let outerDepth = -1;
    for (let d = depth - 2; d > 0; d--) {
      const name = $from.node(d).type.name;
      if (name === "listItem" || name === "taskItem") {
        outerDepth = d;
        break;
      }
    }
    if (outerDepth === -1) return false;

    // Only lift when the item type is compatible with the outer list
    const outerListName = $from.node(outerDepth - 1).type.name;
    const currentItemName = $from.node(depth).type.name;
    const compatible =
      (currentItemName === "listItem" &&
        (outerListName === "bulletList" || outerListName === "orderedList")) ||
      (currentItemName === "taskItem" && outerListName === "taskList");
    if (!compatible) return false;

    if (dispatch) {
      const tr = state.tr;
      const currentItem = parent.child(index);
      const currentStart = $from.before(depth);
      const currentEnd = $from.after(depth);
      const anchorOffset = state.selection.anchor - currentStart;

      // Delete the item, or the entire nested list when it's the only child
      if (parent.childCount === 1) {
        tr.delete($from.before(depth - 1), $from.after(depth - 1));
      } else {
        tr.delete(currentStart, currentEnd);
      }

      // Insert into the outer list: before the outer item (up) or after (down)
      const targetPos =
        direction === "up" ? $from.before(outerDepth) : $from.after(outerDepth);
      const insertPos = tr.mapping.map(targetPos);
      tr.insert(insertPos, currentItem);

      tr.setSelection(TextSelection.create(tr.doc, insertPos + anchorOffset));
      dispatch(tr.scrollIntoView());
    }
    return true;
  };
}

function joinTaskItemBackward(
  state: EditorState,
  dispatch?: (tr: Transaction) => void,
) {
  const { selection } = state;
  const { $from } = selection;

  if (!selection.empty || $from.parentOffset !== 0) {
    return false;
  }

  let itemDepth = -1;
  for (let depth = $from.depth; depth > 0; depth--) {
    if ($from.node(depth).type === schema.nodes.taskItem) {
      itemDepth = depth;
      break;
    }
  }

  if (itemDepth === -1 || $from.parent.type !== schema.nodes.paragraph) {
    return false;
  }

  const list = $from.node(itemDepth - 1);
  if (list.type !== schema.nodes.taskList) {
    return false;
  }

  if ($from.index(itemDepth) !== 0) {
    return false;
  }

  const itemIndex = $from.index(itemDepth - 1);
  const currentItem = list.child(itemIndex);
  if (itemIndex === 0) {
    // Lift instead of falling through to joinBackward, which would merge the
    // item's text into the preceding block and silently destroy the task
    if (!currentItem.firstChild?.content.size) return false;
    return liftListItem(schema.nodes.taskItem)(state, dispatch);
  }

  const previousItem = list.child(itemIndex - 1);
  const previousParagraphIndex = previousItem.childCount - 1;
  const previousParagraph = previousItem.child(previousParagraphIndex);
  const currentParagraph = currentItem.firstChild;

  if (
    previousParagraph.type !== schema.nodes.paragraph ||
    currentParagraph?.type !== schema.nodes.paragraph
  ) {
    return false;
  }

  if (!dispatch) {
    return true;
  }

  const mergedParagraph = previousParagraph.type.create(
    previousParagraph.attrs,
    previousParagraph.content.append(currentParagraph.content),
    previousParagraph.marks,
  );

  const mergedPreviousContent = [
    ...Array.from({ length: previousParagraphIndex }, (_, index) =>
      previousItem.child(index),
    ),
    mergedParagraph,
    ...Array.from({ length: currentItem.childCount - 1 }, (_, index) =>
      currentItem.child(index + 1),
    ),
  ];
  const mergedPreviousItem = previousItem.type.create(
    previousItem.attrs,
    Fragment.from(mergedPreviousContent),
    previousItem.marks,
  );

  const currentStart = $from.before(itemDepth);
  const currentEnd = $from.after(itemDepth);
  const previousStart = currentStart - previousItem.nodeSize;
  let paragraphOffset = 0;
  for (let index = 0; index < previousParagraphIndex; index++) {
    paragraphOffset += previousItem.child(index).nodeSize;
  }
  const selectionPos =
    previousStart + 1 + paragraphOffset + 1 + previousParagraph.content.size;

  const tr = state.tr.replaceWith(
    previousStart,
    currentEnd,
    mergedPreviousItem,
  );
  tr.setSelection(TextSelection.create(tr.doc, selectionPos));
  dispatch(tr.scrollIntoView());
  return true;
}

// ---------------------------------------------------------------------------
// Input rules
// ---------------------------------------------------------------------------
function headingRule(nodeType: NodeType, maxLevel: number) {
  return textblockTypeInputRule(
    new RegExp(`^(#{1,${maxLevel}})\\s$`),
    nodeType,
    (match) => ({ level: match[1].length }),
  );
}

function blockquoteRule(nodeType: NodeType) {
  return wrappingInputRule(/^\s*>\s$/, nodeType);
}

function bulletListRule(nodeType: NodeType) {
  return wrappingInputRule(/^\s*([-+*])\s$/, nodeType);
}

function orderedListRule(nodeType: NodeType) {
  return wrappingInputRule(
    /^\s*(\d+)\.\s$/,
    nodeType,
    (match) => ({ start: +match[1] }),
    (match, node) => node.childCount + node.attrs.start === +match[1],
  );
}

function codeBlockRule(nodeType: NodeType) {
  return textblockTypeInputRule(/^```$/, nodeType);
}

function horizontalRuleRule() {
  return new InputRule(
    /^(?:---|___|\*\*\*)\s$/,
    (state, _match, start, end) => {
      const hr = schema.nodes.horizontalRule.create();
      return state.tr.replaceWith(start - 1, end, [
        hr,
        schema.nodes.paragraph.create(),
      ]);
    },
  );
}

function markInputRule(pattern: RegExp, markType: MarkType, delimLen: number) {
  return new InputRule(pattern, (state, match, start, end) => {
    const prefix = match[1];
    const content = match[2];
    const { tr } = state;

    const openStart = start + prefix.length;
    // The typed character that triggered this rule is the last char of
    // the closing delimiter and is NOT in the document yet.  Only the
    // remaining delimLen-1 chars need to be removed.
    const closeCharsInDoc = delimLen - 1;

    const $start = state.doc.resolve(openStart);
    if (!$start.parent.type.allowsMarkType(markType)) return null;

    if (closeCharsInDoc > 0) {
      tr.delete(end - closeCharsInDoc, end);
    }
    tr.delete(openStart, openStart + delimLen);
    tr.addMark(openStart, openStart + content.length, markType.create());
    tr.removeStoredMark(markType);

    return tr;
  });
}

/// A task list with one task. Its paragraph holds `content`, and `blocks`
/// follow the paragraph in the task.
export function taskListWith(
  content: Fragment,
  checked = false,
  blocks: readonly PMNode[] = [],
): PMNode {
  const paragraph = schema.nodes.paragraph.create(null, content);
  return schema.nodes.taskList.create(
    null,
    schema.nodes.taskItem.create({ checked }, [paragraph, ...blocks]),
  );
}

// `[] `, `[ ] ` or `[x] ` at the start of a paragraph makes a task list, with
// the text after the cursor as the task text. At the start of a bullet item
// (the result of typing `- ` first), only that item becomes a task, so that
// `- [ ] ` also makes a task and the other items stay bullets.
function taskListRule() {
  return new InputRule(/^\s*\[([ x]?)\]\s$/, (state, match, start, end) => {
    const checked = match[1] === "x";
    const $start = state.doc.resolve(start);
    const depth = $start.depth;
    if ($start.parent.type !== schema.nodes.paragraph) return null;
    // The typed text that matched is from `start` to `end`.
    const rest = $start.parent.content.cut(end - $start.start());

    const inItem =
      depth >= 3 &&
      $start.node(depth - 1).type === schema.nodes.listItem &&
      $start.index(depth - 1) === 0;
    if (inItem && $start.node(depth - 2).type === schema.nodes.bulletList) {
      const list = $start.node(depth - 2);
      const index = $start.index(depth - 2);
      const item = list.child(index);
      const blocks: PMNode[] = [];
      item.forEach((child, _offset, i) => {
        if (i > 0) blocks.push(child);
      });
      const items = (from: number, to: number) => {
        const nodes: PMNode[] = [];
        for (let i = from; i < to; i++) nodes.push(list.child(i));
        return nodes;
      };
      const before = items(0, index);
      const after = items(index + 1, list.childCount);
      const nodes: PMNode[] = [];
      if (before.length) nodes.push(list.type.create(list.attrs, before));
      nodes.push(taskListWith(rest, checked, blocks));
      if (after.length) nodes.push(list.type.create(list.attrs, after));

      const listStart = $start.before(depth - 2);
      const tr = state.tr.replaceWith(listStart, $start.after(depth - 2), nodes);
      const taskStart = listStart + (before.length ? nodes[0].nodeSize : 0);
      // The text starts three levels into taskList > taskItem > paragraph.
      return tr.setSelection(TextSelection.create(tr.doc, taskStart + 3));
    }
    // A task list cannot be the first block of another list item.
    if (inItem || $start.node(depth - 1).type === schema.nodes.taskItem) {
      return null;
    }

    const blockStart = $start.before(depth);
    const tr = state.tr.replaceWith(
      blockStart,
      $start.after(depth),
      taskListWith(rest, checked),
    );
    return tr.setSelection(TextSelection.create(tr.doc, blockStart + 3));
  });
}

export function buildInputRules() {
  return inputRules({
    rules: [
      headingRule(schema.nodes.heading, 6),
      blockquoteRule(schema.nodes.blockquote),
      bulletListRule(schema.nodes.bulletList),
      orderedListRule(schema.nodes.orderedList),
      codeBlockRule(schema.nodes.codeBlock),
      horizontalRuleRule(),
      taskListRule(),
      markInputRule(/(^|[^*])\*\*([^*]+)\*\*$/, schema.marks.bold, 2),
      markInputRule(/(^|[^~])~~([^~]+)~~$/, schema.marks.strike, 2),
      markInputRule(/(^|[^=])==([^=]+)==$/, schema.marks.highlight, 2),
      // One delimiter makes a mark only as markdown reads it: the opening
      // one has no letter or number before it and no space after it, and
      // the closing one has no space before it. So `my_var_name`,
      // `2*3*4` and `~5 min, ~10` stay as typed.
      markInputRule(
        /(^|[^\p{L}\p{N}*])\*([^*\s](?:[^*]*[^*\s])?)\*$/u,
        schema.marks.italic,
        1,
      ),
      markInputRule(
        /(^|[^\p{L}\p{N}_])_([^_\s](?:[^_]*[^_\s])?)_$/u,
        schema.marks.italic,
        1,
      ),
      markInputRule(
        /(^|[^\p{L}\p{N}~])~([^~\s](?:[^~]*[^~\s])?)~$/u,
        schema.marks.strike,
        1,
      ),
    ],
  });
}

// ---------------------------------------------------------------------------
// Keymaps
// ---------------------------------------------------------------------------
const mac =
  typeof navigator !== "undefined"
    ? /Mac|iP(hone|[oa]d)/.test(navigator.platform)
    : false;

export function buildKeymap() {
  const keys: Record<string, Command> = {};

  keys["Mod-z"] = undo;
  keys["Mod-Shift-z"] = redo;
  if (!mac) keys["Mod-y"] = redo;

  keys["Mod-b"] = toggleMark(schema.marks.bold);
  keys["Mod-i"] = toggleMark(schema.marks.italic);

  const exitCodeBlockOnEmptyLine: Command = (state, dispatch) => {
    const { $from } = state.selection;
    if (!$from.parent.type.spec.code) return false;

    const lastLine = $from.parent.textContent.split("\n").pop() ?? "";
    const atEnd = $from.parentOffset === $from.parent.content.size;
    if (!atEnd || lastLine !== "") return false;

    if (dispatch) {
      const codeBlockPos = $from.before($from.depth);
      const codeBlock = $from.parent;
      const textContent = codeBlock.textContent.replace(/\n$/, "");
      const tr = state.tr;

      tr.replaceWith(
        codeBlockPos,
        codeBlockPos + codeBlock.nodeSize,
        textContent
          ? [
              schema.nodes.codeBlock.create(
                codeBlock.attrs,
                schema.text(textContent),
              ),
              schema.nodes.paragraph.create(),
            ]
          : [schema.nodes.paragraph.create()],
      );

      const newParaPos = textContent
        ? codeBlockPos + textContent.length + 2 + 1
        : codeBlockPos + 1;
      tr.setSelection(TextSelection.create(tr.doc, newParaPos));
      dispatch(tr.scrollIntoView());
    }
    return true;
  };

  keys["Enter"] = chainCommands(
    exitCodeBlockOnEmptyLine,
    newlineInCode,
    (state, dispatch) => {
      const itemName = isInListItem(state);
      if (!itemName) return false;
      const { $from } = state.selection;
      if ($from.parent.content.size !== 0) return false;
      const nodeType = state.schema.nodes[itemName];
      if (!nodeType) return false;
      return liftListItem(nodeType)(state, dispatch);
    },
    (state, dispatch) => {
      const itemName = isInListItem(state);
      if (!itemName) return false;
      const nodeType = state.schema.nodes[itemName];
      if (!nodeType) return false;
      return splitListItem(nodeType)(state, dispatch);
    },
    createParagraphNear,
    liftEmptyBlock,
    splitBlock,
  );

  const revertBlockToParagraph: Command = (state, dispatch) => {
    const { $from } = state.selection;
    if (!state.selection.empty || $from.parentOffset !== 0) return false;
    const node = $from.parent;
    if (
      node.type !== schema.nodes.heading &&
      node.type !== schema.nodes.codeBlock
    ) {
      return false;
    }
    return setBlockType(schema.nodes.paragraph)(state, dispatch);
  };

  const backspaceCmd: Command = chainCommands(
    // Right after a typed shortcut made a mark or a block, Backspace puts
    // back the typed text.
    undoInputRule,
    deleteSelection,
    (state, _dispatch) => {
      const { selection } = state;
      if (selection.$head.pos === 0 && selection.empty) return true;
      return false;
    },
    revertBlockToParagraph,
    joinTaskItemBackward,
    joinBackward,
    selectNodeBackward,
  );
  keys["Backspace"] = backspaceCmd;
  keys["Mod-Backspace"] = backspaceCmd;
  keys["Shift-Backspace"] = backspaceCmd;

  const deleteCmd: Command = chainCommands(
    deleteSelection,
    joinForward,
    selectNodeForward,
  );
  keys["Delete"] = deleteCmd;
  keys["Mod-Delete"] = deleteCmd;

  keys["Mod-a"] = selectAll;

  if (mac) {
    keys["Ctrl-h"] = backspaceCmd;
    keys["Alt-Backspace"] = backspaceCmd;
    keys["Ctrl-d"] = deleteCmd;
    keys["Ctrl-Alt-Backspace"] = deleteCmd;
    keys["Alt-Delete"] = deleteCmd;
    keys["Alt-d"] = deleteCmd;
    keys["Ctrl-a"] = selectTextblockStart;
    keys["Ctrl-e"] = selectTextblockEnd;
  }

  // Prevent Tab from moving focus outside the editor
  keys["Tab"] = (state, dispatch) => {
    const itemName = isInListItem(state);
    if (!itemName) return true;
    const nodeType = state.schema.nodes[itemName];
    if (!nodeType) return true;
    // The first item cannot go in: Tab still stays in the editor.
    return sinkListItem(nodeType)(state, dispatch) || true;
  };

  // Check or clear the task at the cursor: the keyboard way to the checkbox.
  keys["Mod-Enter"] = (state, dispatch) => {
    const { $from } = state.selection;
    for (let depth = $from.depth; depth > 0; depth--) {
      const node = $from.node(depth);
      if (node.type !== schema.nodes.taskItem) continue;
      dispatch?.(
        state.tr.setNodeMarkup($from.before(depth), undefined, {
          ...node.attrs,
          checked: !node.attrs.checked,
        }),
      );
      return true;
    }
    return false;
  };

  keys["Shift-Tab"] = (state, dispatch) => {
    const itemName = isInListItem(state);
    if (!itemName) return false;
    const nodeType = state.schema.nodes[itemName];
    if (!nodeType) return false;
    return liftListItem(nodeType)(state, dispatch);
  };

  keys["Alt-ArrowUp"] = moveListItem("up");
  keys["Alt-ArrowDown"] = moveListItem("down");

  return keymap(keys);
}
