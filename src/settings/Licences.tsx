import type { ModelStatus } from "../bindings";
import { Group, Pane, Row } from "./ui";
import { roleLabel } from "./Models";

export function Licences({ models }: { models: ModelStatus[] }) {
  return (
    <Pane title="Licences">
      <p className="-mt-2 px-1 text-label-secondary">
        Longhand downloads each model directly from its publisher, pinned to the exact revision
        below, and checks every byte against a recorded fingerprint.
      </p>
      {models.map((m) => (
        <Group key={m.id} header={m.name}>
          <Row label="Used for" detail={roleLabel[m.role]} />
          <Row label="Licence" detail={m.licence.name}>
            <span className="max-w-60 truncate text-[0.85rem] text-label-secondary select-text">
              {m.licence.url}
            </span>
          </Row>
          <Row label="Attribution" detail={<span className="select-text">{m.licence.attribution}</span>} />
          <Row
            label="Source"
            detail={
              <span className="select-text">
                {m.repository} @ <code>{m.revision.slice(0, 12)}</code>
              </span>
            }
          />
        </Group>
      ))}
    </Pane>
  );
}
