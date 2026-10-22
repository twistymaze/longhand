import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { StrictMode, useState } from "react";
import type { CredentialStatus, EndpointCheck, EndpointModels, Provider, ProviderEntry, Settings } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { ProviderForm } from "./ProviderForm";

// The pane shows Repair Keychain Access only on a Mac. Set a Mac user agent
// for these tests, then restore the user agent for the other test files.
beforeAll(() => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
});
afterAll(() => {
  delete (navigator as { userAgent?: string }).userAgent;
});

const empty: ProviderEntry = { address: null, model: "", context_limit_tokens: null };

const baseSettings: Settings = {
  settings_schema_version: 7,
  interface_zoom_percent: 100,
  shortcut: "Alt+Shift+L",
  show_window_shortcut: null,
  pause_shortcut: null,
  widget_shortcut: null,
  storage_folder: "/notes",
  active_provider: null,
  providers: {
    openai: empty,
    anthropic: empty,
    gemini: empty,
    openrouter: empty,
    ollama: empty,
    lm_studio: empty,
    custom: empty,
  },
  default_speaker_count: null,
  asr_model: "a",
  live_transcript: true,
  speaker_source: "pyannote",
  speaker_processing_mode: "after_stop",
  recognition_language: null,
  chosen_recognition: { asr_model: "a", live_transcript: true, speaker_source: "pyannote", speaker_processing_mode: "after_stop", recognition_language: null, default_speaker_count: null },
  recognition: { differs: false, chosen_not_known: false, waiting_for: [], failure: null },
  microphone: null,
  system_audio_default: false,
  apps: [],
  new_note_records: true,
  alert_sound: true,
  show_recording_widget: true,
  hide_from_dock: false,
  check_updates_automatically: true,
  audio_removal: { kind: "after_days", days: 30 },
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
  background: "",
  background_limit: 4000,
  background_read_only: null,
};

const listed = (...ids: string[]): EndpointModels => ({ kind: "listed", ids });
const keyNeeded: EndpointModels = { kind: "not-requested", reason: "key-needed" };

const hosts: Record<Provider, string> = {
  openai: "api.openai.com",
  anthropic: "api.anthropic.com",
  gemini: "generativelanguage.googleapis.com",
  openrouter: "openrouter.ai",
  ollama: "localhost:11434",
  lm_studio: "localhost:1234",
  custom: "",
};

type Deferred = { provider: Provider; resolve: (value: EndpointModels | EndpointCheck) => void };

