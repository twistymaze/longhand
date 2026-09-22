import { useCallback, useEffect, useState } from "react";
import {
  checkMicrophonePermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { commands, events, type CredentialStatus, type ProbeStatus } from "./bindings";

// Phase 2a shell probe. Temporary: each panel proves one thing the platform
// has to allow before later units depend on it, and goes away as those units
// supersede it (the microphone with U6, the shortcut with U7, the storage
// panels with U4's real settings surface).

const isMac = navigator.userAgent.includes("Mac");

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-neutral-300 p-4 dark:border-neutral-700">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-neutral-500">
        {title}
      </h2>
      {children}
    </section>
  );
}

const button =
  "rounded-md bg-neutral-800 px-3 py-1.5 text-sm text-white hover:bg-neutral-700 disabled:opacity-40 dark:bg-neutral-200 dark:text-neutral-900 dark:hover:bg-neutral-300";
const input =
  "w-full rounded-md border border-neutral-300 bg-transparent px-2 py-1.5 text-sm dark:border-neutral-700";

function MicrophonePanel() {
  const [granted, setGranted] = useState<boolean | null>(null);
  const refresh = useCallback(async () => {
    if (isMac) setGranted(await checkMicrophonePermission());
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!isMac) {
    return (
      <Panel title="Microphone">
        <p className="text-sm">Verified on Windows from Phase 3 (U21).</p>
      </Panel>
    );
  }
  return (
    <Panel title="Microphone">
      <p className="mb-3 text-sm">
        Status:{" "}
        <strong>{granted === null ? "checking" : granted ? "authorized" : "not authorized"}</strong>
      </p>
      <button
        className={button}
        onClick={async () => {
          await requestMicrophonePermission();
          await refresh();
        }}
      >
        Request access
      </button>
    </Panel>
  );
}

function ShortcutPanel() {
  const [status, setStatus] = useState<ProbeStatus | null>(null);
  useEffect(() => {
    void commands.probeStatus().then(setStatus);
    const unlisten = events.probeShortcutFired.listen((event) => {
      setStatus((s) => (s ? { ...s, fired: event.payload.count } : s));
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);
  if (!status) return null;
  return (
    <Panel title="Global shortcut">
      <p className="text-sm">
        <code>{status.shortcut}</code>{" "}
        {status.registered ? "registered" : `failed: ${status.registration_error}`}
      </p>
      <p className="mt-2 text-sm">
        Fired <strong>{status.fired}</strong> time{status.fired === 1 ? "" : "s"} this launch.
        Press it with another app focused.
      </p>
    </Panel>
  );
}

function SettingPanel() {
  const [stored, setStored] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => setStored(await commands.getProbeSetting()), []);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <Panel title="Setting">
      <p className="mb-3 text-sm">
        Stored value: <strong>{stored === null ? "…" : stored === "" ? "(empty)" : stored}</strong>
      </p>
      <div className="flex gap-2">
        <input className={input} value={draft} onChange={(e) => setDraft(e.target.value)} />
        <button
          className={button}
          onClick={async () => {
            const result = await commands.setProbeSetting(draft);
            setError(result.status === "error" ? result.error : null);
            await load();
          }}
        >
          Save
        </button>
      </div>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
    </Panel>
  );
}

function describe(status: CredentialStatus): string {
  switch (status.kind) {
    case "present":
      return `present for ${status.host} (${status.length} chars, fingerprint ${status.fingerprint})`;
    case "absent":
      return `absent for ${status.host}`;
    case "needs_unlock":
      return `needs unlock: ${status.message}`;
    case "failed":
      return `failed: ${status.message}`;
  }
}

function CredentialPanel() {
  const [baseUrl, setBaseUrl] = useState("https://probe.example.com/v1");
  const [secret, setSecret] = useState("");
  const [status, setStatus] = useState<string>("not checked");
  const check = useCallback(async () => {
    const result = await commands.credentialStatus(baseUrl);
    setStatus(result.status === "ok" ? describe(result.data) : `error: ${result.error}`);
  }, [baseUrl]);
  const run = async (op: () => Promise<{ status: "ok" } | { status: "error"; error: string }>) => {
    const result = await op();
    if (result.status === "error") setStatus(`error: ${result.error}`);
    else await check();
  };
  return (
    <Panel title="Credential">
      <label className="mb-1 block text-xs text-neutral-500">Endpoint base URL</label>
      <input className={`${input} mb-3`} value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
      <label className="mb-1 block text-xs text-neutral-500">Credential (never shown back)</label>
      <input
        className={`${input} mb-3`}
        type="password"
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
      />
      <div className="flex flex-wrap gap-2">
        <button className={button} onClick={check}>
          Check
        </button>
        <button
          className={button}
          disabled={secret === ""}
          onClick={() =>
            run(async () => {
              const r = await commands.setCredential(baseUrl, secret);
              setSecret("");
              return r;
            })
          }
        >
          Save
        </button>
        <button className={button} onClick={() => run(() => commands.deleteCredential(baseUrl))}>
          Delete
        </button>
        {isMac && (
          <button className={button} onClick={() => run(() => commands.repairKeychainAccess())}>
            Repair Keychain Access
          </button>
        )}
      </div>
      <p className="mt-3 text-sm">{status}</p>
    </Panel>
  );
}

export default function App() {
  return (
    <main className="min-h-screen bg-white p-6 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      <h1 className="mb-1 text-xl font-semibold">Longhand</h1>
      <p className="mb-5 text-sm text-neutral-500">
        Shell probe. Closing this window keeps Longhand running in the menu bar.
      </p>
      <div className="grid gap-4">
        <MicrophonePanel />
        <ShortcutPanel />
        <SettingPanel />
        <CredentialPanel />
      </div>
    </main>
  );
}
