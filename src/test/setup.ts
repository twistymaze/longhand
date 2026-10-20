// Setup file for Vitest (see the `test` block in vite.config.ts). It runs before
// each test file. It registers happy-dom as the global DOM, so that component
// tests can render React. Pure-function tests do not use the DOM and run as
// before.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterEach, vi } from "vitest";

GlobalRegistrator.register();

// happy-dom has no InputEvent.getTargetRanges. The editor (react-prosemirror)
// reads it on each `beforeinput`; an empty list means "at the selection",
// which is what a browser reports for plain typing.
if (!("getTargetRanges" in InputEvent.prototype)) {
  Object.defineProperty(InputEvent.prototype, "getTargetRanges", {
    value: () => [],
  });
}

// Testing Library moves fake timers in `waitFor` and after each user-event
// action only when a `jest` global exists. Vitest gives no `jest` global, so
// these calls wait for a fake timer that never runs. This object gives
// Testing Library the one function that it calls.
(globalThis as { jest?: unknown }).jest = {
  advanceTimersByTime: (ms: number) => vi.advanceTimersByTime(ms),
};

// React reads this flag to allow act() in a test environment.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Import after the DOM exists: Testing Library reads `document` when it loads.
const { cleanup, prettyDOM } = await import("@testing-library/react");
const { clearMocks } = await import("@tauri-apps/api/mocks");

// A failed expect prints the value it received. For a happy-dom node, the
// message must show the markup and the text of the node. Without this method,
// Vitest prints only the empty tag (for example `<p></p>`). A printer that
// goes through each property of the node to the document takes seconds on a
// large page: a `waitFor` check that fails then blocks the timers, and the
// test times out.
Object.defineProperty(Node.prototype, Symbol.for("nodejs.util.inspect.custom"), {
  configurable: true,
  value(this: Node) {
    if (this instanceof Element || this instanceof Document) {
      return prettyDOM(this, 2000, { highlight: false }) || this.nodeName;
    }
    return `${this.nodeName} ${JSON.stringify(this.textContent)}`;
  },
});

afterEach(async () => {
  cleanup();
  // An event listener that a component registers can still be in progress
  // when the component unmounts. Its unlisten then runs on a later microtask
  // and needs the mocked IPC. The mocked IPC uses only microtasks, so all of
  // these calls are complete when the next task starts. Real timers make
  // sure that this task runs when a test left fake timers on.
  vi.useRealTimers();
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  clearMocks();
});
