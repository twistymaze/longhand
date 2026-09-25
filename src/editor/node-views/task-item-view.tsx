import {
  type NodeViewComponentProps,
  useEditorEventCallback,
  useIsNodeSelected,
} from "@handlewithcare/react-prosemirror";
import type { NodeSpec } from "prosemirror-model";
import { forwardRef, type ReactNode } from "react";

import { TaskCheckbox } from "./task-checkbox";

// The task list nodes and the React view of a task item. A task item stores
// only `checked`. See PROVENANCE.md.

export const taskListNodeSpec: NodeSpec = {
  content: "taskItem+",
  group: "block",
  parseDOM: [{ tag: 'ul[data-type="taskList"]' }],
  toDOM() {
    return ["ul", { "data-type": "taskList", class: "task-list" }, 0];
  },
};

export const taskItemNodeSpec: NodeSpec = {
  content: "paragraph block*",
  defining: true,
  attrs: { checked: { default: false } },
  parseDOM: [
    {
      tag: 'li[data-type="taskItem"]',
      getAttrs(dom) {
        return {
          checked: (dom as HTMLElement).getAttribute("data-checked") === "true",
        };
      },
    },
  ],
  toDOM(node) {
    return [
      "li",
      {
        "data-type": "taskItem",
        "data-checked": node.attrs.checked ? "true" : "false",
      },
      0,
    ];
  },
};

// `getPos` throws when the node is no longer in the document.
function getSafeNodePos(getPos: () => number): number | null {
  try {
    return getPos();
  } catch {
    return null;
  }
}

export const TaskItemView = forwardRef<
  HTMLLIElement,
  NodeViewComponentProps & { children?: ReactNode }
>(function TaskItemView({ nodeProps, children, ...htmlAttrs }, ref) {
  const { node, getPos } = nodeProps;
  const checked = node.attrs.checked === true;
  const isSelected = useIsNodeSelected();

  const handleToggle = useEditorEventCallback((view) => {
    if (!view) return;
    const pos = getSafeNodePos(getPos);
    if (pos === null) return;

    const tr = view.state.tr.setNodeMarkup(pos, undefined, {
      ...node.attrs,
      checked: !checked,
    });
    view.dispatch(tr);
  });

  return (
    <li
      ref={ref}
      {...htmlAttrs}
      data-type="taskItem"
      data-checked={checked ? "true" : "false"}
    >
      <TaskCheckbox
        checked={checked}
        isInteractive
        isSelected={isSelected}
        onToggle={handleToggle}
      />
      <div ref={nodeProps.contentDOMRef}>{children}</div>
    </li>
  );
});