/// A fake backend. `check` gives the result of each check by the provider
/// and the index of the check. A result of "defer" keeps the check open
/// until the test resolves it through `pending`. An `EndpointCheck` result
/// that has an address saves that address for Custom, as the backend does.
function backend({
  active = null,
  entries = {},
  check = () => keyNeeded,
  credentials = {},
  over = {},
}: {
  active?: Provider | null;
  entries?: Partial<Record<Provider, Partial<ProviderEntry>>>;
  check?: (provider: Provider, index: number) => EndpointModels | EndpointCheck | "defer";
  credentials?: Partial<Record<Provider, CredentialStatus>>;
  over?: Record<string, CommandHandler>;
} = {}) {
  const providers = { ...baseSettings.providers };
  for (const [id, entry] of Object.entries(entries)) providers[id as Provider] = { ...empty, ...entry };
  let current: Settings = { ...baseSettings, active_provider: active, providers };
  const keys: Partial<Record<Provider, CredentialStatus>> = { ...credentials };
  let checkIndex = 0;
  const pending: Deferred[] = [];
  const hostOf = (p: Provider) => (p === "custom" ? new URL(current.providers.custom.address ?? "http://none").host : hosts[p]);
  const setEntry = (p: Provider, change: Partial<ProviderEntry>) => {
    current = { ...current, providers: { ...current.providers, [p]: { ...current.providers[p], ...change } } };
    return current;
  };
  const finish = (p: Provider, result: EndpointModels | EndpointCheck): EndpointCheck => {
    if ("models" in result) {
      if (p === "custom" && result.address) setEntry(p, { address: result.address });
      return result;
    }
    return { models: result, address: null };
  };
  const mock = mockTauri({
    get_settings: () => current,
    choose_provider: ({ provider }) => {
      current = { ...current, active_provider: provider as Provider | null };
      return current;
    },
    set_provider_address: ({ provider, address }) =>
      setEntry(provider as Provider, { address: address === "" ? null : (address as string) }),
    set_provider_model: ({ provider, model }) => setEntry(provider as Provider, { model: model as string }),
    set_provider_context_limit: ({ provider, limit }) =>
      setEntry(provider as Provider, { context_limit_tokens: limit as number | null }),
    provider_credential_status: ({ provider }) =>
      keys[provider as Provider] ?? { kind: "absent", host: hostOf(provider as Provider) },
    set_provider_credential: ({ provider }) => {
      const p = provider as Provider;
      keys[p] = { kind: "present", host: hostOf(p), length: 10, fingerprint: "abcd" };
      return null;
    },
    delete_provider_credential: ({ provider }) => {
      delete keys[provider as Provider];
      return null;
    },
    repair_keychain_access: () => null,
    check_provider: ({ provider }) => {
      const p = provider as Provider;
      const result = check(p, checkIndex++);
      if (result !== "defer") return finish(p, result);
      return new Promise<EndpointModels | EndpointCheck>((resolve) => pending.push({ provider: p, resolve })).then(
        (r) => finish(p, r),
      );
    },
    ...over,
  });
  mockWindows("settings");
  const checkCalls = () => mock.calls.filter((c) => c.cmd === "check_provider").length;
  const setKey = (p: Provider, next: CredentialStatus) => {
    keys[p] = next;
  };
  const argsOf = (cmd: string) => mock.calls.filter((c) => c.cmd === cmd).map((c) => c.args);
  return { mock, pending, checkCalls, initial: current, setKey, argsOf };
}

function Harness({ initial }: { initial: Settings }) {
  const [settings, setSettings] = useState(initial);
  return <ProviderForm settings={settings} onSettings={setSettings} />;
}

const status = () => screen.getByRole("status");
const modelIsPicker = () => screen.queryByRole("combobox", { name: "Model" }) !== null;
const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const has = (label: string) => screen.queryByLabelText(label) !== null;
const choose = (user: ReturnType<typeof userEvent.setup>, provider: Provider | "") =>
  user.selectOptions(screen.getByLabelText("Provider"), provider);

