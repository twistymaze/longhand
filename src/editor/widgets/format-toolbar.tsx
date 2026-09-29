import {
  autoUpdate,
  computePosition,
  flip,
  offset,
  shift,
  type VirtualElement,
} from "@floating-ui/dom";
import {
  useEditorEffect,
  useEditorEventCallback,
  useEditorEventListener,
  useEditorState,
} from "@handlewithcare/react-prosemirror";
import { toggleMark } from "prosemirror-commands";
import type { MarkType } from "prosemirror-model";
import type { EditorState, Transaction } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { type ComponentType, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { cn } from "../cn";
import {
  BoldIcon,
  HighlightIcon,
  ItalicIcon,
  LinkIcon,
  StrikeIcon,
} from "../icons";
import { schema } from "../note/schema";
import { isValidUrl, normalizeUrlHref } from "../plugins/url";

// The toolbar that appears over a text selection. See PROVENANCE.md.

const OVERFLOW_CLIP = /(auto|scroll|overlay|hidden|clip)/;

export function getClipBoundary(element: Element): Element {
  let current = element.parentElement;
  while (current && current !== document.documentElement) {
    const { overflow, overflowX, overflowY } = getComputedStyle(current);
    if (
      OVERFLOW_CLIP.test(overflowY) ||
      OVERFLOW_CLIP.test(overflowX) ||
      OVERFLOW_CLIP.test(overflow)
    ) {
      return current;
    }
    current = current.parentElement;
  }
  return element;
}

export function createSelectionVirtualElement(
  view: EditorView,
  from: number,
  to: number,
): VirtualElement {
  const start = view.coordsAtPos(from);
  const end = view.coordsAtPos(to);
  return {
    contextElement: view.dom,
    getBoundingClientRect: () =>
      new DOMRect(
        Math.min(start.left, end.left),
        start.top,
        Math.abs(end.right - start.left),
        end.bottom - start.top,
      ),
  };
}

export function isMarkActive(state: EditorState, type: MarkType): boolean {
  const { from, $from, to, empty } = state.selection;
  if (empty) {
    return !!type.isInSet(state.storedMarks || $from.marks());
  }
  return state.doc.rangeHasMark(from, to, type);
}

// Put a link on the selected text, or remove the link when `href` is null.
// Text that is not a web address gives no link.
export function setLink(
  state: EditorState,
  dispatch: (tr: Transaction) => void,
  href: string | null,
): boolean {
  const { from, to, empty } = state.selection;
  if (empty) return false;
  const linkType = schema.marks.link;
  const tr = state.tr.removeMark(from, to, linkType);
  if (href !== null) {
    const text = href.trim();
    if (!isValidUrl(text)) return false;
    tr.addMark(
      from,
      to,
      linkType.create({ href: normalizeUrlHref(text), target: null }),
    );
  }
  dispatch(tr);
  return true;
}

const TOOLBAR_BUTTONS: {
  id: string;
  label: string;
  icon: ComponentType<{ className?: string }>;
  markType: MarkType;
}[] = [
  { id: "bold", label: "Bold", icon: BoldIcon, markType: schema.marks.bold },
  { id: "italic", label: "Italic", icon: ItalicIcon, markType: schema.marks.italic },
  { id: "strike", label: "Strikethrough", icon: StrikeIcon, markType: schema.marks.strike },
  { id: "highlight", label: "Highlight", icon: HighlightIcon, markType: schema.marks.highlight },
];

const buttonClass = (active: boolean) =>
  cn([
    "flex size-7 items-center justify-center rounded-md",
    "cursor-default border-none transition-colors",
    active
      ? "bg-accent text-white"
      : "bg-transparent text-label-secondary hover:bg-fill hover:text-label active:bg-fill-pressed",
  ]);

export function FormatToolbar() {
  const toolbarRef = useRef<HTMLDivElement>(null);
  const cleanupRef = useRef<(() => void) | null>(null);
  const [linkDraft, setLinkDraft] = useState<string | null>(null);
  const [linkInvalid, setLinkInvalid] = useState(false);

  // During an IME composition, the DOM selection is a range over the composed
  // text. That range is not a real selection, so the toolbar stays hidden
  // until the composition ends. ProseMirror applies the committed text and
  // the last selection in its own 20 ms timer after compositionend. Thus the
  // gate opens at the first selection change after compositionend, or after
  // 50 ms. After that time, a range cannot be the composition range.
  const [isComposing, setIsComposing] = useState(false);
  const compositionEndState = useRef<EditorState | null>(null);
  const releaseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelReleaseTimer = () => {
    if (releaseTimer.current != null) {
      clearTimeout(releaseTimer.current);
      releaseTimer.current = null;
    }
  };
  const releaseComposition = () => {
    compositionEndState.current = null;
    cancelReleaseTimer();
    setIsComposing(false);
  };
  // Stop the timer when the toolbar unmounts.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => cancelReleaseTimer, []);
  useEditorEventListener("compositionstart", () => {
    compositionEndState.current = null;
    cancelReleaseTimer();
    setIsComposing(true);
  });
  useEditorEventListener("compositionend", (view) => {
    compositionEndState.current = view.state;
    releaseTimer.current = setTimeout(releaseComposition, 50);
  });
  useEditorEffect((view) => {
    const snapshot = compositionEndState.current;
    if (snapshot && !view.state.selection.eq(snapshot.selection)) {
      releaseComposition();
    }
  });

  const editorState = useEditorState();
  const shouldShowToolbar = editorState
    ? !editorState.selection.empty && !isComposing
    : false;

  if (!shouldShowToolbar && linkDraft !== null) {
    setLinkDraft(null);
    setLinkInvalid(false);
  }

  const toggle = useEditorEventCallback((view, markType: MarkType) => {
    if (!view) return;
    toggleMark(markType)(view.state, (tr) => view.dispatch(tr));
    view.focus();
  });

  const applyLink = useEditorEventCallback((view, href: string | null) => {
    if (!view) return false;
    const done = setLink(view.state, (tr) => view.dispatch(tr), href);
    if (done) view.focus();
    return done;
  });

  const cancelLink = useEditorEventCallback((view) => {
    setLinkDraft(null);
    setLinkInvalid(false);
    view?.focus();
  });

  useEditorEffect((view) => {
    if (!view || !shouldShowToolbar) {
      cleanupRef.current?.();
      cleanupRef.current = null;
      return;
    }

    const toolbar = toolbarRef.current;
    if (!toolbar) return;

    const { from, to } = view.state.selection;
    const referenceEl = createSelectionVirtualElement(view, from, to);
    // Portaled toolbars clip to the viewport by default, which includes window
    // chrome. Stay inside the editor scrollport so the menu flips below the
    // first line instead of covering traffic lights.
    const boundary = getClipBoundary(view.dom);

    const update = () => {
      void computePosition(referenceEl, toolbar, {
        placement: "top",
        strategy: "fixed",
        middleware: [
          offset(8),
          flip({
            boundary,
            fallbackPlacements: ["bottom"],
            padding: 8,
          }),
          shift({ boundary, padding: 8 }),
        ],
      }).then(({ x, y }) => {
        Object.assign(toolbar.style, {
          left: `${x}px`,
          top: `${y}px`,
        });
      });
    };

    cleanupRef.current?.();
    cleanupRef.current = autoUpdate(referenceEl, toolbar, update);
    update();
  });

  if (!shouldShowToolbar || !editorState) return null;

  const linkActive = isMarkActive(editorState, schema.marks.link);

  return createPortal(
    <div
      ref={toolbarRef}
      role="toolbar"
      aria-label="Format selection"
      data-editor-escape-consumer
      className="fixed z-50 flex items-center gap-0.5 rounded-lg bg-group p-1 shadow-[0_0_0_0.5px_var(--separator),0_4px_16px_rgb(0_0_0/0.14)]"
      style={{ top: 0, left: 0 }}
      onMouseDown={(e) => {
        if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
      }}
    >
      {linkDraft === null ? (
        <>
          {TOOLBAR_BUTTONS.map((button) => {
            const active = isMarkActive(editorState, button.markType);
            return (
              <button
                key={button.id}
                type="button"
                aria-label={button.label}
                aria-pressed={active}
                className={buttonClass(active)}
                onClick={() => toggle(button.markType)}
              >
                <button.icon className="size-4" />
              </button>
            );
          })}
          <button
            type="button"
            aria-label={linkActive ? "Remove link" : "Link"}
            aria-pressed={linkActive}
            className={buttonClass(linkActive)}
            onClick={() => (linkActive ? applyLink(null) : setLinkDraft(""))}
          >
            <LinkIcon className="size-4" />
          </button>
        </>
      ) : (
        <input
          autoFocus
          type="url"
          aria-label="Link address"
          aria-invalid={linkInvalid}
          placeholder="Paste or type a link"
          value={linkDraft}
          className={cn([
            "h-7 w-56 rounded-md bg-control px-2 text-label outline-none",
            linkInvalid && "ring-1 ring-negative",
          ])}
          onChange={(e) => {
            setLinkDraft(e.target.value);
            setLinkInvalid(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              if (applyLink(linkDraft)) setLinkDraft(null);
              else setLinkInvalid(true);
            } else if (e.key === "Escape") {
              e.preventDefault();
              cancelLink();
            }
          }}
          onBlur={() => {
            setLinkDraft(null);
            setLinkInvalid(false);
          }}
        />
      )}
    </div>,
    document.body,
  );
}
