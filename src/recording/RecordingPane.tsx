import type { ModelStatus, Readiness, Settings } from "../bindings";
import { formatChord } from "../lib/timeline";
import { Rail } from "./Rail";
import { Scratchpad } from "./Scratchpad";
import type { Capture } from "./useCapture";

const isMac = navigator.userAgent.includes("Mac");

export function RecordingPane({
  capture,
  readiness,
  models,
  settings,
}: {
  capture: Capture;
  readiness: Readiness;
  models: ModelStatus[];
  settings: Settings;
}) {
  return (
    <div className="mx-auto flex h-full max-w-[34rem] flex-col gap-3 px-6 pt-5 pb-4">
      <Rail
        capture={capture}
        readiness={readiness}
        models={models}
        chord={formatChord(settings.shortcut, isMac)}
      />
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[10px] bg-group shadow-[0_0_0_0.5px_var(--separator)]">
        <Scratchpad sessionId={capture.status?.session_id ?? null} />
      </div>
    </div>
  );
}
