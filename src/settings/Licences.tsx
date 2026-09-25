// One group with one row for each model: the name, what it is used for and
// its licence. A click on a row shows the attribution, the licence address
// and the pinned source. The list stays short as more models come.

import { useState } from "react";
import type { ModelStatus } from "../bindings";
import { ChevronIcon } from "./icons";
import { roleLabel } from "./Models";
import { Group, Pane } from "./ui";

function Detail({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt className="text-label-secondary">{label}</dt>
      <dd className="min-w-0 break-words select-text">{children}</dd>
    </>
  );
}

function LicenceRow({ model: m }: { model: ModelStatus }) {
  const [open, setOpen] = useState(false);
  const details = `licence-${m.id}`;
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
          <span className="min-w-0 flex-1 truncate font-medium">{m.name}</span>
          <span className="shrink-0 truncate text-[0.85rem] text-label-secondary">
            {roleLabel[m.role]} · {m.licence.name}
          </span>
          <ChevronIcon
            className={`size-3 shrink-0 text-label-tertiary transition-transform duration-150 motion-reduce:transition-none ${open ? "rotate-90" : ""}`}
          />
        </button>
        {open && (
          <dl
            id={details}
            className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 pb-2.5 text-[0.85rem] leading-[1.4]"
          >
            <Detail label="Attribution">{m.licence.attribution}</Detail>
            <Detail label="Licence">{m.licence.url}</Detail>
            <Detail label="Source">
              {m.repository} @ <code>{m.revision.slice(0, 12)}</code>
            </Detail>
          </dl>
        )}
      </div>
    </div>
  );
}

export function Licences({ models }: { models: ModelStatus[] }) {
  return (
    <Pane title="Licences">
      <p className="-mt-2 px-1 text-label-secondary">
        Longhand downloads each model directly from its publisher, pinned to an exact revision,
        and checks every byte against a recorded fingerprint. Click a model to see its source and
        attribution.
      </p>
      {models.length > 0 && (
        <Group>
          {models.map((m) => (
            <LicenceRow key={m.id} model={m} />
          ))}
        </Group>
      )}
    </Pane>
  );
}
