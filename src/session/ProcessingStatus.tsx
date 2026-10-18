// A stopped session on its way to the destination (F2, R14, R30), on the
// Summary and Transcript tabs: each stage with its status, what the summary
// stage sends and to whom, and the one action each exception needs. The
// page reads the detail again whenever the session moves (AE4). The only
// motion is a progress bar that fills, and a quiet sweep on the speaker bar
// while its work cannot be measured. Regenerate Summary on a saved note
// shows the summarize stage in the same form.

import { useEffect, useState } from "react";
import { commands, type EndpointView, type SessionDetail, type SpeakerProgressView, type WaitView } from "../bindings";
import {
  type SpeakerMeter,
  type StageRow,
  sentence,
  speakerMeter,
  stageLabel,
  stageRows,
  stoppedAt,
  summaryStepLabel,
  waitText,
} from "../processing/describe";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { useFolderChange } from "../settings/OldNotesDialog";
import { providerKind, providerName } from "../settings/ProviderForm";
import { Button, ErrorText, Group, ProgressBar, Row, Status } from "../settings/ui";
import { REMOVED_REASON } from "./RemoveAudio";
import type { Live } from "./useSessions";

function StageIcon({ status }: { status: StageRow["status"] }) {
  if (status === "done") {
    return (
      <svg aria-label="done" viewBox="0 0 16 16" className="size-4 shrink-0 text-positive">
        <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  if (status === "failed") {
    return (
      <svg aria-label="stopped" viewBox="0 0 16 16" className="size-4 shrink-0 text-negative">
        <path d="M4.5 4.5l7 7m0-7l-7 7" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      </svg>
    );
  }
  if (status === "running") {
    return <span aria-label="in progress" className="mx-1 size-2 shrink-0 rounded-full bg-accent" />;
  }
  return <span aria-label="to do" className="mx-1 size-2 shrink-0 rounded-full border border-label-tertiary" />;
}

function progressOf(row: StageRow, detail: SessionDetail, live: Live | undefined): number | null {
  if (row.status !== "running" || row.stage === "diarize") return null;
  if (row.stage === "transcribe" && detail.windows_total) {
    return (detail.windows_done ?? 0) / detail.windows_total;
  }
  // A summary step of one call shows no bar: it would stay at 0% for the
  // whole call.
  const least = row.stage === "summarize" ? 1 : 0;
  if (live && live.stage === row.stage && live.total > least) return live.done / live.total;
  return null;
}

/// The label of `row`. A running summary in parts names its step.
function labelOf(row: StageRow, live: Live | undefined): StageRow {
  if (row.stage !== "summarize" || row.status !== "running" || live?.stage !== "summarize") return row;
  return { ...row, label: summaryStepLabel(live) };
}

/// A measured percent that has not changed for this long shows activity
/// too, so a long step does not look stopped (R4).
const UNCHANGED_MS = 3000;

/// True when `percent` has not changed for `UNCHANGED_MS`.
function useUnchanged(percent: number | null): boolean {
  const [unchanged, setUnchanged] = useState(false);
  useEffect(() => {
    setUnchanged(false);
    const timer = setTimeout(() => setUnchanged(true), UNCHANGED_MS);
    return () => clearTimeout(timer);
  }, [percent]);
  return unchanged;
}

/// The speaker stage's percent and bar. The percent shows measured work
/// only; activity shows work that it cannot measure (KTD3, KTD5).
function SpeakerProgress({ meter }: { meter: SpeakerMeter }) {
  const unchanged = useUnchanged(meter.percent);
  const active = !meter.measuring || meter.percent === null || unchanged;
  const label = stageLabel("diarize");
  const valueText =
    meter.percent === null ? "In progress" : active ? `${meter.percent}%, still working` : `${meter.percent}%`;
  return (
    <>
      {active && <span className="hidden text-label-secondary motion-reduce:inline">Still working</span>}
      {meter.percent !== null && <span className="text-label-secondary tabular-nums">{meter.percent}%</span>}
      <ProgressBar
        fraction={meter.percent === null ? null : meter.percent / 100}
        label={label}
        valueText={valueText}
        active={active}
      />
    </>
  );
}

/// Where the summary goes (R22): a cloud provider by its name, a local
/// provider by its name and host, and Custom by its host.
function destination(endpoint: EndpointView): string {
  switch (providerKind(endpoint.provider)) {
    case "cloud":
      return providerName(endpoint.provider);
    case "local":
      return `${providerName(endpoint.provider)} at ${endpoint.host}`;
    case "custom":
      return endpoint.host;
  }
}

function SummarizeDetail({ detail, sent }: { detail: SessionDetail; sent: boolean }) {
  if (detail.summary_problem) {
    return <Status tone="caution">Skipped. {sentence(detail.summary_problem)}</Status>;
  }
  const endpoint = detail.endpoint;
  if (!endpoint) return <>No AI provider is set, so the note is saved without a summary.</>;
  return (
    <>
      {sent ? "Sent" : "Sends"} the transcript and notes to {destination(endpoint)} ({endpoint.model})
      {!endpoint.encrypted && <span className="text-caution">, unencrypted</span>}.
    </>
  );
}

/// Opens Settings on the AI Provider pane (R11, KTD10).
export function SetUpProviderButton() {
  return <Button onClick={() => void commands.openSettings("provider")}>Set Up AI Provider…</Button>;
}

type Outcome = { status: "ok" } | { status: "error"; error: string };

function Exception({ detail, onDone }: { detail: SessionDetail; onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [startOver, setStartOver] = useState(false);
  const folder = useFolderChange();
  const run = (action: () => Promise<Outcome>) =>
    void action().then((r) => {
      setError(r.status === "error" ? r.error : null);
      onDone();
    });
  const id = detail.entry.id;

  switch (detail.entry.state) {
    case "processing_failed": {
      const kept = [detail.audio_kept && "audio", detail.transcript_kept && "transcript"].filter(Boolean);
      // Removed audio does not come back, so Retry cannot run (R14).
      const removed = detail.audio === "removed";
      return (
        <Group
          footer={
            removed
              ? REMOVED_REASON
              : detail.retry_starts_over
                ? "Retry starts over from the recorded audio."
                : kept.length > 0 &&
                  `The ${kept.join(" and ")} ${kept.length > 1 ? "are" : "is"} kept. Retry continues from where processing stopped.`
          }
        >
          <Row
            label={<Status tone="negative">{stoppedAt(detail.failed_stage)}</Status>}
            detail={detail.message && sentence(detail.message)}
          >
            <Button
              prominent
              disabled={removed}
              onClick={() =>
                // The same step failed again after a Retry: the next run
                // starts over, so the user confirms first.
                detail.retry_starts_over ? setStartOver(true) : run(() => commands.retrySession(id, false))
              }
            >
              Retry
            </Button>
          </Row>
          {error && <ErrorText>{error}</ErrorText>}
          {startOver && (
            <ConfirmDialog
              title="Start Over?"
              message="Retry stopped at the same step again. Longhand can transcribe the note again from the recorded audio. This takes longer. Your notes and the audio are kept."
              action="Start Over"
              destructive={false}
              onConfirm={() => {
                setStartOver(false);
                run(() => commands.retrySession(id, true));
              }}
              onCancel={() => setStartOver(false)}
            />
          )}
        </Group>
      );
    }
    case "blocked":
      return (
        <Group footer="The summary's credential is in your keychain. Continuing asks macOS to unlock it.">
          <Row
            label={<Status tone="caution">Waiting for the keychain</Status>}
            detail={detail.message && sentence(detail.message)}
          >
            <Button prominent onClick={() => run(() => commands.unblockSession(id))}>
              Continue
            </Button>
          </Row>
          {error && <ErrorText>{error}</ErrorText>}
        </Group>
      );
    case "awaiting_destination":
      return (
        <Group footer="Everything is kept safely on this Mac until it can be saved.">
          <Row
            label={<Status tone="caution">Waiting for a folder</Status>}
            detail={detail.message && sentence(detail.message)}
          >
            <Button onClick={() => run(() => commands.retrySession(id, false))}>Try Again</Button>
            <Button
              prominent
              onClick={() =>
                // A change of folder retries every note that waits for one.
                run(folder.change)
              }
            >
              Choose Folder…
            </Button>
          </Row>
          {error && <ErrorText>{error}</ErrorText>}
          {folder.dialog}
        </Group>
      );
    default:
      return null;
  }
}

export function ProcessingStatus({
  detail,
  live,
  speakers,
  waits = detail.entry.waits_for,
  onChanged,
}: {
  detail: SessionDetail;
  live: Live | undefined;
  /// The latest speaker progress event.
  speakers?: SpeakerProgressView;
  /// What the note waits for, from the newest read of the note (KTD10).
  waits?: WaitView | null;
  onChanged: () => void;
}) {
  const state = detail.entry.state;
  const active = state === "queued" || state === "processing";
  return (
    <div className="grid gap-5">
      {/* A status line, not a notice: a wait is normal (CLAUDE.md). */}
      {active && waits && <p className="text-label-secondary">{waitText(waits)}</p>}
      <Exception detail={detail} onDone={onChanged} />
      <Group>
        {stageRows(detail, active ? waits : null).map((row) => (
          <StageLine
            key={row.stage}
            row={labelOf(row, live)}
            detail={detail}
            fraction={progressOf(row, detail, live)}
            step={row.stage === "summarize" ? (live?.summary_step ?? undefined) : undefined}
            meter={row.stage === "diarize" && row.status === "running" ? speakerMeter(detail, speakers) : null}
            sent={detail.done.includes("summarize")}
          />
        ))}
      </Group>
    </div>
  );
}

/// One stage with its status, and its progress while it runs.
function StageLine({
  row,
  detail,
  fraction,
  meter,
  sent,
  step,
}: {
  row: StageRow;
  detail: SessionDetail;
  fraction: number | null;
  meter?: SpeakerMeter | null;
  sent: boolean;
  /// The summary step. Each step gets a new bar, so the fill does not
  /// move back to 0% at the next step.
  step?: string;
}) {
  return (
    <Row
      label={
        <span className="flex items-center gap-2">
          <StageIcon status={row.status} />
          <span className={row.status === "pending" ? "text-label-secondary" : undefined}>{row.label}</span>
        </span>
      }
      detail={
        row.stage === "summarize" ? (
          <span className="ml-6 block">
            <SummarizeDetail detail={detail} sent={sent} />
          </span>
        ) : undefined
      }
    >
      {/* With no provider, the note is saved without a summary (R11). */}
      {row.stage === "summarize" && !detail.endpoint && <SetUpProviderButton />}
      {meter && <SpeakerProgress meter={meter} />}
      {fraction !== null && (
        <>
          <span className="text-label-secondary tabular-nums">{Math.round(fraction * 100)}%</span>
          <ProgressBar key={step} fraction={fraction} />
        </>
      )}
    </Row>
  );
}

/// A saved note whose summary is written again: the summarize stage alone,
/// as the processing view shows it. It waits for its turn in the summary
/// queue until `started` (KTD10). The progress shows only when the
/// transcript is summarized in more than one part.
export function RegenerateStatus({
  detail,
  live,
  started,
}: {
  detail: SessionDetail;
  live: Live | undefined;
  /// The regeneration has its turn: its summarize stage started.
  started: boolean;
}) {
  const row: StageRow = started
    ? labelOf({ stage: "summarize", label: stageLabel("summarize"), status: "running" }, live)
    : { stage: "summarize", label: waitText("summary_turn"), status: "pending" };
  const fraction = started && live && live.stage === "summarize" && live.total > 1 ? live.done / live.total : null;
  return (
    <Group>
      <StageLine
        row={row}
        detail={{ ...detail, summary_problem: null }}
        fraction={fraction}
        sent={false}
        step={live?.summary_step ?? undefined}
      />
    </Group>
  );
}
