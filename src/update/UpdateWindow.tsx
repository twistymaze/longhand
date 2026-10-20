// The update window (KTD9): a small window in the form of a macOS alert.
// It shows the state of the update manager and sends the user's choice.
// The manager opens and closes the window. A close by the user counts as
// the left button's choice.
//
// An automatic notice can open while the user types in a note, so the page
// then ignores the keyboard: no button gets the focus, and Return and Space
// do not click. A window that the user asked for (the tray item or "Check
// for Updates…") has the focus: the default button gets the focus, and
// Return chooses it.

import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { commands, events, type Choice, type UpdateState } from "../bindings";
import { AppIcon } from "../settings/icons";
import { sentence } from "../processing/describe";
import { Button, formatBytes } from "../settings/ui";

/// A prominent button is blue, as the default button of a macOS alert.
type Action = { label: string; choice: Choice; prominent?: boolean };

/// The text and the buttons of one state. The last button is at the right,
/// as the default button of a macOS alert.
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
        message: "Local builds do not update. To get updates, install the latest release.",
        buttons: [ok],
      };
    default:
      return null;
  }
}

/// Before the first bytes arrive, the text is a no-break space. The line then
/// keeps its height, and the window does not change size.
function progressText({ downloaded, total }: { downloaded: number; total: number | null }): string {
  if (downloaded === 0) return "\u00a0";
  return total ? `${formatBytes(downloaded)} of ${formatBytes(total)}` : formatBytes(downloaded);
}

/// Size the window to the page, as a macOS alert sizes to its text. The
/// window has no interface zoom, so one CSS pixel is one logical pixel.
///
/// On macOS the window can show less than the requested size. The page
/// requests the content size plus an offset. After each window resize, the
/// offset is the last request minus the viewport. The page reads the
/// viewport only then, because before the resize it shows the old size.
///
/// The window opens hidden. After the first resize that needs no new
/// request, the page asks the app to show the window.
function useWindowFitsContent(content: RefObject<HTMLElement | null>, shown: boolean) {
  const showRequested = useRef(false);
  useLayoutEffect(() => {
    const element = content.current;
    if (!element) return;
    let requested: { width: number; height: number } | null = null;
    let offset = { width: 0, height: 0 };
    // True when the window already has the size that fits the content.
    const fit = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return false;
      const width = Math.ceil(rect.width) + offset.width;
      const height = Math.ceil(rect.height) + offset.height;
      if (requested?.width === width && requested.height === height) return true;
      requested = { width, height };
      getCurrentWindow().setSize(new LogicalSize(width, height)).catch(() => {});
      return false;
    };
    const windowResized = () => {
      if (requested) {
        offset = { width: requested.width - window.innerWidth, height: requested.height - window.innerHeight };
      }
      if (fit() && !showRequested.current) {
        showRequested.current = true;
        void commands.showUpdateWindow();
      }
    };
    fit();
    const contentResized = new ResizeObserver(fit);
    contentResized.observe(element);
    window.addEventListener("resize", windowResized);
    return () => {
      contentResized.disconnect();
      window.removeEventListener("resize", windowResized);
    };
  }, [content, shown]);
}

export function UpdateWindow() {
  const [state, setState] = useState<UpdateState | null>(null);
  const [withKey, setWithKey] = useState(false);
  useEffect(() => {
    // The event can come before the first read ends; the event is newer.
    let heard = false;
    const unlisten = events.updateStateChanged.listen(({ payload }) => {
      heard = true;
      setState(payload.state);
      setWithKey(payload.with_key);
    });
    commands.updateState().then((s) => {
      if (heard) return;
      setState(s.state);
      setWithKey(s.with_key);
    }, () => {});
    return () => void unlisten.then((f) => f());
  }, []);

  const view = state ? viewOf(state) : null;
  const content = useRef<HTMLElement>(null);
  useWindowFitsContent(content, view !== null);
  const defaultAction = view?.buttons.find((b) => b.prominent);
  const defaultButton = useRef<HTMLButtonElement>(null);
  // WebKit does not focus a clicked button, so the focus is set here.
  useEffect(() => {
    if (withKey) defaultButton.current?.focus();
  }, [withKey, defaultAction?.choice]);

  if (!view) return <main className="h-screen bg-window" />;
  const choose = (choice: Choice) => void commands.chooseUpdate(choice);
  // The fraction is null only when bytes arrive and the server sent no size.
  // Before the first bytes arrive, the bar is empty.
  const progress = view.progress;
  const fraction = !progress
    ? null
    : progress.total
      ? Math.min(1, progress.downloaded / progress.total)
      : progress.downloaded === 0
        ? 0
        : null;

  return (
    <main
      ref={content}
      className="flex w-[480px] gap-4 bg-window px-5 pt-5 pb-4 text-label select-none"
      // Without the focus, Return and Space do not click a button that has
      // the focus all the same. With it, Return chooses the default button,
      // and Space clicks the button that has the focus. A click from the
      // pointer or from VoiceOver always chooses.
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== " ") return;
        if (e.key === " " && withKey) return;
        e.preventDefault();
        if (e.key === "Enter" && withKey && defaultAction) choose(defaultAction.choice);
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
          {view.buttons.map((b) => (
            <Button
              key={b.choice}
              ref={b === defaultAction ? defaultButton : undefined}
              prominent={b.prominent}
              onClick={() => choose(b.choice)}
            >
              {b.label}
            </Button>
          ))}
        </div>
      </div>
    </main>
  );
}
