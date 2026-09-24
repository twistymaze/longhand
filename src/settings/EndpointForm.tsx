import { useCallback, useEffect, useState } from "react";
import { commands, type CredentialStatus, type Settings } from "../bindings";
import { Button, ErrorText, Group, Pane, Row, Status, fieldClass } from "./ui";

const isMac = navigator.userAgent.includes("Mac");

function isPlaintextRemote(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    if (url.protocol !== "http:") return false;
    return !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  } catch {
    return false;
  }
}

/// A text field that commits on Return or when focus leaves, the way macOS
/// settings fields apply without a Save button.
function CommitField({
  value,
  placeholder,
  onCommit,
  inputMode,
  width = "w-64",
  label,
}: {
  value: string;
  placeholder?: string;
  onCommit: (v: string) => void;
  inputMode?: "numeric";
  width?: string;
  label: string;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  return (
    <input
      aria-label={label}
      className={`${fieldClass} ${width}`}
      value={draft}
      placeholder={placeholder}
      inputMode={inputMode}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}

function Credential({ baseUrl }: { baseUrl: string }) {
  const [status, setStatus] = useState<CredentialStatus | null>(null);
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const result = await commands.credentialStatus(baseUrl);
    if (result.status === "ok") setStatus(result.data);
    else setError(result.error);
  }, [baseUrl]);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (op: () => Promise<{ status: "ok" } | { status: "error"; error: string }>) => {
    const result = await op();
    setError(result.status === "error" ? result.error : null);
    await refresh();
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
            Saved in {isMac ? "Keychain" : "the credential store"} for {status.host} · fingerprint{" "}
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
        {status?.kind === "needs_unlock" && isMac && (
          <Button onClick={() => run(() => commands.repairKeychainAccess())}>
            Repair Keychain Access
          </Button>
        )}
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

export function EndpointForm({
  settings,
  onSettings,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [limitError, setLimitError] = useState<string | null>(null);

  const setEndpoint = async (baseUrl: string, model: string) => {
    const result = await commands.setEndpoint(baseUrl, model);
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      onSettings(result.data);
    }
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

  const baseUrl = settings.endpoint_base_url;

  return (
    <Pane title="Summarization">
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
            onCommit={(v) => setEndpoint(v, settings.endpoint_model)}
          />
        </Row>
        <Row label="Model">
          <CommitField
            label="Model"
            value={settings.endpoint_model}
            placeholder="model name"
            onCommit={(v) => setEndpoint(baseUrl, v)}
          />
        </Row>
        {error && <ErrorText>{error}</ErrorText>}
      </Group>

      {baseUrl && <Credential key={baseUrl} baseUrl={baseUrl} />}

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
