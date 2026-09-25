// Preload for `bun test` (see bunfig.toml). It registers happy-dom as the
// global DOM, so that component tests can render React. Pure-function tests
// do not use the DOM and run as before.
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterEach } from "bun:test";

GlobalRegistrator.register();

// React reads this flag to allow act() in a test environment.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Import after the DOM exists: Testing Library reads `document` when it loads.
const { cleanup } = await import("@testing-library/react");
const { clearMocks } = await import("@tauri-apps/api/mocks");

afterEach(() => {
  cleanup();
  clearMocks();
});
