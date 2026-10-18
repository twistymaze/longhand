// The AI Provider pane (R16–R19, R21). One group holds the provider, its
// address, its API key, its model and the connection status. The Advanced
// group holds the context limit. Each provider keeps its own values (R10).

import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type CredentialStatus,
  type EndpointModels,
  type Provider,
  type Result,
  type Settings,
} from "../bindings";
import { ModelPicker } from "./ModelPicker";
import { Button, CommitField, ErrorText, Group, Pane, Row, Status, type Tone, fieldClass } from "./ui";

const isMac = () => navigator.userAgent.includes("Mac");
const keyStore = () => (isMac() ? "Keychain" : "the credential store");

type Kind = "cloud" | "local" | "custom";

/// The providers in the order of the Provider menu (R7). `address` is the
/// usual address of a local provider. These values must agree with
/// `Provider::default_address` in crates/summarize/src/provider.rs.
export const providers: { id: Provider; name: string; kind: Kind; address?: string }[] = [
  { id: "openai", name: "OpenAI", kind: "cloud" },
  { id: "anthropic", name: "Anthropic", kind: "cloud" },
  { id: "gemini", name: "Gemini", kind: "cloud" },
  { id: "openrouter", name: "OpenRouter", kind: "cloud" },
  { id: "ollama", name: "Ollama", kind: "local", address: "http://localhost:11434" },
  { id: "lm_studio", name: "LM Studio", kind: "local", address: "http://localhost:1234/v1" },
  { id: "custom", name: "Custom (OpenAI-compatible)", kind: "custom" },
];

const info = (id: Provider) => providers.find((p) => p.id === id)!;

/// The name of a provider in the UI.
export const providerName = (id: Provider) => info(id).name;

/// The kind of a provider: a cloud service, a local app, or Custom.
export const providerKind = (id: Provider): Kind => info(id).kind;

/// The address that the Address row shows: the saved address, or the usual
/// address of a local provider. Custom with no saved address has none.
function shownAddress(id: Provider, settings: Settings): string {
  return settings.providers[id].address ?? info(id).address ?? "";
}

/// True for plain HTTP to another computer (R21). Plain HTTP to this
/// computer stays on this computer.
export function isPlaintextRemote(address: string): boolean {
  try {
    const url = new URL(address);
    if (url.protocol !== "http:") return false;
    return !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  } catch {
    return false;
  }
}

function hostOf(address: string): string {
  try {
    return new URL(address).host || address;
  } catch {
    return address;
  }
}