describe("the AI Provider pane rows", () => {
  test("a cloud provider shows API key and Model rows and no Address row", async () => {
    const b = backend({ active: "anthropic" });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Enter an API key"));
    expect(has("API key")).toBe(true);
    expect(has("Model")).toBe(true);
    expect(has("Address")).toBe(false);
    expect(b.argsOf("check_provider")).toEqual([{ provider: "anthropic" }]);
    expect(await screen.findByText("No key saved for api.anthropic.com")).toBeTruthy();
  });

  test("Covers AE5. Ollama shows Address and Model rows, no API key row, and names the address it cannot reach", async () => {
    const b = backend({ active: "ollama", check: () => ({ kind: "unreachable" }) });
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toContain("Can't reach Ollama at localhost:11434. Check that Ollama is open."),
    );
    expect(field("Address").value).toBe("http://localhost:11434");
    expect(has("Model")).toBe(true);
    expect(has("API key")).toBe(false);
    expect(b.mock.calls.some((c) => c.cmd === "provider_credential_status")).toBe(false);
  });

  test("Custom shows Address, API key and Model rows", async () => {
    const b = backend({
      active: "custom",
      entries: { custom: { address: "https://gw.example.com/v1" } },
      check: () => listed("a"),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · 1 model"));
    expect(field("Address").value).toBe("https://gw.example.com/v1");
    expect(has("API key")).toBe(true);
    expect(has("Model")).toBe(true);
    expect(await screen.findByText("No key saved for gw.example.com")).toBeTruthy();
  });

  test("Custom with no address hides the API key row and asks for an address", async () => {
    const b = backend({ active: "custom", check: () => ({ kind: "not-requested", reason: "no-address" }) });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Enter an address"));
    expect(field("Address").value).toBe("");
    expect(has("API key")).toBe(false);
  });

  test("the order is Provider, Address, API key, Model, then the status row", async () => {
    const b = backend({ active: "custom", entries: { custom: { address: "https://gw.example.com/v1" } }, check: () => listed() });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected"));
    const nodes = [field("Provider"), field("Address"), field("API key"), field("Model"), status()];
    for (let i = 1; i < nodes.length; i++) {
      expect(nodes[i - 1].compareDocumentPosition(nodes[i]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });

  test("None hides the other rows", async () => {
    const b = backend({ active: "anthropic" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Enter an API key"));
    await choose(user, "");
    await waitFor(() => expect(has("Model")).toBe(false));
    expect(b.argsOf("choose_provider")).toEqual([{ provider: null }]);
    expect(field("Provider").value).toBe("");
    expect(has("API key")).toBe(false);
    expect(has("Address")).toBe(false);
    expect(has("Context limit")).toBe(false);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByText("With no provider, notes are saved without a summary.")).toBeTruthy();
  });

  test("the plain HTTP warning shows for another computer and not for this computer", async () => {
    const warning = "Plain HTTP: the transcript and API key cross the network unencrypted.";
    const b = backend({ active: "custom", entries: { custom: { address: "http://example.com" } }, check: () => listed() });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    expect(await screen.findByText(warning)).toBeTruthy();
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://127.0.0.1:8080{Enter}");
    await waitFor(() => expect(screen.queryByText(warning)).toBeNull());
    expect(b.argsOf("set_provider_address")).toEqual([{ provider: "custom", address: "http://127.0.0.1:8080" }]);
  });
});

describe("the status row", () => {
  test("checking", async () => {
    const b = backend({ active: "openai", check: () => "defer" });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    expect(status().textContent).toBe("Checking…");
    await act(async () => b.pending[0].resolve(listed("a", "b")));
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    expect(modelIsPicker()).toBe(true);
  });

  test("not set up keeps the remembered model in a text field", async () => {
    const b = backend({ active: "anthropic", entries: { anthropic: { model: "claude-haiku-5-5" } } });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toBe("Enter an API key"));
    expect(modelIsPicker()).toBe(false);
    expect(field("Model").value).toBe("claude-haiku-5-5");
  });

  test("a refused key keeps the remembered model in a text field", async () => {
    const b = backend({
      active: "anthropic",
      entries: { anthropic: { model: "claude-haiku-5-5" } },
      check: () => ({ kind: "key-rejected" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Anthropic refused this API key"));
    expect(status().querySelector(".text-negative")).not.toBeNull();
    expect(modelIsPicker()).toBe(false);
    expect(field("Model").value).toBe("claude-haiku-5-5");
  });

  test("a cloud provider that cannot be reached", async () => {
    const b = backend({ active: "gemini", check: () => ({ kind: "unreachable" }) });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toBe("Can't reach Gemini"));
  });

  test("a Custom server that cannot be reached names its host", async () => {
    const b = backend({
      active: "custom",
      entries: { custom: { address: "https://gw.example.com/v1" } },
      check: () => ({ kind: "unreachable" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toBe("Can't reach gw.example.com"));
  });

  test("connected with no model list makes Model a text field that saves on blur", async () => {
    const b = backend({
      active: "custom",
      entries: { custom: { address: "https://gw.example.com" } },
      check: () => ({ kind: "chat-only" }),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · no model list. Type a model name."));
    expect(modelIsPicker()).toBe(false);
    await user.type(field("Model"), "my-route");
    await user.tab();
    await waitFor(() => expect(b.argsOf("set_provider_model")).toEqual([{ provider: "custom", model: "my-route" }]));
  });

  test("a server that does not list models asks for a model name", async () => {
    const b = backend({
      active: "lm_studio",
      check: () => ({ kind: "not-listed" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toContain("LM Studio doesn't list its models. Type a model name, or check the address."),
    );
    expect(modelIsPicker()).toBe(false);
  });

  test("a locked Keychain shows the locked text, a text field and the Repair control", async () => {
    const b = backend({
      active: "openai",
      credentials: { openai: { kind: "needs_unlock", message: "The login keychain is locked." } },
      check: () => ({ kind: "not-requested", reason: "keychain-locked" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toContain("Keychain is locked. Use Repair Keychain Access above."),
    );
    expect(modelIsPicker()).toBe(false);
    expect(await screen.findByRole("button", { name: "Repair Keychain Access" })).toBeTruthy();
    expect(status().querySelector("button")).toBeNull();
  });

  test("a failed Keychain shows the message of the API key row", async () => {
    const b = backend({
      active: "openai",
      credentials: { openai: { kind: "failed", message: "Keychain refused access to the key." } },
      check: () => ({ kind: "not-requested", reason: "keychain-failed" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Keychain refused access to the key."));
  });

  test("a check that throws shows the error in the negative tone", async () => {
    const b = backend({
      active: "openai",
      over: {
        check_provider: () => {
          throw new Error("The check command failed.");
        },
      },
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("The check command failed."));
    expect(status().querySelector(".text-negative")).not.toBeNull();
  });
});

describe("the model", () => {
  test("a first choice of a provider shows no model", async () => {
    const b = backend({ check: () => listed("gpt-5", "gpt-5-mini") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await choose(user, "openai");
    await waitFor(() => expect(status().textContent).toBe("Connected · 2 models"));
    expect(b.argsOf("choose_provider")).toEqual([{ provider: "openai" }]);
    expect((screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe("");
    expect(b.mock.calls.some((c) => c.cmd === "set_provider_model")).toBe(false);
  });

  test("a remembered model that the list lacks stays selected, and the status row says so", async () => {
    const b = backend({
      active: "anthropic",
      entries: { anthropic: { model: "claude-old" } },
      check: () => listed("claude-a", "claude-b"),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toBe("Connected · 2 models · Anthropic doesn't list claude-old"),
    );
    expect(status().querySelector(".text-caution")).not.toBeNull();
    expect((screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe("claude-old");
    expect(screen.queryByText("This server doesn't list this model")).toBeNull();
  });

  test("a choice in the picker saves through the Model setter only", async () => {
    const b = backend({ active: "openai", check: () => listed("gpt-5", "gpt-5-mini") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    await user.click(screen.getByRole("combobox", { name: "Model" }));
    await user.click(screen.getByRole("option", { name: "gpt-5-mini" }));
    await waitFor(() => expect(b.argsOf("set_provider_model")).toEqual([{ provider: "openai", model: "gpt-5-mini" }]));
    expect(b.mock.calls.some((c) => c.cmd === "set_provider_address")).toBe(false);
  });

  test("a list that arrives while the user types does not replace the text field", async () => {
    const b = backend({ active: "openai", check: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    await user.type(field("Model"), "gpt-4");
    await act(async () => b.pending[0].resolve(listed("a", "b")));
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    expect(modelIsPicker()).toBe(false);
    expect(field("Model").value).toBe("gpt-4");
    await user.tab();
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    expect(b.argsOf("set_provider_model")).toEqual([{ provider: "openai", model: "gpt-4" }]);
    await waitFor(() => expect((screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe("gpt-4"));
  });
});

describe("the checks", () => {
  test("a key save checks again and shows the picker", async () => {
    const b = backend({ active: "openai", check: (_, i) => (i === 0 ? keyNeeded : "defer") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Enter an API key"));
    await user.type(field("API key"), "sk-test");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(b.checkCalls()).toBe(2));
    expect(b.argsOf("set_provider_credential")).toEqual([{ provider: "openai", value: "sk-test" }]);
    expect(status().textContent).toContain("Checking…");
    await act(async () => b.pending[0].resolve(listed("a", "b", "c")));
    await waitFor(() => expect(status().textContent).toContain("Connected · 3 models"));
    expect(modelIsPicker()).toBe(true);
    expect(await screen.findByText(/Saved in Keychain for api\.openai\.com/)).toBeTruthy();
  });

  test("Remove deletes the key of the provider and checks again", async () => {
    const b = backend({
      active: "openai",
      credentials: { openai: { kind: "present", host: "api.openai.com", length: 10, fingerprint: "abcd" } },
      check: (_, i) => (i === 0 ? listed("a") : keyNeeded),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await user.click(await screen.findByRole("button", { name: "Remove" }));
    await waitFor(() => expect(status().textContent).toBe("Enter an API key"));
    expect(b.argsOf("delete_provider_credential")).toEqual([{ provider: "openai" }]);
    expect(await screen.findByText("No key saved for api.openai.com")).toBeTruthy();
  });

  test("an address commit checks again and shows the picker", async () => {
    const b = backend({ active: "ollama", check: (_, i) => (i === 0 ? { kind: "unreachable" } : listed("llama3", "qwen")) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Can't reach"));
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://studio.local:11434{Enter}");
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    expect(b.checkCalls()).toBe(2);
    expect(modelIsPicker()).toBe(true);
    expect(b.argsOf("set_provider_address")).toEqual([{ provider: "ollama", address: "http://studio.local:11434" }]);
  });

  test("committing the unchanged usual address saves nothing", async () => {
    const b = backend({ active: "ollama", check: () => listed() });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected"));
    await user.click(field("Address"));
    await user.keyboard("{Enter}");
    await user.tab();
    expect(b.mock.calls.some((c) => c.cmd === "set_provider_address")).toBe(false);
    expect(b.checkCalls()).toBe(1);
  });

  test("a window close saves a typed address and does not wait for the check", async () => {
    const b = backend({
      active: "ollama",
      check: (_, i) => (i === 0 ? listed() : "defer"),
      over: { "plugin:window|destroy": () => null, log_from_webview: () => null },
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected"));
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://studio.local:11434");
    await act(() => b.mock.emit("tauri://close-requested"));
    await waitFor(() => expect(b.mock.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true), {
      timeout: 3000,
    });
    expect(b.argsOf("set_provider_address")).toEqual([{ provider: "ollama", address: "http://studio.local:11434" }]);
    const logs = b.mock.calls.filter((c) => c.cmd === "log_from_webview").map((c) => c.args.message);
    expect(logs).toContain("the save is complete");
    await act(async () => b.pending.forEach((p) => p.resolve({ kind: "not-listed" })));
  });

  test("an address edit that is open at a provider change saves to the old provider", async () => {
    const b = backend({ active: "ollama", check: () => listed() });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected"));
    // A change event with no blur, as when WKWebView does not move the focus.
    fireEvent.change(field("Address"), { target: { value: "http://studio.local:11434" } });
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "lm_studio" } });
    await waitFor(() => expect(field("Address").value).toBe("http://localhost:1234/v1"));
    await waitFor(() =>
      expect(b.argsOf("set_provider_address")).toEqual([{ provider: "ollama", address: "http://studio.local:11434" }]),
    );
    expect(b.argsOf("check_provider")).toEqual([{ provider: "ollama" }, { provider: "lm_studio" }]);
  });

  test("a slow reply for an old address does not replace the status of a newer address", async () => {
    const b = backend({ active: "custom", entries: { custom: { address: "https://one.example.com/v1" } }, check: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    await user.clear(field("Address"));
    await user.type(field("Address"), "https://two.example.com/v1{Enter}");
    await waitFor(() => expect(b.checkCalls()).toBe(2));
    await act(async () => b.pending[0].resolve(listed("old-a", "old-b")));
    expect(status().textContent).toContain("Checking…");
    expect(modelIsPicker()).toBe(false);
    await act(async () => b.pending[1].resolve({ kind: "not-listed" }));
    await waitFor(() => expect(status().textContent).toContain("The server doesn't list its models"));
    expect(modelIsPicker()).toBe(false);
  });

  test("a reply for the provider before a provider change changes nothing", async () => {
    const b = backend({ active: "openai", check: (p) => (p === "openai" ? "defer" : { kind: "unreachable" }) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    await choose(user, "gemini");
    await waitFor(() => expect(status().textContent).toBe("Can't reach Gemini"));
    await act(async () => b.pending[0].resolve(listed("gpt-a")));
    expect(status().textContent).toBe("Can't reach Gemini");
    expect(modelIsPicker()).toBe(false);
  });

  test("while a Refresh runs, the picker still shows the first list", async () => {
    const b = backend({ active: "openai", check: (_, i) => (i === 0 ? listed("a", "b") : "defer") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(b.checkCalls()).toBe(2);
    await waitFor(() => expect(status().textContent).toContain("Checking…"));
    expect(modelIsPicker()).toBe(true);
    await user.click(screen.getByRole("combobox", { name: "Model" }));
    expect(within(screen.getByRole("listbox")).getAllByRole("option").map((o) => o.textContent)).toEqual(["a", "b"]);
  });

  test("after an address commit, Model is a text field until the new result arrives", async () => {
    const b = backend({ active: "lm_studio", check: (_, i) => (i === 0 ? listed("a", "b") : "defer") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://studio.local:1234/v1{Enter}");
    await waitFor(() => expect(b.checkCalls()).toBe(2));
    expect(modelIsPicker()).toBe(false);
    await act(async () => b.pending[0].resolve(listed("local")));
    await waitFor(() => expect(modelIsPicker()).toBe(true));
  });

  test("Repair Keychain Access checks again", async () => {
    const b = backend({
      active: "openai",
      credentials: { openai: { kind: "needs_unlock", message: "The login keychain is locked." } },
      check: (_, i) => (i === 0 ? { kind: "not-requested", reason: "keychain-locked" } : listed("a")),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    const repair = await screen.findByRole("button", { name: "Repair Keychain Access" });
    b.setKey("openai", { kind: "present", host: "api.openai.com", length: 10, fingerprint: "abcd" });
    await user.click(repair);
    await waitFor(() => expect(b.checkCalls()).toBe(2));
    await waitFor(() => expect(status().textContent).toContain("Connected · 1 model"));
  });

  test("Refresh checks once more", async () => {
    const b = backend({ active: "openai", check: () => ({ kind: "unreachable" }) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Can't reach OpenAI"));
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(b.checkCalls()).toBe(2));
  });

  test("a refused address save starts no check and shows no status for the old address", async () => {
    const b = backend({
      active: "custom",
      entries: { custom: { address: "https://gw.example.com/v1" } },
      check: () => listed("old-a", "old-b"),
      over: {
        set_provider_address: () => {
          throw "The address needs a host.";
        },
      },
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://{Enter}");
    await screen.findByText("The address needs a host.");
    await waitFor(() => expect(status().textContent).not.toContain("Checking…"));
    expect(status().textContent).not.toContain("Connected");
    expect(b.checkCalls()).toBe(1);
    expect(modelIsPicker()).toBe(false);
  });

  test("a save that throws leaves no Checking… status", async () => {
    const b = backend({
      active: "ollama",
      check: () => listed(),
      over: {
        set_provider_address: () => {
          throw new Error("IPC failed");
        },
      },
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected"));
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://studio.local:11434{Enter}");
    await screen.findByText("IPC failed");
    await waitFor(() => expect(status().textContent).not.toContain("Checking…"));
    expect(b.checkCalls()).toBe(1);
  });

  test("Covers AE3. After a Custom check that adds /v1, the Address row shows that address", async () => {
    const b = backend({
      active: "custom",
      entries: { custom: { address: "https://litellm.example.com" } },
      check: () => ({ models: listed("a"), address: "https://litellm.example.com/v1" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(field("Address").value).toBe("https://litellm.example.com/v1"));
    expect(status().textContent).toBe("Connected · 1 model");
  });

  test("a Custom check that adds /v1 does not replace an address that the user is typing", async () => {
    const b = backend({ active: "custom", entries: { custom: { address: "https://litellm.example.com" } }, check: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    await user.clear(field("Address"));
    await user.type(field("Address"), "https://other.example.com");
    await act(async () => b.pending[0].resolve({ models: listed("a"), address: "https://litellm.example.com/v1" }));
    await waitFor(() => expect(b.mock.calls.some((c) => c.cmd === "get_settings")).toBe(true));
    await waitFor(() => expect(status().textContent).toBe("Connected · 1 model"));
    expect(field("Address").value).toBe("https://other.example.com");
  });

  test("a Custom check that adds /v1 keeps a typed key and reads the key status again", async () => {
    const b = backend({ active: "custom", entries: { custom: { address: "https://litellm.example.com" } }, check: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.checkCalls()).toBe(1));
    await screen.findByText("No key saved for litellm.example.com");
    await user.type(field("API key"), "sk-typed");
    const reads = b.argsOf("provider_credential_status").length;
    await act(async () => b.pending[0].resolve({ models: listed("a"), address: "https://litellm.example.com/v1" }));
    await waitFor(() => expect(field("Address").value).toBe("https://litellm.example.com/v1"));
    await waitFor(() => expect(b.argsOf("provider_credential_status").length).toBe(reads + 1));
    expect(field("API key").value).toBe("sk-typed");
    expect(b.argsOf("set_provider_credential")).toEqual([]);
  });

  test("in React.StrictMode, an address commit checks again", async () => {
    const b = backend({ active: "ollama", check: () => listed("llama3") });
    const user = userEvent.setup();
    render(
      <StrictMode>
        <Harness initial={b.initial} />
      </StrictMode>,
    );
    await waitFor(() => expect(status().textContent).toContain("Connected · 1 model"));
    const before = b.checkCalls();
    await user.clear(field("Address"));
    await user.type(field("Address"), "http://studio.local:11434{Enter}");
    await waitFor(() => expect(b.checkCalls()).toBe(before + 1));
    expect(b.argsOf("set_provider_address")).toEqual([{ provider: "ollama", address: "http://studio.local:11434" }]);
  });

  test("Covers AE1. Anthropic's model and key come back after a switch to Ollama and back", async () => {
    const b = backend({
      active: "anthropic",
      entries: { anthropic: { model: "claude-haiku-5-5" } },
      credentials: { anthropic: { kind: "present", host: "api.anthropic.com", length: 10, fingerprint: "beef" } },
      check: (p) => (p === "anthropic" ? listed("claude-haiku-5-5", "claude-opus-5-5") : { kind: "unreachable" }),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toBe("Connected · 2 models"));
    await choose(user, "ollama");
    await waitFor(() => expect(status().textContent).toContain("Can't reach Ollama"));
    expect(has("API key")).toBe(false);
    await choose(user, "anthropic");
    await waitFor(() => expect(status().textContent).toBe("Connected · 2 models"));
    expect((screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe("claude-haiku-5-5");
    expect((await screen.findByText(/Saved in Keychain for api\.anthropic\.com/)).textContent).toContain("beef");
    expect(b.mock.calls.some((c) => c.cmd === "set_provider_model")).toBe(false);
  });
});

describe("the context limit", () => {
  test("a limit saves to the active provider", async () => {
    const b = backend({ active: "ollama", check: () => listed() });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await user.type(field("Context limit"), "32000{Enter}");
    await waitFor(() =>
      expect(b.argsOf("set_provider_context_limit")).toEqual([{ provider: "ollama", limit: 32000 }]),
    );
  });

  test("a value that is not a whole number shows an error and saves nothing", async () => {
    const b = backend({ active: "ollama", check: () => listed() });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await user.type(field("Context limit"), "12.5{Enter}");
    expect(await screen.findByText("Enter a whole number of tokens, or leave empty for automatic.")).toBeTruthy();
    expect(b.mock.calls.some((c) => c.cmd === "set_provider_context_limit")).toBe(false);
  });

  test("each provider shows its own limit", async () => {
    const b = backend({
      active: "ollama",
      entries: { ollama: { context_limit_tokens: 32000 }, anthropic: { context_limit_tokens: null } },
      check: () => listed(),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    expect(field("Context limit").value).toBe("32000");
    await choose(user, "anthropic");
    await waitFor(() => expect(has("Address")).toBe(false));
    expect(field("Context limit").value).toBe("");
  });
});
