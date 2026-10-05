// The Background section of the Summaries pane (R2–R6). It edits the shared
// Background text with the Markdown editor of My Notes (KTD7). The text saves
// as the user types: one second after the last change, and at once on blur
// or a pane change. Only a user edit saves the text (KTD2). The editor gets a
// new key only on a new document: on Reload, and when another computer
// changes the text while there is no draft (KTD10).

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type Settings } from "../bindings";
import { defaultCloseSaveLimit, registerCloseSave } from "./closeSaves";
import { MarkdownField } from "./MarkdownField";
import { readOnlyReason } from "./shared";
import { Button, ErrorText, Group } from "./ui";

/// The time from the last change to the save, in milliseconds.
const saveDelay = 1000;

const hint =
  "What summaries should know: your name, your work, the names and terms you use, and words the transcript often gets wrong.";

export function Background({
  settings,
  onSettings,
  closeSaveLimit = defaultCloseSaveLimit,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
  /// Tests set a shorter limit.
  closeSaveLimit?: number;
}) {
  const saved = settings.background;
  const limit = settings.background_limit;
  const readOnly = settings.background_read_only ?? readOnlyReason(settings);
  // The unsaved text, or null when the editor shows the saved text.
  const [draft, setDraft] = useState<string | null>(null);
  // The saved text that the draft started from. When the saved text changes
  // to a text that this window did not send, another computer changed it.
  const [base, setBase] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  // The editor key. It changes only on a new document (KTD10).
  const [generation, setGeneration] = useState(0);
  // The texts that this window sent.
  const sentTexts = useRef(new Set<string>());
  // The valid text that waits for the timer, the timer, and the last save
  // that started. Each save waits for the save before it, so the saves apply
  // in the order typed.
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saving = useRef<Promise<void>>(Promise.resolve());
  const failed = useRef(false);
  const onSettingsRef = useRef(onSettings);
  onSettingsRef.current = onSettings;

  const cancel = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    pending.current = null;
  }, []);

  // Starts the save of the pending text at once. Resolves when all saves
  // that started are complete: to false if the last save failed.
  const flush = useCallback(() => {
    const sent = pending.current;
    cancel();
    if (sent !== null) {
      sentTexts.current.add(sent);
      const fail = (reason: string | null) => {
        failed.current = true;
        setSaveError(reason ?? "The Background could not be saved.");
      };
      saving.current = saving.current
        .then(async () => {
          const result = await commands.setBackground(sent);
          if (result.status === "error") {
            fail(result.error);
            return;
          }
          failed.current = false;
          setSaveError(null);
          setBase(sent);
          onSettingsRef.current(result.data);
          // Clear the draft only if it still holds the sent text, so that
          // text typed during the save stays for the next save.
          setDraft((d) => (d === sent ? null : d));
        })
        .catch((e: unknown) => {
          // A failed save must not stop the saves after it.
          console.error("The Background could not be saved:", e);
          fail(null);
        });
    }
    return saving.current.then(() => !failed.current);
  }, [cancel]);

  // A pane change unmounts the pane, and the save then runs with no wait.
  // A window close and the page events go through the close coordinator
  // (KTD9).
  useEffect(() => {
    const remove = registerCloseSave(flush, closeSaveLimit);
    return () => {
      remove();
      void flush();
    };
  }, [flush, closeSaveLimit]);

  // The draft started from a saved text that another computer then changed.
  // The editor keeps the draft.
  const changedElsewhere =
    draft !== null && draft !== saved && base !== null && saved !== base && !sentTexts.current.has(saved);

  // Such a draft waits for Reload or Keep My Text, and does not replace the
  // text of the other computer.
  useEffect(() => {
    if (changedElsewhere && pending.current !== null) cancel();
  }, [changedElsewhere, cancel]);
  // A base goes with its draft.
  useEffect(() => {
    if (draft === null) setBase(null);
  }, [draft]);

  // A change of the saved text from another computer, while there is no
  // draft, shows the new text in a new editor. An own save does not.
  const shownSaved = useRef(saved);
  useEffect(() => {
    if (saved === shownSaved.current) return;
    shownSaved.current = saved;
    if (draft === null && !sentTexts.current.has(saved)) setGeneration((g) => g + 1);
  }, [saved, draft]);

  const valid = (text: string) => [...text].length <= limit;

  // Only valid text waits for the timer; the last valid text stays saved.
  // A draft that changed on another computer does not wait for the timer:
  // only Keep My Text saves it.
  const change = (text: string) => {
    setBase((b) => b ?? saved);
    setDraft(text);
    cancel();
    if (!valid(text) || changedElsewhere) return;
    pending.current = text;
    timer.current = setTimeout(() => void flush(), saveDelay);
  };
  // Reload shows the text of the other computer and drops the draft.
  const reload = () => {
    cancel();
    setDraft(null);
    setSaveError(null);
    shownSaved.current = saved;
    setGeneration((g) => g + 1);
  };
  // Keep My Text saves the draft over the text of the other computer.
  const keep = () => {
    setBase(saved);
    if (draft !== null && valid(draft)) {
      pending.current = draft;
      void flush();
    }
  };

  const text = draft ?? saved;
  return (
    <Group footer={readOnly ? `${readOnly} The Background can’t be changed now.` : undefined}>
      <div className="settings-row px-4">
        <div className="py-3">
          <MarkdownField
            key={generation}
            id="background"
            label="Background"
            hint={hint}
            value={text}
            count={[...text].length}
            limit={limit}
            readOnly={readOnly !== null}
            rows={6}
            onChange={change}
            onBlur={() => void flush()}
          />
        </div>
      </div>
      {changedElsewhere && (
        <div className="settings-row px-4">
          <div className="flex min-h-[3.25rem] flex-wrap items-center gap-2 py-2.5">
            <div role="status" className="min-w-0 flex-1 text-[0.85rem] leading-[1.4]">
              <p className="font-medium text-caution">The Background changed on another computer.</p>
              <p className="text-label-secondary">Reload shows the new text. Keep My Text saves your text.</p>
            </div>
            <Button onClick={keep}>Keep My Text</Button>
            <Button prominent onClick={reload}>
              Reload
            </Button>
          </div>
        </div>
      )}
      {saveError && <ErrorText>{saveError}</ErrorText>}
    </Group>
  );
}
