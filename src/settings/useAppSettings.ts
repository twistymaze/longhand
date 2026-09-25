// The settings, the readiness and the model statuses, read on mount, on
// focus and when a model download ends. A folder can vanish or the
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

export type AppSettings = {
  settings: Settings | null;
  readiness: Readiness | null;
  models: ModelStatus[];
  refresh: () => Promise<void>;
  setSettings: (next: Settings) => void;
};

export function useAppSettings(): AppSettings {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [models, setModels] = useState<ModelStatus[]>([]);

  const refresh = useCallback(async () => {
    const [s, r, m] = await Promise.all([
      commands.getSettings(),
      commands.readiness(),
      commands.modelStatuses(),
    ]);
    setSettings(s);
    if (r.status === "ok") setReadiness(r.data);
    setModels(m);
  }, []);

  useEffect(() => {
    void refresh();
    window.addEventListener("focus", refresh);
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => {
      const kind = payload.update.kind;
      if (kind === "completed" || kind === "cancelled" || kind === "failed") void refresh();
    });
    return () => {
      window.removeEventListener("focus", refresh);
      void unlisten.then((f) => f());
    };
  }, [refresh]);

  return { settings, readiness, models, refresh, setSettings };
}
