// The Recording pane (R1, R6, KTD15): the usual microphone and the default of
// the Computer audio switch. A new note starts with both.

import { type KeyboardEvent, useCallback, useEffect, useRef, useState } from "react";
import { commands, type MicrophoneChoice, type MicrophoneList, type Settings } from "../bindings";
import { sentence } from "../processing/describe";
import { CheckIcon } from "./icons";
import { ErrorText, Group, Pane, Row, Switch } from "./ui";

/// One row of the Microphone group. `choice` is null for "System default".
export type MicrophoneOption = {
  key: string;
  label: string;
  detail: string | null;
  choice: MicrophoneChoice | null;
};

/// The rows of the Microphone group: "System default" first, then the
/// connected inputs in the order of the list. Inputs with the same name get a
/// number after the name, in the order of the device list. A chosen input
/// that is not connected is added after the inputs as "Not connected".
/// `selected` is the key of the row that holds the choice.
export function microphoneOptions(
  list: MicrophoneList,
  chosen: MicrophoneChoice | null,
): { options: MicrophoneOption[]; selected: string } {
  const count = new Map<string, number>();
  for (const input of list.inputs) count.set(input.name, (count.get(input.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  const inputs: MicrophoneOption[] = list.inputs.map((input) => {
    const n = (seen.get(input.name) ?? 0) + 1;
    seen.set(input.name, n);
    return {
      key: `input:${input.id}`,
      label: (count.get(input.name) ?? 0) > 1 ? `${input.name} ${n}` : input.name,
      detail: null,
      // The label is the device name, so the backend can find the input by name.
      choice: { id: input.id, label: input.name },
    };
  });

  const current = list.inputs.find((i) => i.id === list.default_id);
  const options: MicrophoneOption[] = [
    { key: "default", label: "System default", detail: current ? `Currently ${current.name}` : null, choice: null },
    ...inputs,
  ];
  if (!chosen) return { options, selected: "default" };

  // The backend finds the input by its ID first, then by the exact name.
  const match =
    inputs.find((o) => o.choice?.id === chosen.id) ?? inputs.find((o) => o.choice?.label === chosen.label);
  if (match) return { options, selected: match.key };
  const missing = { key: "missing", label: chosen.label, detail: "Not connected", choice: chosen };
  return { options: [...options, missing], selected: missing.key };
}

function MicrophoneGroup({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const [list, setList] = useState<MicrophoneList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const radios = useRef(new Map<string, HTMLButtonElement>());

  const read = useCallback(async () => {
    const result = await commands.listMicrophones();
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      setList(result.data);
    }
  }, []);

  // A microphone can be connected or removed while the window is away.
  useEffect(() => {
    void read();
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [read]);

  const { options, selected } = microphoneOptions(list ?? { default_id: null, inputs: [] }, settings.microphone);

  const choose = async (option: MicrophoneOption) => {
    // WKWebView does not focus a clicked button, and the arrow keys go to the
    // focused element.
    radios.current.get(option.key)?.focus();
    if (option.key === selected) return;
    const result = await commands.setMicrophone(option.choice);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      onSettings(result.data);
    }
  };

  // The up and down arrows move the choice, as in a radio group.
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const at = options.findIndex((o) => o.key === selected);
    const next = options[Math.min(options.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)))];
    void choose(next);
  };

  return (
    <Group
      header="Microphone"
      footer="Each new note records from this microphone. If it is not connected, the note uses the system default."
    >
      <div role="radiogroup" aria-label="Microphone" onKeyDown={onKey}>
        {options.map((o) => {
          const on = o.key === selected;
          return (
            <div
              key={o.key}
              onClick={() => void choose(o)}
              className="settings-row flex cursor-default items-stretch pl-2 hover:bg-row-hover active:bg-fill"
            >
              <span aria-hidden className="flex w-6 shrink-0 items-center justify-center text-accent">
                {on && <CheckIcon className="size-[14px]" />}
              </span>
              <div className="flex min-h-[2.75rem] min-w-0 flex-1 items-center py-2 pr-4 pl-1">
                <button
                  ref={(el) => {
                    if (el) radios.current.set(o.key, el);
                    else radios.current.delete(o.key);
                  }}
                  type="button"
                  role="radio"
                  aria-label={o.label}
                  aria-checked={on}
                  tabIndex={on ? 0 : -1}
                  className="min-w-0 flex-1 text-left outline-offset-2 focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <span className="block truncate font-medium">{o.label}</span>
                  {o.detail && (
                    <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-label-secondary">{o.detail}</span>
                  )}
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

function SystemAudio({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const [error, setError] = useState<string | null>(null);
  const change = async (enabled: boolean) => {
    const result = await commands.setSystemAudioDefault(enabled);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      onSettings(result.data);
    }
  };
  return (
    <Group footer="Computer audio records all sound that the computer plays, from any app, such as the other people on a call. You can turn it on or off in each note.">
      <Row label="Computer audio" detail="Start each new note with Computer audio on">
        <Switch
          label="Computer audio"
          checked={settings.system_audio_default}
          onChange={(v) => void change(v)}
        />
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

export function RecordingPane({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  return (
    <Pane title="Recording">
      <MicrophoneGroup settings={settings} onSettings={onSettings} />
      <SystemAudio settings={settings} onSettings={onSettings} />
    </Pane>
  );
}
