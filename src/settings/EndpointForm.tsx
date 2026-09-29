import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type CredentialStatus,
  type EndpointModels,
  type Result,
  type Settings,
} from "../bindings";
import { ModelPicker } from "./ModelPicker";
import { SummaryPrompts } from "./SummaryPrompts";
import { readOnlyReason } from "./shared";
import { Button, CommitField, ErrorText, Group, Pane, Row, Status, type Tone, fieldClass } from "./ui";

// A function, not a constant, so that a test can set the platform.
const isMac = () => navigator.userAgent.includes("Mac");

// The Base URL that applies when the user never saved one (R1). This value
// must match `DEFAULT_ENDPOINT_BASE_URL` in src-tauri/src/managers/settings.rs.
const DEFAULT_BASE_URL = "https://api.openai.com/v1";

function isPlaintextRemote(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    if (url.protocol !== "http:") return false;
    return !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  } catch {
    return false;
  }
}

function hostOf(baseUrl: string): string {
  try {
    return new URL(baseUrl).host || baseUrl;
  } catch {
    return baseUrl;
  }
}

function Credential({
  baseUrl,
  onStatus,
  onChanged,
}: {
  baseUrl: string;
  /// Receives each status that the Credential group reads.
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
    const result = await commands.credentialStatus(baseUrl);
    if (result.status === "ok") {
      setStatus(result.data);
      onStatusRef.current(result.data);
    } else setError(result.error);
  }, [baseUrl]);
  useEffect(() => {
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
      const result = await commands.setCredential(baseUrl, secret);
      if (result.status === "ok") setSecret("");
      return result;
    });

  let summary = null;
  if (status) {
    switch (status.kind) {
      case "present":
        summary = (
          <Status tone="positive">
            Saved in {isMac() ? "Keychain" : "the credential store"} for {status.host} · fingerprint{" "}
            <code className="text-[0.85rem]">{status.fingerprint}</code>
          </Status>
        );
        break;
      case "absent":
        summary = <Status tone="neutral">No credential for {status.host}</Status>;
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
    <Group
      header="Credential"
      footer="Stored by the operating system and sent only to the host above. A different host needs its own credential."
    >
      <Row label="API key">
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
      </Row>
      <Row label={summary ?? <span className="text-label-tertiary">Checking…</span>}>
        {status?.kind === "present" && (
          <Button onClick={() => run(() => commands.deleteCredential(baseUrl))}>Remove</Button>
        )}
        {status?.kind === "needs_unlock" && isMac() && (
          <Button onClick={() => run(() => commands.repairKeychainAccess())}>
            Repair Keychain Access
          </Button>
        )}
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

/// The tone and the text of the status line for the latest list result
/// (KTD6). `failedMessage` is the text that the Credential group shows when
/// Keychain cannot be read.
function statusLine(
  outcome: Result<EndpointModels, string>,
  baseUrl: string,
  failedMessage: string | null,
): [Tone, ReactNode] {
  if (outcome.status === "error") return ["negative", outcome.error];
  const o = outcome.data;
  switch (o.kind) {
    case "listed":
      return o.ids.length > 0
        ? ["positive", `Connected · ${o.ids.length} ${o.ids.length === 1 ? "model" : "models"}`]
        : ["positive", "Connected · this server lists no models"];
    case "key-needed":
      return ["neutral", "Add an API key to see the models"];
    case "key-rejected":
      return ["negative", "The server rejected this API key"];
    case "key-cannot-list":
      return ["caution", "This API key can't list models. Type a model name."];
    case "not-listed":
      return [
        "caution",
        "This server doesn't list its models. Type a model name, or check that the URL ends in /v1.",
      ];
    case "redirected":
      return [
        "negative",
        o.host
          ? `The server redirected to ${o.host}. Use that URL instead.`
          : "The server redirected to another URL. Use that URL instead.",
      ];
    case "server-error":
      return [
        "negative",
        o.status === null
          ? "The server returned an error. Try again."
          : `The server returned an error (${o.status}). Try again.`,
      ];
    case "unreachable":
      return ["negative", `Can't reach ${hostOf(baseUrl)}`];
    case "not-requested":
      switch (o.reason) {
        case "no-base-url":
          return ["neutral", "Enter a Base URL"];
        case "key-needed":
          return ["neutral", "Add an API key to see the models"];
        case "keychain-locked":
          return ["caution", "Keychain is locked. Use Repair Keychain Access above."];
        case "keychain-failed":
          return [
            "negative",
            failedMessage ?? `Can't read the API key from ${isMac() ? "Keychain" : "the credential store"}.`,
          ];
      }
  }
}

export function EndpointForm({
  settings,
  onSettings,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const [urlError, setUrlError] = useState<string | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);
  const [limitError, setLimitError] = useState<string | null>(null);

  // The list request state (KTD7). `latest` counts the requests; only the
  // result of the latest request applies. `models` is the list that the
  // picker shows, or null when Model is a text field.
  const latest = useRef(0);
  // True at the start: the first request starts when the form mounts.
  const [checking, setChecking] = useState(true);
  const [outcome, setOutcome] = useState<Result<EndpointModels, string> | null>(null);
  const [models, setModels] = useState<string[] | null>(null);
  const [credential, setCredential] = useState<CredentialStatus | null>(null);
  // True while the Model text field has focus and the user typed in it.
  // A list that arrives then shows the picker only after the field blurs.
  const [typing, setTyping] = useState(false);

  /// Start a list request. `keep` keeps the current list while the request
  /// runs (Refresh and pane open). `before` runs after the counter changes
  /// and before the request, for example to save a new Base URL. When
  /// `before` returns false, no request starts and the status line is empty:
  /// the field error shows why. When `before` throws, the status line shows
  /// the error.
  const check = useCallback(async (keep: boolean, before?: () => Promise<boolean>) => {
    const id = ++latest.current;
    if (!keep) setModels(null);
    setChecking(true);
    let result: Result<EndpointModels, string>;
    try {
      if (before && !(await before())) {
        // The save failed. Do not list the models of the previous Base URL.
        if (id !== latest.current) return;
        setChecking(false);
        setOutcome(null);
        setModels(null);
        return;
      }
      result = await commands.listEndpointModels();
    } catch (e) {
      // The save or the IPC call itself failed. Show the failure on the status line.
      result = { status: "error", error: e instanceof Error ? e.message : String(e) };
    }
    if (id !== latest.current) return;
    setChecking(false);
    setOutcome(result);
    setModels(result.status === "ok" && result.data.kind === "listed" && result.data.ids.length > 0 ? result.data.ids : null);
  }, []);

  useEffect(() => {
    void check(true);
  }, [check]);

  /// Save a setting. Returns true when the save succeeds. A rejected save
  /// and a failed IPC call both show the error under the field.
  const save = async (
    saving: Promise<Result<Settings, string>>,
    setError: (e: string | null) => void,
  ): Promise<boolean> => {
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
  };

  const setLimit = async (text: string) => {
    const trimmed = text.trim();
    const limit = trimmed === "" ? null : Number(trimmed);
    if (limit !== null && (!Number.isInteger(limit) || limit <= 0)) {
      setLimitError("Enter a whole number of tokens, or leave empty for automatic.");
      return;
    }
    const result = await commands.setContextLimit(limit);
    if (result.status === "error") setLimitError(result.error);
    else {
      setLimitError(null);
      onSettings(result.data);
    }
  };

  const baseUrl = settings.endpoint_base_url ?? DEFAULT_BASE_URL;
  const saveModel = (v: string) => save(commands.setEndpointModel(v), setModelError);
  const showPicker = models !== null && !typing;

  let line: ReactNode = null;
  if (checking) line = <span className="text-label-tertiary">Checking…</span>;
  else if (outcome) {
    const [tone, text] = statusLine(outcome, baseUrl, credential?.kind === "failed" ? credential.message : null);
    line = <Status tone={tone}>{text}</Status>;
  }

  return (
    <Pane title="Summaries">
      <Group
        header="Endpoint"
        footer={
          isPlaintextRemote(baseUrl) ? (
            <span className="text-caution">
              This address uses plain HTTP, so the transcript and credential cross the network
              unencrypted.
            </span>
          ) : (
            "Any OpenAI-compatible server: a hosted provider, a gateway, or one running on this machine."
          )
        }
      >
        <Row label="Base URL">
          <CommitField
            label="Base URL"
            value={baseUrl}
            placeholder="https://api.example.com/v1"
            onCommit={(v) => void check(false, () => save(commands.setEndpointBaseUrl(v), setUrlError))}
          />
        </Row>
        {urlError && <ErrorText>{urlError}</ErrorText>}
      </Group>

      {baseUrl && (
        <Credential
          key={baseUrl}
          baseUrl={baseUrl}
          onStatus={setCredential}
          onChanged={() => void check(false)}
        />
      )}

      <Group>
        <Row label="Model">
          <div
            className="contents"
            onChange={() => {
              if (!showPicker) setTyping(true);
            }}
            onBlur={() => setTyping(false)}
          >
            {showPicker ? (
              <ModelPicker value={settings.endpoint_model} models={models} onCommit={saveModel} />
            ) : (
              <CommitField
                label="Model"
                value={settings.endpoint_model}
                placeholder="model name"
                onCommit={saveModel}
              />
            )}
          </div>
        </Row>
        {/* The dot keeps its size when a long status text wraps. */}
        <Row label={<div role="status" className="[&_[aria-hidden]]:shrink-0">{line}</div>}>
          <Button onClick={() => void check(true)}>Refresh</Button>
        </Row>
        {modelError && <ErrorText>{modelError}</ErrorText>}
      </Group>

      <SummaryPrompts onSettings={onSettings} readOnly={readOnlyReason(settings)} />

      <Group
        header="Context"
        footer="Longhand asks the server how much text the model accepts. Set a limit only if the server reports it wrongly."
      >
        <Row label="Context limit">
          <CommitField
            label="Context limit"
            value={settings.context_limit_tokens?.toString() ?? ""}
            placeholder="Automatic"
            inputMode="numeric"
            width="w-28"
            onCommit={setLimit}
          />
          <span className="text-label-secondary">tokens</span>
        </Row>
        {limitError && <ErrorText>{limitError}</ErrorText>}
      </Group>
    </Pane>
  );
}
