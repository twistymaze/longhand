// The window with the full licence list (R9, R10): the text for the running
// platform, as plain text that scrolls and that the person can select and
// copy. The three part titles are headings for screen readers; on screen
// they look the same as the text around them.

import { type ReactNode, useEffect, useState } from "react";
import { commands } from "../bindings";
import { PART_TITLES } from "../../scripts/licences/group";

/// The part titles that the licence generator writes, each on its own line.
const HEADINGS = new Set<string>(PART_TITLES.map((title) => title.toLowerCase()));

/// Split the text into runs of plain text and headings. Each heading stays
/// inline, and the line breaks stay in the text, so the layout is the same
/// as the plain text.
function withHeadings(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  let run = "";
  const lines = text.split("\n");
  lines.forEach((line, i) => {
    const end = i < lines.length - 1 ? "\n" : "";
    if (HEADINGS.has(line.trim().toLowerCase())) {
      if (run) parts.push(run);
      run = end;
      parts.push(
        <h2 key={i} className="inline">
          {line}
        </h2>,
      );
    } else {
      run += line + end;
    }
  });
  if (run) parts.push(run);
  return parts;
}

export function LicencesWindow() {
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    commands.licenceText().then(setText, () => setFailed(true));
  }, []);

  return (
    <main className="h-screen overflow-y-auto bg-pane text-label">
      {failed && (
        <p className="px-6 py-5 text-label-secondary">
          The licence list could not be loaded. Close this window and open it again.
        </p>
      )}
      {text !== null && (
        <div
          data-testid="licence-text"
          className="px-6 py-5 font-mono text-[0.78rem] leading-[1.5] break-words whitespace-pre-wrap select-text"
        >
          {withHeadings(text)}
        </div>
      )}
    </main>
  );
}
