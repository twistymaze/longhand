// The changelog months as a list (R7, KTD3 of the monthly changelog plan): a
// heading for each month, a heading for each group, and the lines. A line
// with a PR opens the PR page in the default browser. A line with no PR is
// plain text. Recent changes and the update window (U6) use this list.
//
// A PR line is a block button. An inline button lays out as inline-block,
// whose baseline is its last line, so the bullet would align with the last
// line of a wrapped line.

import { openUrl } from "@tauri-apps/plugin-opener";
import type { ChangelogEntry } from "../bindings";
import { type ChangelogMonth, prAddress } from "./months";

function Line({ entry }: { entry: ChangelogEntry }) {
  if (entry.pr === null) return <>{entry.line}</>;
  const url = prAddress(entry.pr);
  return (
    <button
      type="button"
      onClick={() => void openUrl(url)}
      className="block rounded-[3px] text-left [overflow-wrap:anywhere] hover:underline active:opacity-70 outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
    >
      {entry.line}
      <span className="sr-only"> (opens in your browser)</span>
    </button>
  );
}

export function ChangelogMonths({ months }: { months: ChangelogMonth[] }) {
  return (
    <>
      {months.map((month) => (
        <section key={month.month} className="mt-6 first:mt-0">
          <h2 className="text-[1.05rem] leading-[1.3] font-semibold tracking-[-0.01em]">{month.name}</h2>
          {month.groups.map((group) => (
            <div key={group.rank} className="mt-3">
              <h3 className="text-[0.85rem] font-semibold text-label">{group.group}</h3>
              <ul className="mt-1 list-disc space-y-1 pl-5 text-[0.9rem] leading-[1.45] marker:text-label-tertiary">
                {group.entries.map((entry) => (
                  <li key={entry.id}>
                    <Line entry={entry} />
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </section>
      ))}
    </>
  );
}
