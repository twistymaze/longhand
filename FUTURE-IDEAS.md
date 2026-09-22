# Future ideas

A parking lot. Nothing here is committed, and nothing here is scoped — these are things that came up while working out the walking skeleton and seemed worth not losing. Read it when deciding what to brainstorm next.

Items marked **ruled out** conflict with a boundary in `STRATEGY.md` and are recorded so the reasoning isn't re-derived, not because they're waiting.

## Deferred from the walking skeleton

These were cut deliberately to reach something usable. `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md` holds the reasoning.

- **System audio capture.** The second capture path and its permission surface. Needed for meetings on calls; irrelevant to a room.
- **Summary kinds derived from the folder.** A Pathfinder folder recaps a session; a work folder produces decisions and follow-ups. Cut because one summary shape gets to Friday faster.
- **Custom vocabulary.** A word list per folder correcting proper nouns and jargon. Handy's fuzzy matcher is borrowable as-is. Cheapest deferred item; deliberately left out so its absence can be felt first.
- **The live streaming transcript view.** The skeleton is honest about whether the model supports one; showing it is the follow-up.

## Capture and reliability

- **Prevent-sleep opt-out.** Holding a sleep assertion for four hours on battery is a real cost. A setting for "let the laptop survive the session instead."
- **A shortcut per folder.** Bind a hotkey per destination — one for Pathfinder, one for work. Keeps recording decision-free while making the destination implicit. Considered and set aside in favor of a default folder you move later; worth revisiting if moving becomes tedious.
- **Importing an existing recording.** Drop in a file from a dedicated recorder and process it as a session. Sidesteps capture entirely and takes the simplest diarization path. Raised early, never resolved.
- **A fixed speaker count per folder.** The diarization engine accepts an exact or bounded speaker count and does better when it has one. A recurring table is always the same size. This is a *count*, not identity, so it does not collide with the no-speaker-identity boundary — worth confirming that reading before building on it.

## Transcript and summary quality

- **Timestamps in the summarization prompt.** anarlog produces per-segment and per-word timings and then drops them at render time, so its summarizer can never say when something happened. Keeping them would let a recap say "around the two-hour mark." Small change, real payoff for long sessions.
- **Session-to-session context.** Feeding the previous session's summary in as context when summarizing the next one — "previously on," and continuity of names and threads across a campaign. Neither reference project does this. Fits the Pathfinder case almost too well.
- **A diarization evaluation loop.** anarlog ships a diarization-error-rate harness and a documented tuning procedure. Running it against real session audio would answer whether one microphone and eight people actually works, rather than assuming.
- **Alternative diarization architectures.** If the pyannote path disappoints: MOSS does joint transcription and diarization with no speaker ceiling, and Sortformer runs as a separate speaker track joined against a strong transcription model. Both have caveats recorded in `docs/reference/borrowing-notes.md`.
- **A transcript review and correction surface.** Virtualized for thousands of turns, with click-to-seek, turn merge and split, and find-and-replace for names the model mangled. Needed eventually; large enough to deserve its own brainstorm.

## Processing

- **Scheduling processing around power and thermals.** A four-hour session is twenty to sixty minutes of sustained local compute. Deferring it to when the machine is idle and plugged in is friendlier than running it immediately on battery.
- **Finer-grained checkpointing.** The skeleton requires processing to survive a quit. How much work a restart costs is a separate question worth tuning once there's something to measure.

## Handoff

These are already named as separate brainstorms, listed so the parking lot is complete.

- **Library and browsing** — finding the session from three weeks ago.
- **Obsidian vault sync.**
- **The local CLI** over the session store.

## Ruled out

- **Speaker identity** — enrollment, voiceprints, names attached to speakers. Turn boundaries are the requirement. Generic labels that drift across a long session are acceptable.
- **Calendar integration** — including auto-start and prefill. Revisit only if a genuinely simple path appears inside an enterprise.
- **Any organization scheme beyond folders** — no linking, no graph, no tags. Obsidian exists.
- **Anything that writes into Longhand** — the only inputs are what you record and what you type.
