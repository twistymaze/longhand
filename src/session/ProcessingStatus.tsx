// A stopped session on its way to the destination (F2, R14, R30), on the
// Summary and Transcript tabs: each stage with its status, what the summary
// stage sends and to whom, and the one action each exception needs. The
// page reads the detail again whenever the session moves (AE4). The only
// motion is a progress bar that fills.

import { useState } from "react";
import { commands, type SessionDetail } from "../bindings";
import { type StageRow, sentence, stageRows, stoppedAt } from "../processing/describe";
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
  if (row.status !== "running") return null;
  if (row.stage === "transcribe" && detail.windows_total) {
    return (detail.windows_done ?? 0) / detail.windows_total;
  }
  if (live && live.stage === row.stage && live.total > 0) return live.done / live.total;
  return null;
}

function SummarizeDetail({ detail }: { detail: SessionDetail }) {
  if (detail.summary_problem) {
    return <Status tone="caution">Skipped. {sentence(detail.summary_problem)}</Status>;
  }
  const endpoint = detail.endpoint;
  if (!endpoint) return <>No endpoint is set, so the note is saved without a summary.</>;
  const sent = detail.done.includes("summarize");
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
                // Choosing a folder retries every session waiting for one.
                void commands.chooseDestination().then(onDone)
              }
            >
              Choose Folder…
            </Button>
          </Row>
          {error && <ErrorText>{error}</ErrorText>}
        </Group>
      );
    default:
      return null;
  }
}

export function ProcessingStatus({
  detail,
  live,
  waitsForRecording,
  onChanged,
}: {
  detail: SessionDetail;
  live: Live | undefined;
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
        {stageRows(detail).map((row) => {
          const fraction = progressOf(row, detail, live);
          return (
            <Row
              key={row.stage}
              label={
                <span className="flex items-center gap-2">
                  <StageIcon status={row.status} />
                  <span className={row.status === "pending" ? "text-label-secondary" : undefined}>{row.label}</span>
                </span>
              }
              detail={
                row.stage === "summarize" ? (
                  <span className="ml-6 block">
                    <SummarizeDetail detail={detail} />
                  </span>
                ) : undefined
              }
            >
              {fraction !== null && (
                <>
                  <span className="text-label-secondary tabular-nums">{Math.round(fraction * 100)}%</span>
                  <ProgressBar fraction={fraction} />
                </>
              )}
            </Row>
          );
        })}
      </Group>
    </div>
  );
}
