// The About pane (R1): a section about Longhand itself, the acknowledgements,
// one licence row for each model and each major component, and a row that
// opens the full licence list in its own window. A click on a licence row
// shows the attribution, the licence address and the source.

import { openUrl } from "@tauri-apps/plugin-opener";
import { type ReactNode, useEffect, useState } from "react";
import { commands, type BuildInfo, type ModelStatus } from "../bindings";
import { type Acknowledgement, components, type LicenceEntry, personal, technical } from "./acknowledgements";
import { ChevronIcon, ExternalIcon } from "./icons";
import { roleLabel } from "./Models";
import { Group, Pane, Row } from "./ui";

/// "Version <version> (<commit>)", or "(local build)" for a build that CI
/// did not make (R7).
export function versionText(info: BuildInfo): string {
  return `Version ${info.version} (${info.commit ?? "local build"})`;
}

/// The pages that the Longhand section opens (R5–R7).
export const longhandAddresses = {
  website: "https://longhand.md",
  repository: "https://github.com/twistymaze/longhand",
  licence: "https://github.com/twistymaze/longhand/blob/main/LICENSE",
};

/// The two opening paragraphs of README.md (R2). Change them together.
const description = [
  "Longhand is a note-taking app for meetings and conversations. You write the notes that matter to you, and Longhand records the audio. When the conversation ends, it uses the transcript to turn your short notes into a complete summary.",
  "Recording and transcription happen on your computer. Your notes are files that you own, and you decide which AI service, if any, sees them.",
];

/// A link that opens a page in the default browser (R6, KTD8 of the
/// acknowledgements plan). The webview does not go to the page. The Recent
/// changes window uses it too.
export function WebLink({ url, children }: { url: string; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={() => void openUrl(url)}
      className="inline-flex items-center gap-0.5 rounded-[3px] font-semibold text-accent select-none hover:underline active:opacity-70 outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
    >
      {children}
      <ExternalIcon className="size-3" />
      <span className="sr-only"> (opens in your browser)</span>
    </button>
  );
}

/// The first section: what Longhand is, the version and commit of the
/// running build, the copyright and licence, the website, the source page, and What's
/// new (R1–R8; R10 of the monthly changelog plan).
function LonghandSection() {
  const [info, setInfo] = useState<BuildInfo | null>(null);
  useEffect(() => {
    // A failed call leaves the version empty.
    commands.buildInfo().then(setInfo, () => {});
  }, []);
  return (
    <Group
      footer={
        <span className="select-text">
          Copyright © 2026 Erv Walter. Longhand is open source under the{" "}
          <WebLink url={longhandAddresses.licence}>MIT licence</WebLink>.
        </span>
      }
    >
      <Row label="Longhand" detail={<span className="select-text">{info ? versionText(info) : " "}</span>} />
      <div className="settings-row px-4">
        <div className="py-2.5">
          {description.map((paragraph) => (
            <p key={paragraph} className="mt-1 text-[0.85rem] leading-[1.45] text-label-secondary select-text first:mt-0">
              {paragraph}
            </p>
          ))}
        </div>
      </div>
      <div className="settings-row px-4">
        <div className="flex flex-wrap gap-x-5 gap-y-1 py-2.5">
          <WebLink url={longhandAddresses.website}>Website</WebLink>
          <WebLink url={longhandAddresses.repository}>Source code</WebLink>
          {/* What's new opens Recent changes in its own window, not a web page (R10, KTD8). */}
          <button
            type="button"
            onClick={() => void commands.openChangelog()}
            className="rounded-[3px] font-semibold text-accent select-none hover:underline active:opacity-70 outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
          >
            What's new
          </button>
        </div>
      </div>
    </Group>
  );
}

function AcknowledgementRow({ entry }: { entry: Acknowledgement }) {
  return (
    <div className="settings-row px-4">
      <div className="py-2.5">
        <WebLink url={entry.url}>{entry.name}</WebLink>
        {entry.text.map((paragraph) => (
          <p key={paragraph} className="mt-1 text-[0.85rem] leading-[1.45] text-label-secondary select-text">
            {paragraph}
          </p>
        ))}
      </div>
    </div>
  );
}

/// The web address at the start of a detail text, such as
/// "github.com/ggml-org/ggml, in transcribe-cpp-sys", and the text after it.
/// A text that does not start with an address gives null.
export function leadingAddress(text: string): { url: string; label: string; rest: string } | null {
  const match = /^(https?:\/\/)?((?:[a-z0-9-]+\.)+[a-z]{2,}\/[^\s,]*)/i.exec(text);
  if (!match) return null;
  return { url: match[1] ? match[0] : `https://${match[2]}`, label: match[0], rest: text.slice(match[0].length) };
}

