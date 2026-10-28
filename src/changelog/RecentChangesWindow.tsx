// The Recent changes window (U5 of the monthly changelog plan; R10, R11,
// R12, F1). It shows the newest 6 calendar months of the changelog that the
// build contains, and a link to the full changelog page. A changelog that is
// not available gives only the link (AE7). The window shows no version
// numbers (R6).

import { useEffect, useState } from "react";
import { commands } from "../bindings";
import { WebLink } from "../settings/About";
import { ChangelogMonths } from "./ChangelogMonths";
import { type ChangelogMonth, PAGES_URL, recentMonths } from "./months";

export function RecentChangesWindow() {
  // Null until the command returns.
  const [months, setMonths] = useState<ChangelogMonth[] | null>(null);

  useEffect(() => {
    // A failed call gives the same window as "not available".
    commands.changelog().then(
      (changelog) => setMonths(changelog.available ? recentMonths(changelog.entries) : []),
      () => setMonths([]),
    );
  }, []);

  return (
    <main className="h-screen overflow-y-auto bg-pane text-label">
      {months !== null && (
        <div className="px-6 py-5 select-text">
          <ChangelogMonths months={months} />
          <p className={months.length > 0 ? "mt-6" : ""}>
            <WebLink url={PAGES_URL}>Full changelog</WebLink>
          </p>
        </div>
      )}
    </main>
  );
}
