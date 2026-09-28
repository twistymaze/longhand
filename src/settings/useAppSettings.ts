// The settings, the readiness and the model statuses, read on mount, on
// focus, when a model download ends and when the backend changes the
// settings (a picked model becomes the selected model). A folder can vanish or the
// microphone be revoked while a window is away; coming back to it is the
// moment to look again.

import { useCallback, useEffect, useState } from "react";
import {
  commands,
  events,
  type ModelStatus,
  type Readiness,
  type Settings,
} from "../bindings";
import { sentence } from "../processing/describe";

export type AppSettings = {
  settings: Settings | null;
  readiness: Readiness | null;
  models: ModelStatus[];
  /// Why the last read failed, or null. The last values read stay.
  error: string | null;
  refresh: () => Promise<void>;
  setSettings: (next: Settings) => void;
};

export function useAppSettings(): AppSettings {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [s, r, m] = await Promise.all([
        commands.getSettings(),
        commands.readiness(),
        commands.modelStatuses(),
      ]);
      setSettings(s);
      if (r.status === "ok") setReadiness(r.data);
      setModels(m);
      setError(r.status === "error" ? sentence(r.error) : null);
    } catch (e) {
      // The commands without a Result reject with the error text.
      setError(sentence(typeof e === "string" ? e : e instanceof Error ? e.message : "The settings could not be read"));
    }
  }, []);

  useEffect(() => {
    void refresh();
    window.addEventListener("focus", refresh);
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => {
      const kind = payload.update.kind;
      if (kind === "completed" || kind === "cancelled" || kind === "failed") void refresh();
    });
    const unlistenSettings = events.settingsChanged.listen(() => void refresh());
    return () => {
      window.removeEventListener("focus", refresh);
      void unlisten.then((f) => f());
      void unlistenSettings.then((f) => f());
    };
  }, [refresh]);

  return { settings, readiness, models, error, refresh, setSettings };
}
