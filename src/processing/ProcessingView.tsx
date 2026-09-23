// A stopped session on its way to the destination (F4, R11a, R30): the
// stages with where each stands, what the summary stage sends and to whom,
// and the one action each exception needs. Rebuilt from the store whenever
// the session moves (AE4); the only motion is a progress bar filling.

import { useCallback, useEffect, useState } from "react";
import { commands, type SessionDetail, type SessionEntry } from "../bindings";
import { Button, ErrorText, Group, ProgressBar, Row, Status } from "../settings/ui";
import { NotesPane } from "../session/NotesPane";
import type { Live } from "../session/useSessions";
import { type StageRow, formatDuration, sentence, sessionTitle, stageRows, stoppedAt } from "./describe";

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
  if (!endpoint) return <>No endpoint is set, so the session is saved without a summary.</>;
  const sent = detail.done.includes("summarize");
  return (
    <>
      {sent ? "Sent" : "Sends"} the transcript and notes to {endpoint.host} ({endpoint.model})
      {!endpoint.encrypted && <span className="text-caution">, unencrypted</span>}.
    </>
  );
}

function Exception({
  detail,
  onDone,
}: {
  detail: SessionDetail;
  onDone: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const run = (action: () => Promise<{ status: "ok" } | { status: "error"; error: string }>) =>
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
          footer={kept.length > 0 && `The ${kept.join(" and ")} ${kept.length > 1 ? "are" : "is"} kept; Retry picks up where this stopped.`}
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
          <Row label={<Status tone="caution">Waiting for the keychain</Status>} detail={detail.message && sentence(detail.message)}>
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
          <Row label={<Status tone="caution">Waiting for a folder</Status>} detail={detail.message && sentence(detail.message)}>
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

export function ProcessingView({
  entry,
  version,
  live,
  recording,
  onChanged,
}: {
  entry: SessionEntry;
  version: number;
  live: Live | undefined;
  recording: boolean;
  onChanged: () => void;
}) {
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await commands.sessionDetail(entry.id);
    if (result.status === "ok") {
      setDetail(result.data);
      setError(null);
    } else setError(result.error);
  }, [entry.id]);

  useEffect(() => {
    void load();
  }, [load, version, entry.state]);

  const changed = () => {
    void load();
    onChanged();
  };

  const [resumeError, setResumeError] = useState<string | null>(null);
  const resume = () =>
    void commands.resumeRecording(entry.id).then((r) => {
      setResumeError(r.status === "error" ? r.error : null);
      changed();
    });

  const waiting = entry.state === "queued" || entry.state === "processing";

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-60 shrink-0 flex-col border-r border-separator">
        <NotesPane key={entry.id} sessionId={entry.id} />
      </aside>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[34rem] px-6 pt-5 pb-8">
          <header className="mb-4 flex items-start gap-3">
            <div className="min-w-0 flex-1">
              <h1 className="text-[1.35rem] leading-tight font-semibold tracking-[-0.01em]">
                {sessionTitle(entry.started)}
              </h1>
              <p className="text-label-secondary">
                {formatDuration(entry.duration_ms)} recorded
                {waiting && recording && " · paused while you record"}
              </p>
            </div>
            {detail?.resumable && <Button onClick={resume}>Resume Recording</Button>}
          </header>
          {resumeError && <p className="mb-3 text-[0.85rem] text-negative">{resumeError}</p>}
          {error && <p className="mb-3 text-[0.85rem] text-negative">{error}</p>}
          {detail && (
            <div className="grid gap-5">
              <Exception detail={detail} onDone={changed} />
              <Group>
                {stageRows(detail).map((row) => {
                  const fraction = progressOf(row, detail, live);
                  return (
                    <Row
                      key={row.stage}
                      label={
                        <span className="flex items-center gap-2">
                          <StageIcon status={row.status} />
                          <span className={row.status === "pending" ? "text-label-secondary" : undefined}>
                            {row.label}
                          </span>
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
          )}
        </div>
      </div>
    </div>
  );
}
