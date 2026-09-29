import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { useState } from "react";
import type { CredentialStatus, EndpointModels, Settings } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { EndpointForm } from "./EndpointForm";

// The form shows Repair Keychain Access only on a Mac. Set a Mac user agent
// for these tests, then restore the user agent for the other test files.
beforeAll(() => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
});
afterAll(() => {
  delete (navigator as { userAgent?: string }).userAgent;
});

const baseSettings: Settings = {
  settings_schema_version: 4,
    interface_zoom_percent: 100,
  shortcut: "Alt+Shift+L",
  show_window_shortcut: null,
  destination_roots: [],
  current_destination: null,
  endpoint_base_url: null,
  endpoint_model: "",
  context_limit_tokens: null,
  default_speaker_count: null,
  asr_model: "a",
  pending_asr_model: null,
  live_transcript: true,
  speaker_source: "pyannote",
  speaker_processing_mode: "after_stop",
  recognition_language: null,
  pending_recognition: null,
  microphone: null,
  system_audio_default: false,
  hide_from_dock: false,
  check_updates_automatically: true,
  learned_context_limits: {},
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
};

const listed = (...ids: string[]): EndpointModels => ({ kind: "listed", ids });

type Deferred = { resolve: (value: EndpointModels) => void };

/// A fake backend. `list` gives the result of each list request by its
/// index. A result of "defer" keeps the request open until the test
/// resolves it through `pending`.
function backend({
  settings = {},
  list = () => ({ kind: "not-requested", reason: "key-needed" }),
  credential = { kind: "absent", host: "api.openai.com" },
  over = {},
}: {
  settings?: Partial<Settings>;
  list?: (index: number) => EndpointModels | "defer";
  credential?: CredentialStatus;
  over?: Record<string, CommandHandler>;
} = {}) {
  let current: Settings = { ...baseSettings, ...settings };
  let credentialNow = credential;
  let listIndex = 0;
  const pending: Deferred[] = [];
  const mock = mockTauri({
    summary_prompts: () => ({
      prompts: [{ id: "topic-summary", name: "Topic summary", text: "Topic text", built_in: true }],
      active: "topic-summary",
      text_limit: 4000,
    }),
    credential_status: () => credentialNow,
    set_credential: () => {
      credentialNow = { kind: "present", host: "api.openai.com", length: 10, fingerprint: "abcd" };
      return null;
    },
    delete_credential: () => {
      credentialNow = { kind: "absent", host: "api.openai.com" };
      return null;
    },
    repair_keychain_access: () => null,
    list_endpoint_models: () => {
      const result = list(listIndex++);
      if (result !== "defer") return result;
      return new Promise<EndpointModels>((resolve) => pending.push({ resolve }));
    },
    set_endpoint_base_url: ({ baseUrl }) => {
      current = { ...current, endpoint_base_url: baseUrl as string };
      return current;
    },
    set_endpoint_model: ({ model }) => {
      current = { ...current, endpoint_model: model as string };
      return current;
    },
    ...over,
  });
  mockWindows("settings");
  const listCalls = () => mock.calls.filter((c) => c.cmd === "list_endpoint_models").length;
  const setCredential = (next: CredentialStatus) => {
    credentialNow = next;
  };
  return { mock, pending, listCalls, initial: current, setCredential };
}

function Harness({ initial }: { initial: Settings }) {
  const [settings, setSettings] = useState(initial);
  return <EndpointForm settings={settings} onSettings={setSettings} />;
}

const status = () => screen.getByRole("status");
const modelIsPicker = () => screen.queryByRole("combobox", { name: "Model" }) !== null;