/// The API key row (R18): the key field, Save, Remove or Repair Keychain
/// Access, and the state of the saved key under the row title.
function Credential({
  provider,
  address,
  onStatus,
  onChanged,
}: {
  provider: Provider;
  /// The address of the provider. The key is stored for its host, so the
  /// row reads the status again when the address changes. The typed key stays.
  address: string;
  /// Receives each status that the row reads.
  onStatus: (status: CredentialStatus) => void;
  /// Runs after each save, removal and Repair, when the status is read again.
  onChanged: () => void;
}) {
  const [status, setStatus] = useState<CredentialStatus | null>(null);
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const onStatusRef = useRef(onStatus);
  onStatusRef.current = onStatus;

  const refresh = useCallback(async () => {
    const result = await commands.providerCredentialStatus(provider);
    if (result.status === "ok") {
      setStatus(result.data);
      onStatusRef.current(result.data);
    } else setError(result.error);
    // `address` is not read here, but a new address can have a different host.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider, address]);
  useEffect(() => {
    // An error from the key of the previous address does not apply to the new one.
    setError(null);
    void refresh();
  }, [refresh]);

  const run = async (op: () => Promise<{ status: "ok" } | { status: "error"; error: string }>) => {
    const result = await op();
    setError(result.status === "error" ? result.error : null);
    await refresh();
    onChanged();
  };

  const save = () =>
    run(async () => {
      const result = await commands.setProviderCredential(provider, secret);
      if (result.status === "ok") setSecret("");
      return result;
    });

  let summary: ReactNode = <span className="text-label-tertiary">Checking…</span>;
  if (status) {
    switch (status.kind) {
      case "present":
        summary = (
          <Status tone="positive">
            Saved in {keyStore()} for {status.host} · fingerprint{" "}
            <code className="text-[0.85rem]">{status.fingerprint}</code>
          </Status>
        );
        break;
      case "absent":
        summary = <Status tone="neutral">No key saved for {status.host}</Status>;
        break;
      case "needs_unlock":
        summary = <Status tone="caution">Locked — {status.message}</Status>;
        break;
      case "failed":
        summary = <Status tone="negative">{status.message}</Status>;
        break;
    }
  }

  return (
    <>
      <Row label="API key" detail={summary}>
        <input
          aria-label="API key"
          type="password"
          className={`${fieldClass} w-48`}
          value={secret}
          placeholder={status?.kind === "present" ? "Replace…" : "Paste key"}
          onChange={(e) => setSecret(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && secret) void save();
          }}
        />
        <Button prominent disabled={!secret} onClick={save}>
          Save
        </Button>
        {status?.kind === "present" && (
          <Button onClick={() => run(() => commands.deleteProviderCredential(provider))}>Remove</Button>
        )}
        {status?.kind === "needs_unlock" && isMac() && (
          <Button onClick={() => run(() => commands.repairKeychainAccess())}>Repair Keychain Access</Button>
        )}
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </>
  );
}

/// The tone and the text of the status row for the latest check (R17).
/// `failedMessage` is the text that the API key row shows when Keychain
/// cannot be read. `model` is the saved model.
export function statusLine(
  outcome: Result<EndpointModels, string>,
  provider: Provider,
  address: string,
  model: string,
  failedMessage: string | null,
): [Tone, ReactNode] {
  if (outcome.status === "error") return ["negative", outcome.error];
  const { name, kind } = info(provider);
  const who = kind === "custom" ? "The server" : name;
  const o = outcome.data;
  switch (o.kind) {
    case "listed": {
      if (o.ids.length === 0) return ["positive", "Connected · no models listed"];
      const count = `Connected · ${o.ids.length} ${o.ids.length === 1 ? "model" : "models"}`;
      if (model !== "" && !o.ids.includes(model)) return ["caution", `${count} · ${who} doesn't list ${model}`];
      return ["positive", count];
    }
    case "key-needed":
      return ["neutral", "Enter an API key"];
    case "key-rejected":
      return ["negative", `${who} refused this API key`];
    case "key-cannot-list":
      return ["caution", "This API key can't list models. Type a model name."];
    case "chat-only":
      return ["positive", "Connected · no model list. Type a model name."];
    case "not-listed":
      return ["caution", `${who} doesn't list its models. Type a model name, or check the address.`];
    case "redirected":
      return [
        "negative",
        o.host
          ? `The server redirected to ${o.host}. Use that address instead.`
          : "The server redirected to another address. Use that address instead.",
      ];
    case "server-error":
      return [
        "negative",
        o.status === null
          ? `${who} returned an error. Try again.`
          : `${who} returned an error (${o.status}). Try again.`,
      ];
    case "unreachable":
      if (kind === "cloud") return ["negative", `Can't reach ${name}`];
      if (kind === "local") return ["negative", `Can't reach ${name} at ${hostOf(address)}. Check that ${name} is open.`];
      return ["negative", `Can't reach ${hostOf(address)}`];
    case "not-requested":
      switch (o.reason) {
        case "no-address":
          return ["neutral", "Enter an address"];
        case "key-needed":
          return ["neutral", "Enter an API key"];
        case "keychain-locked":
          return ["caution", "Keychain is locked. Use Repair Keychain Access above."];
        case "keychain-failed":
          return ["negative", failedMessage ?? `Can't read the API key from ${keyStore()}.`];
        case "no-provider":
          return ["neutral", "No provider set"];
        case "not-chosen":
          return ["neutral", "Not checked"];
      }
  }
}

/// Save a setting. Returns true when the save succeeds. A rejected save and
/// a failed IPC call both show the error under the field.
async function save(
  saving: Promise<Result<Settings, string>>,
  setError: (e: string | null) => void,
  onSettings: (s: Settings) => void,
): Promise<boolean> {
  let result: Result<Settings, string>;
  try {
    result = await saving;
  } catch (e) {
    setError(e instanceof Error ? e.message : String(e));
    return false;
  }
  if (result.status === "error") {
    setError(result.error);
    return false;
  }
  setError(null);
  onSettings(result.data);
  return true;
}

/// The rows of one provider. The pane mounts it again for each provider, so
/// each field saves to the provider that was active when its edit started:
/// a field that is open at a provider change saves at unmount, with the
/// provider of its own render.
function ProviderRows({
  provider,
  settings,
  onSettings,
}: {
  provider: Provider;
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const { kind } = info(provider);
  const entry = settings.providers[provider];
  const address = shownAddress(provider, settings);
  const [addressError, setAddressError] = useState<string | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);

  // The check state. `latest` counts the checks; only the result of the
  // latest check applies. `models` is the list that the picker shows, or
  // null when Model is a text field.
  const latest = useRef(0);
  const alive = useRef(true);
  // True at the start: the first check starts when the rows mount.
  const [checking, setChecking] = useState(true);
  const [outcome, setOutcome] = useState<Result<EndpointModels, string> | null>(null);
  const [models, setModels] = useState<string[] | null>(null);
  const [credential, setCredential] = useState<CredentialStatus | null>(null);
  // True while the Model text field has focus and the user typed in it.
  // A list that arrives then shows the picker only after the field blurs.
  const [typing, setTyping] = useState(false);
  const onSettingsRef = useRef(onSettings);
  onSettingsRef.current = onSettings;
  const addressRef = useRef(address);
  addressRef.current = address;

  useEffect(() => {
    // React.StrictMode runs the cleanup and the effect again at mount, so the
    // effect sets `alive` to true again.
    alive.current = true;
    return () => {
      // A reply that arrives after a provider change applies to no rows.
      alive.current = false;
      latest.current += 1;
    };
  }, []);

  /// Start a check. `keep` keeps the current list while the check runs
  /// (Refresh and mount). `before` runs after the counter changes and before
  /// the check, for example to save a new address. When `before` returns
  /// false, no check starts and the status row is empty: the field error
  /// shows why. When `before` throws, the status row shows the error.
  const check = useCallback(
    async (keep: boolean, before?: () => Promise<boolean>) => {
      const id = ++latest.current;
      if (!keep) setModels(null);
      setChecking(true);
      let result: Result<EndpointModels, string>;
      let resolved: string | null = null;
      try {
        if (before && !(await before())) {
          // The save failed. Do not show the models of the previous address.
          if (id !== latest.current) return;
          setChecking(false);
          setOutcome(null);
          setModels(null);
          return;
        }
        const checked = await commands.checkProvider(provider);
        if (checked.status === "ok") {
          result = { status: "ok", data: checked.data.models };
          resolved = checked.data.address;
        } else result = checked;
      } catch (e) {
        // The save or the IPC call itself failed. Show it on the status row.
        result = { status: "error", error: e instanceof Error ? e.message : String(e) };
      }
      if (id !== latest.current) return;
      setChecking(false);
      setOutcome(result);
      setModels(
        result.status === "ok" && result.data.kind === "listed" && result.data.ids.length > 0 ? result.data.ids : null,
      );
      // The check can save a Custom address with /v1 added (R14, AE3). Read
      // the settings again so that the Address row shows it. The field keeps
      // a text that the user is still typing.
      if (provider === "custom" && resolved !== null && resolved !== addressRef.current) {
        try {
          onSettingsRef.current(await commands.getSettings());
        } catch (e) {
          console.error("The settings could not be read again:", e);
        }
      }
    },
    [provider],
  );

  useEffect(() => {
    void check(true);
  }, [check]);

  // The close waits only for the save of the address, not for the check
  // that follows it. After an unmount, no check starts.
  const saveAddress = (v: string) => {
    const saved = save(commands.setProviderAddress(provider, v.trim()), setAddressError, onSettings);
    if (alive.current) void check(false, () => saved);
    return saved;
  };
  const saveModel = (v: string) => save(commands.setProviderModel(provider, v), setModelError, onSettings);
  const showPicker = models !== null && !typing;
  // Custom needs an address before its key has a host.
  const showKey = kind === "cloud" || (kind === "custom" && address !== "");

  let line: ReactNode = null;
  if (checking) line = <span className="text-label-tertiary">Checking…</span>;
  else if (outcome) {
    const [tone, text] = statusLine(
      outcome,
      provider,
      address,
      entry.model,
      credential?.kind === "failed" ? credential.message : null,
    );
    line = <Status tone={tone}>{text}</Status>;
  }

  return (
    <>
      {kind !== "cloud" && (
        <>
          <Row
            label="Address"
            detail={
              isPlaintextRemote(address) ? (
                <Status tone="caution">
                  Plain HTTP: the transcript and API key cross the network unencrypted.
                </Status>
              ) : undefined
            }
          >
            <CommitField
              label="Address"
              value={address}
              placeholder={info(provider).address ?? "https://example.com/v1"}
              width="w-72"
              onCommit={saveAddress}
            />
          </Row>
          {addressError && <ErrorText>{addressError}</ErrorText>}
        </>
      )}
      {showKey && (
        <Credential
          provider={provider}
          address={address}
          onStatus={setCredential}
          onChanged={() => void check(false)}
        />
      )}
      <Row label="Model">
        <div
          className="contents"
          onChange={() => {
            if (!showPicker) setTyping(true);
          }}
          onBlur={() => setTyping(false)}
        >
          {showPicker ? (
            <ModelPicker value={entry.model} models={models} onCommit={saveModel} showUnlisted={false} />
          ) : (
            <CommitField label="Model" value={entry.model} placeholder="Model name" onCommit={saveModel} />
          )}
        </div>
      </Row>
      {modelError && <ErrorText>{modelError}</ErrorText>}
      {/* The dot keeps its size when a long status text wraps. */}
      <Row label={<div role="status" className="[&_[aria-hidden]]:shrink-0">{line}</div>}>
        <Button onClick={() => void check(true)}>Refresh</Button>
      </Row>
    </>
  );
}

/// The context limit of one provider (R15, R19). The pane mounts it again
/// for each provider, as for `ProviderRows`.
function ContextLimit({
  provider,
  settings,
  onSettings,
}: {
  provider: Provider;
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const setLimit = async (text: string): Promise<boolean> => {
    const trimmed = text.trim();
    const limit = trimmed === "" ? null : Number(trimmed);
    if (limit !== null && (!Number.isInteger(limit) || limit <= 0)) {
      setError("Enter a whole number of tokens, or leave empty for automatic.");
      return false;
    }
    return save(commands.setProviderContextLimit(provider, limit), setError, onSettings);
  };
  return (
    <Group
      header="Advanced"
      footer="Longhand asks the provider how much text the model accepts. If the provider does not say, Longhand uses 128,000 tokens. Enter the model's limit if the provider reports it wrongly, or if the model accepts less."
    >
      <Row label="Context limit">
        <CommitField
          label="Context limit"
          value={settings.providers[provider].context_limit_tokens?.toString() ?? ""}
          placeholder="Automatic"
          inputMode="numeric"
          width="w-28"
          onCommit={setLimit}
        />
        <span className="text-label-secondary">tokens</span>
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

/// The text under the provider group.
function footer(provider: Provider | null): string {
  if (provider === null) return "With no provider, notes are saved without a summary.";
  const { kind, name } = info(provider);
  switch (kind) {
    case "cloud":
      return `The API key is stored in ${keyStore()} and sent only to ${name}.`;
    case "local":
      return `${name} usually runs on this computer. If it runs on another computer, enter its address.`;
    case "custom":
      return `Any server that accepts OpenAI requests. The API key is optional. It is stored in ${keyStore()} and sent only to this address.`;
  }
}

export function ProviderForm({
  settings,
  onSettings,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const [chooseError, setChooseError] = useState<string | null>(null);
  const active = settings.active_provider;

  const choose = (value: string) => {
    const next = value === "" ? null : (value as Provider);
    void save(commands.chooseProvider(next), setChooseError, onSettings);
  };

  return (
    <Pane title="AI Provider">
      <Group footer={footer(active)}>
        <Row label="Provider">
          <select
            aria-label="Provider"
            className={`${fieldClass} w-72`}
            value={active ?? ""}
            onChange={(e) => choose(e.target.value)}
          >
            <option value="">None</option>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </Row>
        {chooseError && <ErrorText>{chooseError}</ErrorText>}
        {active !== null && <ProviderRows key={active} provider={active} settings={settings} onSettings={onSettings} />}
      </Group>
      {active !== null && <ContextLimit key={active} provider={active} settings={settings} onSettings={onSettings} />}
    </Pane>
  );
}