/// A detail text whose address opens in the default browser. It looks like
/// the text around it until the pointer is over it.
function AddressText({ text }: { text: string }) {
  const address = leadingAddress(text);
  if (!address) return <>{text}</>;
  return (
    <>
      <button
        type="button"
        onClick={() => void openUrl(address.url)}
        className="rounded-[3px] text-left [overflow-wrap:anywhere] hover:underline active:opacity-70 outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
      >
        {address.label}
        <span className="sr-only"> (opens in your browser)</span>
      </button>
      {address.rest}
    </>
  );
}

function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-label-secondary">{label}</dt>
      <dd className="min-w-0 [overflow-wrap:anywhere] select-text">{children}</dd>
    </>
  );
}

/// One row for a model or a component (KTD9). The collapsed line shows the
/// role, when there is one, and the licence name.
function LicenceRow({ entry: e }: { entry: LicenceEntry }) {
  const [open, setOpen] = useState(false);
  const details = `licence-${e.id}`;
  return (
    <div className="settings-row px-4">
      <div>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={details}
          onClick={() => setOpen(!open)}
          className="flex w-full items-center gap-3 py-2 text-left outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
        >
          {/* Open, the row shows the full name; the role and licence move into the details. */}
          <span className={`min-w-0 flex-1 font-medium ${open ? "[overflow-wrap:anywhere]" : "truncate"}`}>
            {e.name}
          </span>
          {!open && (
            <span className="min-w-0 truncate text-[0.85rem] text-label-secondary">
              {e.kind ? `${e.kind} · ${e.licence.name}` : e.licence.name}
            </span>
          )}
          <ChevronIcon
            className={`size-3 shrink-0 text-label-tertiary transition-transform duration-150 motion-reduce:transition-none ${open ? "rotate-90" : ""}`}
          />
        </button>
        {open && (
          <dl
            id={details}
            className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 pb-2.5 text-[0.85rem] leading-[1.4]"
          >
            {e.kind && <Detail label="Used for">{e.kind}</Detail>}
            <Detail label="Licence">{e.licence.name}</Detail>
            <Detail label="Attribution">{e.licence.attribution}</Detail>
            <Detail label="Licence text">
              <AddressText text={e.licence.url} />
            </Detail>
            <Detail label="Source">
              <AddressText text={e.source} />
              {e.revision && (
                <>
                  {" @ "}
                  <code>{e.revision}</code>
                </>
              )}
            </Detail>
          </dl>
        )}
      </div>
    </div>
  );
}

const modelEntry = (m: ModelStatus): LicenceEntry => ({
  id: `model-${m.id}`,
  name: m.name,
  kind: roleLabel[m.role],
  licence: m.licence,
  source: m.repository,
  revision: m.revision.slice(0, 12),
});

export function About({ models }: { models: ModelStatus[] }) {
  return (
    <Pane title="About">
      <LonghandSection />
      <Group header="Acknowledgements">
        {personal.map((a) => (
          <AcknowledgementRow key={a.name} entry={a} />
        ))}
      </Group>
      <Group header="Built with">
        {technical.map((a) => (
          <AcknowledgementRow key={a.name} entry={a} />
        ))}
      </Group>
      <Group
        header="Licences"
        footer="Longhand downloads each model directly from its publisher, pinned to an exact revision, and checks every byte against a recorded fingerprint. Click a row to see its source and attribution."
      >
        {models.map((m) => (
          <LicenceRow key={m.id} entry={modelEntry(m)} />
        ))}
        {components.map((c) => (
          <LicenceRow key={c.id} entry={c} />
        ))}
      </Group>
      <Group>
        <div className="settings-row">
          <button
            type="button"
            onClick={() => void commands.openLicences()}
            className="flex w-full items-center gap-3 px-4 py-2.5 text-left active:bg-fill-pressed outline-offset-[-2px] focus-visible:outline-2 focus-visible:outline-accent"
          >
            <span className="min-w-0 flex-1">
              <span className="block font-medium">All open source licences</span>
              <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-label-secondary">
                The full text of every licence, in a new window.
              </span>
            </span>
            <ChevronIcon className="size-3 shrink-0 text-label-tertiary" />
          </button>
        </div>
      </Group>
    </Pane>
  );
}