describe("the Summaries pane", () => {
  test("a new install shows the OpenAI URL, asks for a key and sends one gated list request", async () => {
    const b = backend();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key to see the models"));
    expect((screen.getByLabelText("Base URL") as HTMLInputElement).value).toBe("https://api.openai.com/v1");
    expect(modelIsPicker()).toBe(false);
    expect(screen.getByLabelText("Model").tagName).toBe("INPUT");
    expect(b.listCalls()).toBe(1);
  });

  test("the order is Base URL, API key, Model, then the status line", async () => {
    const b = backend();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key"));
    const nodes = [
      screen.getByLabelText("Base URL"),
      screen.getByLabelText("API key"),
      screen.getByLabelText("Model"),
      status(),
    ];
    for (let i = 1; i < nodes.length; i++) {
      expect(nodes[i - 1].compareDocumentPosition(nodes[i]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  });

  test("a key save requests the list again and shows the picker", async () => {
    const b = backend({
      list: (i) => (i === 0 ? { kind: "not-requested", reason: "key-needed" } : i === 1 ? "defer" : listed()),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key"));
    await user.type(screen.getByLabelText("API key"), "sk-test");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(b.listCalls()).toBe(2));
    expect(status().textContent).toContain("Checking…");
    await act(async () => b.pending[0].resolve(listed("a", "b", "c")));
    await waitFor(() => expect(status().textContent).toContain("Connected · 3 models"));
    expect(modelIsPicker()).toBe(true);
  });

  test("a Base URL commit requests the list and shows the picker", async () => {
    const b = backend({ list: (i) => (i === 0 ? { kind: "not-requested", reason: "key-needed" } : listed("llama3", "qwen")) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key"));
    const field = screen.getByLabelText("Base URL");
    await user.clear(field);
    await user.type(field, "http://localhost:11434/v1{Enter}");
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    expect(b.listCalls()).toBe(2);
    expect(modelIsPicker()).toBe(true);
    expect(b.mock.calls.find((c) => c.cmd === "set_endpoint_base_url")?.args).toEqual({
      baseUrl: "http://localhost:11434/v1",
    });
  });

  test("a server that does not list models gives a text field that saves on blur", async () => {
    const b = backend({ settings: { endpoint_base_url: "https://gw.example.com" }, list: () => ({ kind: "not-listed" }) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toContain(
        "This server doesn't list its models. Type a model name, or check that the URL ends in /v1.",
      ),
    );
    expect(modelIsPicker()).toBe(false);
    await user.type(screen.getByLabelText("Model"), "my-route");
    await user.tab();
    await waitFor(() =>
      expect(b.mock.calls.find((c) => c.cmd === "set_endpoint_model")?.args).toEqual({ model: "my-route" }),
    );
  });

  test("a rejected key shows the text, and a typed model still saves through the Model setter only", async () => {
    const b = backend({ list: () => ({ kind: "key-rejected" }) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("The server rejected this API key"));
    await user.type(screen.getByLabelText("Model"), "gpt-4o{Enter}");
    await waitFor(() => expect(b.mock.calls.some((c) => c.cmd === "set_endpoint_model")).toBe(true));
    expect(b.mock.calls.find((c) => c.cmd === "set_endpoint_model")?.args).toEqual({ model: "gpt-4o" });
    expect(b.mock.calls.some((c) => c.cmd === "set_endpoint_base_url")).toBe(false);
  });

  test("a result that returns after a Base URL commit changes nothing; only the second result applies", async () => {
    const b = backend({ settings: { endpoint_base_url: "https://one.example.com/v1" }, list: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.listCalls()).toBe(1));
    const field = screen.getByLabelText("Base URL");
    await user.clear(field);
    await user.type(field, "https://two.example.com/v1{Enter}");
    await waitFor(() => expect(b.listCalls()).toBe(2));
    await act(async () => b.pending[0].resolve(listed("old-a", "old-b")));
    expect(status().textContent).toContain("Checking…");
    expect(modelIsPicker()).toBe(false);
    await act(async () => b.pending[1].resolve({ kind: "not-listed" }));
    await waitFor(() => expect(status().textContent).toContain("This server doesn't list its models"));
    expect(modelIsPicker()).toBe(false);
  });

  test("while a Refresh request runs, the picker still shows the first list", async () => {
    const b = backend({ list: (i) => (i === 0 ? listed("a", "b") : "defer") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(b.listCalls()).toBe(2);
    await waitFor(() => expect(status().textContent).toContain("Checking…"));
    expect(modelIsPicker()).toBe(true);
    await user.click(screen.getByRole("combobox", { name: "Model" }));
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["a", "b"]);
  });

  test("after a Base URL commit, Model is a text field until the new result arrives", async () => {
    const b = backend({ list: (i) => (i === 0 ? listed("a", "b") : "defer") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    const field = screen.getByLabelText("Base URL");
    await user.clear(field);
    await user.type(field, "http://localhost:1234/v1{Enter}");
    await waitFor(() => expect(b.listCalls()).toBe(2));
    expect(modelIsPicker()).toBe(false);
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    await act(async () => b.pending[0].resolve(listed("local")));
    await waitFor(() => expect(modelIsPicker()).toBe(true));
  });

  test("Repair Keychain Access requests the list again", async () => {
    const b = backend({
      credential: { kind: "needs_unlock", message: "The login keychain is locked." },
      list: (i) => (i === 0 ? { kind: "not-requested", reason: "keychain-locked" } : listed("a")),
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    const repair = await screen.findByRole("button", { name: "Repair Keychain Access" });
    b.setCredential({ kind: "present", host: "api.openai.com", length: 10, fingerprint: "abcd" });
    await user.click(repair);
    await waitFor(() => expect(b.listCalls()).toBe(2));
    await waitFor(() => expect(status().textContent).toContain("Connected · 1 model"));
  });

  test("a locked Keychain shows the locked text, a text field and the Repair control", async () => {
    const b = backend({
      credential: { kind: "needs_unlock", message: "The login keychain is locked." },
      list: () => ({ kind: "not-requested", reason: "keychain-locked" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() =>
      expect(status().textContent).toContain("Keychain is locked. Use Repair Keychain Access above."),
    );
    expect(modelIsPicker()).toBe(false);
    expect(await screen.findByRole("button", { name: "Repair Keychain Access" })).toBeTruthy();
    expect(status().querySelector("button")).toBeNull();
  });

  test("a failed Keychain shows the Credential failed message", async () => {
    const b = backend({
      credential: { kind: "failed", message: "Keychain refused access to the key." },
      list: () => ({ kind: "not-requested", reason: "keychain-failed" }),
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Keychain refused access to the key."));
  });

  test("Refresh calls the list command once more", async () => {
    const b = backend({ list: () => ({ kind: "unreachable" }) });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Can't reach api.openai.com"));
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(b.listCalls()).toBe(2));
  });

  test("a list that arrives while the user types does not replace the text field", async () => {
    const b = backend({ list: () => "defer" });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(b.listCalls()).toBe(1));
    await user.type(screen.getByLabelText("Model"), "gpt-4");
    await act(async () => b.pending[0].resolve(listed("a", "b")));
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    expect(modelIsPicker()).toBe(false);
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe("gpt-4");
    await user.tab();
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    expect(b.mock.calls.find((c) => c.cmd === "set_endpoint_model")?.args).toEqual({ model: "gpt-4" });
    await waitFor(() =>
      expect((screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement).value).toBe("gpt-4"),
    );
  });

  test("an empty Base URL hides the API key field and asks for a Base URL", async () => {
    const b = backend({ settings: { endpoint_base_url: "" }, list: () => ({ kind: "not-requested", reason: "no-base-url" }) });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Enter a Base URL"));
    expect(screen.queryByLabelText("API key")).toBeNull();
    expect((screen.getByLabelText("Base URL") as HTMLInputElement).value).toBe("");
  });

  test("a choice in the picker saves through the Model setter only", async () => {
    const b = backend({ list: () => listed("gpt-4o", "gpt-4o-mini") });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(modelIsPicker()).toBe(true));
    await user.click(screen.getByRole("combobox", { name: "Model" }));
    await user.click(screen.getByRole("option", { name: "gpt-4o-mini" }));
    await waitFor(() =>
      expect(b.mock.calls.find((c) => c.cmd === "set_endpoint_model")?.args).toEqual({ model: "gpt-4o-mini" }),
    );
    expect(b.mock.calls.some((c) => c.cmd === "set_endpoint_base_url")).toBe(false);
  });

  test("committing the unchanged default URL saves nothing", async () => {
    const b = backend();
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key"));
    await user.click(screen.getByLabelText("Base URL"));
    await user.keyboard("{Enter}");
    await user.tab();
    expect(b.mock.calls.some((c) => c.cmd === "set_endpoint_base_url")).toBe(false);
    expect(b.listCalls()).toBe(1);
  });

  test("a rejected Base URL save sends no list request and shows no status for the old URL", async () => {
    const b = backend({
      list: () => listed("old-a", "old-b"),
      over: {
        set_endpoint_base_url: () => {
          throw "The Base URL needs a host.";
        },
      },
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Connected · 2 models"));
    const field = screen.getByLabelText("Base URL");
    await user.clear(field);
    await user.type(field, "http://{Enter}");
    await screen.findByText("The Base URL needs a host.");
    await waitFor(() => expect(status().textContent).not.toContain("Checking…"));
    expect(status().textContent).not.toContain("Connected");
    expect(b.listCalls()).toBe(1);
    expect(modelIsPicker()).toBe(false);
  });

  test("a save that throws leaves no Checking… status", async () => {
    const b = backend({
      over: {
        set_endpoint_base_url: () => {
          throw new Error("IPC failed");
        },
      },
    });
    const user = userEvent.setup();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("Add an API key"));
    const field = screen.getByLabelText("Base URL");
    await user.clear(field);
    await user.type(field, "http://localhost:11434/v1{Enter}");
    await screen.findByText("IPC failed");
    await waitFor(() => expect(status().textContent).not.toContain("Checking…"));
    expect(b.listCalls()).toBe(1);
  });

  test("a list request that throws shows the error on the status line in the negative tone", async () => {
    const b = backend({
      over: {
        list_endpoint_models: () => {
          throw new Error("The list command failed.");
        },
      },
    });
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().textContent).toContain("The list command failed."));
    expect(status().querySelector(".text-negative")).not.toBeNull();
  });

  test("the status line is a live region", async () => {
    const b = backend();
    render(<Harness initial={b.initial} />);
    await waitFor(() => expect(status().getAttribute("role")).toBe("status"));
  });
});
