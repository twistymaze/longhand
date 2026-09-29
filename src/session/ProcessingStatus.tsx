// A stopped session on its way to the destination (F2, R14, R30), on the
// Summary and Transcript tabs: each stage with its status, what the summary
// stage sends and to whom, and the one action each exception needs. The
// page reads the detail again whenever the session moves (AE4). The only
// motion is a progress bar that fills, and a quiet sweep on the speaker bar
// while its work cannot be measured. Regenerate Summary on a saved note
// shows the summarize stage in the same form.

import { useEffect, useState } from "react";
import { commands, type SessionDetail, type SpeakerProgressView } from "../bindings";
import {
  type SpeakerMeter,
  type StageRow,
  sentence,
  speakerMeter,
  stageLabel,
  stageRows,
  stoppedAt,
} from "../processing/describe";
import { useFolderChange } from "../settings/OldNotesDialog";
import { Button, ErrorText, Group, ProgressBar, Row, Status } from "../settings/ui";
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
  if (live && live.stage === row.stage && live.total > 0) return live.done / live.total;
  return null;
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

function SummarizeDetail({ detail, sent }: { detail: SessionDetail; sent: boolean }) {
  if (detail.summary_problem) {
    return <Status tone="caution">Skipped. {sentence(detail.summary_problem)}</Status>;
  }
  const endpoint = detail.endpoint;
  if (!endpoint) return <>No endpoint is set, so the note is saved without a summary.</>;
  return (
    <>
      {sent ? "Sent" : "Sends"} the transcript and notes to {endpoint.host} ({endpoint.model})
      {!endpoint.encrypted && <span className="text-caution">, unencrypted</span>}.
    </>
  );
}

type Outcome = { status: "ok" } | { status: "error"; error: string };

function Exception({ detail, onDone }: { detail: SessionDetail; onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
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
      return (
        <Group
          footer={
            kept.length > 0 &&
            `The ${kept.join(" and ")} ${kept.length > 1 ? "are" : "is"} kept. Retry continues from where processing stopped.`
          }
        >
          <Row
            label={<Status tone="negative">{stoppedAt(detail.failed_stage)}</Status>}
            detail={detail.message && sentence(detail.message)}
          >
            <Button prominent onClick={() => run(() => commands.retrySession(id))}>
              Retry
            </Button>
          </Row>
          {error && <ErrorText>{error}</ErrorText>}
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
            <Button onClick={() => run(() => commands.retrySession(id))}>Try Again</Button>
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
  waitsForRecording,
  onChanged,
}: {
  detail: SessionDetail;
  live: Live | undefined;
  /// The latest speaker progress event.
  speakers?: SpeakerProgressView;
  /// Another session records, and processing waits for it to stop.
  waitsForRecording: boolean;
  onChanged: () => void;
}) {
  const state = detail.entry.state;
  const waiting = state === "queued" || state === "processing";
  return (
    <div className="grid gap-5">
      {waiting && waitsForRecording && (
        <p className="text-label-secondary">Processing starts when the current recording stops.</p>
      )}
      <Exception detail={detail} onDone={onChanged} />
      <Group>
        {stageRows(detail).map((row) => (
          <StageLine
            key={row.stage}
            row={row}
            detail={detail}
            fraction={progressOf(row, detail, live)}
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
}: {
  row: StageRow;
  detail: SessionDetail;
  fraction: number | null;
  meter?: SpeakerMeter | null;
  sent: boolean;
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
      {meter && <SpeakerProgress meter={meter} />}
      {fraction !== null && (
        <>
          <span className="text-label-secondary tabular-nums">{Math.round(fraction * 100)}%</span>
          <ProgressBar fraction={fraction} />
        </>
      )}
    </Row>
  );
}

/// A saved note whose summary is written again: the summarize stage alone,
/// as the processing view shows it. The progress shows only when the
/// transcript is summarized in more than one part.
export function RegenerateStatus({ detail, live }: { detail: SessionDetail; live: Live | undefined }) {
  const row: StageRow = { stage: "summarize", label: stageLabel("summarize"), status: "running" };
  const fraction = live && live.stage === "summarize" && live.total > 1 ? live.done / live.total : null;
  return (
    <Group>
      <StageLine row={row} detail={{ ...detail, summary_problem: null }} fraction={fraction} sent={false} />
    </Group>
  );
}
