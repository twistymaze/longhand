// The footer under the note list: Settings, and the version with its
// Check for Updates… menu, or the waiting update in its place (the library
// footer plan). It is quiet text that changes only when an update comes or
// goes, so it does not draw the eye during a meeting.

import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { commands, events } from "../bindings";
import { Menu } from "../session/SessionMenu";
import { SlidersIcon } from "../settings/icons";

const quiet =
  "min-w-0 truncate rounded-[6px] px-2 py-[3px] text-[0.85rem] text-label-secondary hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent";

export function LibraryFooter() {
  const [version, setVersion] = useState<string | null>(null);
  const [offer, setOffer] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const versionButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // A failed call leaves the version out; Settings still works.
    commands.buildInfo().then((info) => setVersion(info.version), () => {});
    commands.updateOffer().then(setOffer, () => {});
    const unlisten = events.updateOfferChanged.listen(({ payload }) => setOffer(payload.version));
    return () => void unlisten.then((f) => f());
  }, []);

  const close = () => {
    setOpen(false);
    versionButton.current?.focus();
  };

  // A button opens its menu with Enter and Space; a menu button also opens
  // it with the down arrow (R7).
  const arrowOpens = (event: KeyboardEvent) => {
    if (event.key !== "ArrowDown") return;
    event.preventDefault();
    setOpen(true);
  };

  return (
    <div className="flex shrink-0 items-center gap-2 border-t border-separator px-[16px] py-1.5 select-none">
      <button type="button" className={`${quiet} inline-flex shrink-0 items-center gap-1.5`} onClick={() => void commands.openSettings(null)}>
        <SlidersIcon className="size-3.5" />
        Settings
      </button>
      <span className="flex-1" />
      {offer !== null ? (
        <button
          type="button"
          className="min-w-0 truncate rounded-full bg-accent/12 px-2.5 py-[2px] text-[0.8rem] font-medium text-accent hover:bg-accent/20 outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
          onClick={() => void commands.openUpdateOffer()}
        >
          Update available: {offer}
        </button>
      ) : (
        version !== null && (
          <span className="relative inline-flex min-w-0">
            <button
              ref={versionButton}
              type="button"
              className={`${quiet} tabular-nums`}
              aria-haspopup="menu"
              aria-expanded={open}
              onKeyDown={arrowOpens}
              onClick={(e) => {
                // WKWebView does not focus a clicked button. The focus must
                // be here, so that Escape in the menu gives it back.
                e.currentTarget.focus();
                setOpen((o) => !o);
              }}
            >
              Version {version}
            </button>
            {open && (
              <Menu
                floating={versionButton}
                onClose={close}
                width="w-max min-w-44"
                items={[
                  {
                    label: "Check for Updates…",
                    onSelect: () => {
                      close();
                      void commands.checkForUpdates();
                    },
                  },
                ]}
              />
            )}
          </span>
        )
      )}
    </div>
  );
}
