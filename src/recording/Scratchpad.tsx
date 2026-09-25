// The open session's notes while it records (R20). Notes exist only inside a
// session (R27), so with no session open there is nothing to type into.
// The recording page replaces this (U13).

import { NotesPane } from "../session/NotesPane";

export function Scratchpad({ sessionId }: { sessionId: string | null }) {
  if (sessionId === null) {
    return <p className="px-3 py-2 text-label-tertiary">Notes open when you start recording.</p>;
  }
  return <NotesPane key={sessionId} sessionId={sessionId} autoFocus />;
}
