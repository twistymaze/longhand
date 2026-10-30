import { Plugin } from "prosemirror-state";

import { isOpenLinkClick, openLink } from "../../lib/links";

/// The address of the link under `target`, or null.
function linkHref(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  return target.closest("a[href]")?.getAttribute("href") ?? null;
}

/// Open a link in the default browser: with a click when the editor is not
/// editable, and with Command-click (Control-click on other systems) when
/// it is. A plain click in an editable editor puts the cursor in the link
/// text. The window never follows a link (see `navigation` in
/// src-tauri/src/window.rs). While the open key is held, the editor root
/// has `data-open-links`, so a link shows the hand pointer.
export function linkOpenPlugin(isEditable: () => boolean) {
  const opens = (event: MouseEvent) => !isEditable() || isOpenLinkClick(event);
  return new Plugin({
    props: {
      handleDOMEvents: {
        // Stop the selection, so that the format toolbar does not open.
        mousedown: (_view, event) => {
          if (event.button !== 0 || !linkHref(event.target) || !opens(event)) return false;
          event.preventDefault();
          return true;
        },
        click: (_view, event) => {
          const href = linkHref(event.target);
          if (!href) return false;
          // The window must not follow the link, also when it does not open.
          event.preventDefault();
          if (event.button !== 0 || !opens(event)) return false;
          openLink(href);
          return true;
        },
      },
    },
    view(view) {
      const root = view.dom as HTMLElement;
      const update = (event: KeyboardEvent | MouseEvent) =>
        root.toggleAttribute("data-open-links", isOpenLinkClick(event));
      const clear = () => root.removeAttribute("data-open-links");
      window.addEventListener("keydown", update);
      window.addEventListener("keyup", update);
      window.addEventListener("mousemove", update);
      window.addEventListener("blur", clear);
      return {
        destroy() {
          window.removeEventListener("keydown", update);
          window.removeEventListener("keyup", update);
          window.removeEventListener("mousemove", update);
          window.removeEventListener("blur", clear);
        },
      };
    },
  });
}
