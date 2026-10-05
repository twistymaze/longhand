// The update window (KTD9): a small window in the form of a macOS alert.
// It shows the state of the update manager and sends the user's choice.
// The manager opens and closes the window. A close by the user counts as
// the left button's choice.

import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { commands, events, type Choice, type UpdateState } from "../bindings";
import { AppIcon } from "../settings/icons";
import { sentence } from "../processing/describe";
import { Button, formatBytes } from "../settings/ui";

/// A prominent button is blue, as the default button of a macOS alert.
type Action = { label: string; choice: Choice; prominent?: boolean };

/// The text and the buttons of one state. The last button is the default.
type View = { title: string; message?: string; progress?: { downloaded: number; total: number | null }; buttons: Action[] };

const ok: Action ={ label: "OK", choice: "close", prominent: true };

export function viewOf(state: UpdateState): View | null {
  switch (state.state) {
    case "checking":
      return { title: "Checking for updates…", buttons: [{ label: "Cancel", choice: "close" }] };
    case "offered":
      return {
        title: "A new version of Longhand is available",
        message: `Longhand ${state.version} is available. You have ${state.current}.`,
        buttons: [
          { label: "Remind Me Later", choice: "remind_later" },
          { label: "Install and Relaunch", choice: "install", prominent: true },
        ],
      };
    case "downloading":
      return {
        title: `Downloading Longhand ${state.version}…`,
        progress: { downloaded: state.downloaded, total: state.total },
        buttons: [{ label: "Hide", choice: "close" }],
      };
    case "failed":
      return {
        title: "The update could not be installed",
        message: `${sentence(state.error)} Your installed version did not change.`,
        buttons: [
          { label: "Close", choice: "close" },
          { label: "Try Again", choice: "try_again", prominent: true },
        ],
      };
    case "ready":
      return {
        title: `Longhand ${state.version} is ready`,
        message: "Relaunch now to use it. If you choose Later, it installs when you quit Longhand.",
        buttons: [
          { label: "Later", choice: "later" },
          { label: "Relaunch Now", choice: "relaunch_now", prominent: true },
        ],
      };
    case "up_to_date":
      return { title: "Longhand is up to date", message: `You have the newest version, ${state.current}.`, buttons: [ok] };
    case "check_failed":
      return { title: "Longhand could not check for updates", message: sentence(state.error), buttons: [ok] };
    case "local_build":
      return {
        title: "This build gets no updates",
        message: "Local builds do not update. To get updates, install the newest preview build.",
        buttons: [ok],
      };
    default:
      return null;
  }
}

function progressText({ downloaded, total }: { downloaded: number; total: number | null }): string {
  return total ? `${formatBytes(downloaded)} of ${formatBytes(total)}` : formatBytes(downloaded);
}

/// The window size that shows all of the page. The webview zoom scales CSS
/// pixels, so one CSS pixel is `scale` logical pixels.
export function windowSize(width: number, height: number, scale: number): LogicalSize {
  return new LogicalSize(Math.ceil(width * scale), Math.ceil(height * scale));
}

function interfaceScale(): number {
  const scale = Number((window as { __interfaceScale?: string }).__interfaceScale);
  return Number.isFinite(scale) && scale > 0 ? scale : 1;
}

/// Size the window to the page, as a macOS alert sizes to its text. The page
/// sets the size again when its content or the interface zoom changes.
function useWindowFitsContent(content: RefObject<HTMLElement | null>, shown: boolean) {
  useLayoutEffect(() => {
    const element = content.current;
    if (!element) return;
    const fit = () => {
      const { width, height } = element.getBoundingClientRect();
      if (width === 0 || height === 0) return;
      getCurrentWindow().setSize(windowSize(width, height, interfaceScale())).catch(() => {});
    };
    fit();
    const resized = new ResizeObserver(fit);
    resized.observe(element);
    // A zoom change sets the scale property on the root element.
    const zoomed = new MutationObserver(fit);
    zoomed.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
    return () => {
      resized.disconnect();
      zoomed.disconnect();
    };
  }, [content, shown]);
}

export function UpdateWindow() {
  const [state, setState] = useState<UpdateState | null>(null);
  useEffect(() => {
    // The event can come before the first read ends; the event is newer.
    let heard = false;
    const unlisten = events.updateStateChanged.listen(({ payload }) => {
      heard = true;
      setState(payload.state);
    });
    commands.updateState().then((s) => {
      if (!heard) setState(s);
    }, () => {});
    return () => void unlisten.then((f) => f());
  }, []);

  const view = state ? viewOf(state) : null;
  const defaultButton = useRef<HTMLButtonElement>(null);
  const title = view?.title;
  // WKWebView does not focus a button on click, so the page sets the focus
  // itself. Return then activates the default button.
  useEffect(() => defaultButton.current?.focus(), [title]);
  const content = useRef<HTMLElement>(null);
  useWindowFitsContent(content, view !== null);

  if (!view) return <main className="h-screen bg-window" />;
  const choose = (choice: Choice) => void commands.chooseUpdate(choice);
  const cancel = view.buttons[0]!;
  const fraction = view.progress?.total ? Math.min(1, view.progress.downloaded / view.progress.total) : null;

  return (
    <main
      ref={content}
      className="flex w-[480px] gap-4 bg-window px-5 pt-5 pb-4 text-label select-none"
      onKeyDown={(e) => {
        if (e.key === "Escape") choose(cancel.choice);
      }}
    >
      <AppIcon className="size-14" />
      <div className="flex min-w-0 flex-1 flex-col">
        <h1 className="font-semibold [overflow-wrap:anywhere]" aria-live="polite">
          {view.title}
        </h1>
        {view.message && (
          <p className="mt-1 text-[0.85rem] leading-[1.4] text-label-secondary select-text [overflow-wrap:anywhere]">
            {view.message}
          </p>
        )}
        {view.progress && (
          <div className="mt-3">
            <div
              role="progressbar"
              aria-label="Download progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
              className="h-1.5 overflow-hidden rounded-full bg-fill"
            >
              <div
                className={`h-full rounded-full bg-accent ${fraction === null ? "w-1/3 animate-pulse" : ""}`}
                style={fraction === null ? undefined : { width: `${fraction * 100}%` }}
              />
            </div>
            <p className="mt-1.5 text-[0.8rem] tabular-nums text-label-secondary">{progressText(view.progress)}</p>
          </div>
        )}
        <div className="flex justify-end gap-2 pt-4">
          {view.buttons.map((b, i) => {
            const isDefault = i === view.buttons.length - 1;
            return (
              <Button
                key={b.choice}
                ref={isDefault ? defaultButton : undefined}
                prominent={b.prominent}
                onClick={() => choose(b.choice)}
              >
                {b.label}
              </Button>
            );
          })}
        </div>
      </div>
    </main>
  );
}
