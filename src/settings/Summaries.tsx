// The Summaries pane (R20): the provider and model that summaries use, with
// a link to the AI Provider pane, then the Background and the summary
// prompts.

import type { Settings } from "../bindings";
import { Background } from "./Background";
import { providerName } from "./ProviderForm";
import { SummaryPrompts } from "./SummaryPrompts";
import { filesReadOnlyReason, readOnlyReason } from "./shared";
import { Button, Group, Pane, Row, Status } from "./ui";

/// The line that names the provider and the model, or says that no
/// provider is set.
export function providerLine(settings: Settings): string {
  const active = settings.active_provider;
  if (active === null) return "No provider set";
  const model = settings.providers[active].model;
  return `${providerName(active)} · ${model === "" ? "No model chosen" : model}`;
}

export function SummariesPane({
  settings,
  onSettings,
  onOpenProvider,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
  /// Shows the AI Provider pane.
  onOpenProvider: () => void;
}) {
  const none = settings.active_provider === null;
  return (
    <Pane title="Summaries">
      <Group>
        <Row
          label="AI provider"
          detail={none ? <Status tone="neutral">{providerLine(settings)}</Status> : providerLine(settings)}
        >
          <Button onClick={onOpenProvider}>{none ? "Set Up…" : "Change…"}</Button>
        </Row>
      </Group>

      <Background settings={settings} onSettings={onSettings} />

      <SummaryPrompts
        onSettings={onSettings}
        readOnly={filesReadOnlyReason(settings)}
        settingsReadOnly={readOnlyReason(settings)}
      />
    </Pane>
  );
}
