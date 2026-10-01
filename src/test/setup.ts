// Preload for `bun test` (see bunfig.toml). It registers happy-dom as the
// global DOM, so that component tests can render React. Pure-function tests
// do not use the DOM and run as before.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterEach } from "bun:test";

GlobalRegistrator.register();

// happy-dom has no InputEvent.getTargetRanges. The editor (react-prosemirror)
// reads it on each `beforeinput`; an empty list means "at the selection",
// which is what a browser reports for plain typing.
if (!("getTargetRanges" in InputEvent.prototype)) {
  Object.defineProperty(InputEvent.prototype, "getTargetRanges", {
    value: () => [],
  });
}

// React reads this flag to allow act() in a test environment.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Import after the DOM exists: Testing Library reads `document` when it loads.
const { cleanup, prettyDOM } = await import("@testing-library/react");
const { clearMocks } = await import("@tauri-apps/api/mocks");

// A failed expect prints the value it received. For a happy-dom node, bun
// prints each property through to the document, which takes seconds on a
// large page. A `waitFor` check that fails then blocks the timers, and the test
// times out. Print the markup of the node instead.
Object.defineProperty(Node.prototype, Symbol.for("nodejs.util.inspect.custom"), {
  configurable: true,
  value(this: Node) {
    if (this instanceof Element || this instanceof Document) {
      return prettyDOM(this, 2000, { highlight: false }) || this.nodeName;
    }
    return `${this.nodeName} ${JSON.stringify(this.textContent)}`;
  },
});

afterEach(() => {
  cleanup();
  clearMocks();
});
