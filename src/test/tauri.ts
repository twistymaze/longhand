// One helper that mocks the Tauri IPC for component tests. Components call
// `commands.*` and `events.*` from `src/bindings.ts`; both go through the
// Tauri IPC, which this helper replaces with local handlers.
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

/** The `shortcut_status` result for a Settings window whose recording
 * shortcut is registered and whose show, Pause and widget shortcuts are off. */
export const shortcutStatuses = {
  recording: { chord: "Alt+Shift+L", registered: true, error: null },
  show_window: { chord: null, registered: false, error: null },
  pause: { chord: null, registered: false, error: null },
  widget: { chord: null, registered: false, error: null },
};

/** A handler for one command. It receives the command arguments. */
export type CommandHandler = (args: Record<string, unknown>) => unknown;

export type TauriMock = {
  /** The commands that were called, in order, with their arguments. */
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** Send an event to the listeners that the components registered. */
  emit: (event: string, payload?: unknown) => Promise<void>;
};

/**
 * Mock the Tauri IPC. `handlers` maps a command name (the snake_case name in
 * `src/bindings.ts`, for example `get_settings`) to its result. A handler that
 * throws a string gives an `{ status: "error" }` result, as the real command
 * does. A command with no handler rejects with an Error, so that a test shows
 * each command it needs. The mock handles the event commands itself.
 */
export function mockTauri(handlers: Record<string, CommandHandler> = {}): TauriMock {
  const calls: TauriMock["calls"] = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      const handler = handlers[cmd];
      if (!handler) throw new Error(`no mock for command ${cmd}`);
      return handler(args);
    },
    { shouldMockEvents: true },
  );
  return {
    calls,
    emit: (event, payload) => emit(event, payload),
  };
}
