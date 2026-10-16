// The settings, the readiness and the model statuses, read on mount, on
// focus, when a model download ends and when the backend changes the
// settings (a picked model becomes the selected model). A folder can vanish or the
// microphone be revoked while a window is away; coming back to it is the
// moment to look again.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  events,
  type ModelStatus,
  type Readiness,
  type RecognitionConfig,
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
};

export function useAppSettings(): AppSettings {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [error, setError] = useState<string | null>(null);
  const readVersion = useRef(0);

  const refresh = useCallback(async () => {
    const version = ++readVersion.current;
    try {
      const [s, r, m] = await Promise.all([
        commands.getSettings(),
        commands.readiness(),
        commands.modelStatuses(),
      ]);
      if (version !== readVersion.current) return;
      setSettings(s);
      if (r.status === "ok") setReadiness(r.data);
      setModels(m);
      setError(r.status === "error" ? sentence(r.error) : null);
    } catch (e) {
      if (version !== readVersion.current) return;
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
      ++readVersion.current;
      window.removeEventListener("focus", refresh);
      void unlisten.then((f) => f());
      void unlistenSettings.then((f) => f());
    };
  }, [refresh]);

  return { settings, readiness, models, error, refresh };
}

// Keep the queue and its feedback in the window while panes change.
export function useRecognitionDefaultsSave(onSaved: () => Promise<void>) {
  const queue = useRef(Promise.resolve());
  const version = useRef(0);
  const [savingModel, setSavingModel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Resolves when this request ends: saved, failed, or not sent because a
  // later request replaced it.
  const save = (config: RecognitionConfig): Promise<void> => {
    const request = ++version.current;
    setSavingModel(config.asr_model);
    setError(null);
    queue.current = queue.current.then(async () => {
      if (request !== version.current) return;
      try {
        const result = await commands.setRecognitionDefaults(config);
        if (request !== version.current) return;
        if (result.status === "error") setError(sentence(result.error));
        else await onSaved();
      } catch {
        if (request === version.current)
          setError("Could not save the transcription defaults. Change the choice to try again.");
      } finally {
        if (request === version.current) setSavingModel(null);
      }
    });
    return queue.current;
  };

  return { save, savingModel, error };
}

export type RecognitionDefaultsSave = ReturnType<typeof useRecognitionDefaultsSave>;
