# Feature ideas

A parking lot of feature ideas for Longhand, one file per idea. Nothing here is committed or scoped. Read it when deciding what to brainstorm next.

## Keeping this up to date

This is a running list. Add ideas as they come up in any session, not only in planning.

- **A new idea** gets its own file, `docs/ideas/<kebab-case-slug>.md`, in the format below, plus a row in the matching area table here. Check the tables first: if the idea extends an existing one, add a note to that file instead of starting a new one.
- **An idea only mentioned in passing** in a plan, brainstorm, review or conversation still belongs here. Record it here as well as wherever else it was mentioned, so the parking lot stays complete.
- **When an idea is picked up,** set its status to `in-progress` and link the brainstorm or plan from its file. When it ships, set `done` and move its row to Done. Keep the file: it records where the work came from.
- **An idea that conflicts with a boundary** in `STRATEGY.md` goes to Ruled out below with its reason. It gets no file, so the reasoning isn't re-derived later.
- **When anything changes,** update the file and its row here in the same commit.

Each idea file has this frontmatter and a short body:

```markdown
---
title: A plain-language name
status: open            # open | parked | in-progress | done
area: capture           # capture | transcript-and-summary | processing | library-and-sharing | interface
added: 2026-09-24
source: "where it came up (a plan section, a brainstorm, a conversation)"
related: [other-idea-slug]
---

# A plain-language name

## Idea
What it is, in a few sentences.

## Why
The problem it solves, or who asked. (For a parked idea: `## Why parked`.)

## Notes / Current state
Open questions, dependencies, what already exists.
```

**Status:** `open` is worth doing and waiting for its turn. `parked` was considered and set aside, with the reason in its file. `in-progress` has a brainstorm or plan. `done` has shipped.

### How this relates to the compound-engineering folders

- `docs/ideation/` is where `ce-ideate` writes each ranked ideation session. An idea that survives one of those sessions and isn't acted on belongs here.
- `docs/plans/` is where `ce-brainstorm` writes the requirements it produces, as a requirements-only plan, and where `ce-plan` adds the implementation plan. Picking an idea up usually means brainstorming it, which creates that plan; link it from the idea's file.
- `docs/brainstorms/` is a legacy location that current compound-engineering skills only read old `*-requirements` files from. Don't put idea briefs there; an idea's brief lives in its file here, with any private material beside it (see `ux-redesign.md`).
- `docs/solutions/` holds learnings from solved problems.

This folder is our own convention, not something compound engineering defines, so it has to keep fitting the plugin as the plugin changes. For example, a future version might give ideas a location of its own, move `ideation/` or where brainstorm output goes, or have a skill read or write backlog items. Whenever something like that surfaces, or the two start to disagree, raise it and resolve it: move this folder to the plugin's convention, or change how it links to the plugin's folders. Don't work around it quietly.

## Capture

| Idea | Status | In short |
|---|---|---|
| [Let the laptop sleep during a session](prevent-sleep-opt-out.md) | open | An opt-out for the four-hour sleep assertion on battery |
| [A recording shortcut per folder](shortcut-per-folder.md) | parked | A hotkey per destination; set aside for "move it later" |
| [Import an existing recording](import-existing-recording.md) | open | Process a file from a dedicated recorder as a session |
| [Remember a speaker count per folder](speaker-count-per-folder.md) | open | Per-folder default; per-session counts already exist |
| [System audio capture](system-audio-capture.md) | in-progress | Built in PR #12 and hidden; the microphone choice ships; processing waits for a new design |
| [Mixer apps and microphone copies](mixer-apps-and-microphone-copies.md) | open | Exclude mixers such as Wave Link from the tap; a GCC-PHAT copy check |

## Transcript and summary

| Idea | Status | In short |
|---|---|---|
| [Saved summary prompts](summary-prompts.md) | in-progress | Named prompts for the summary; "Topic summary" and "Play-by-play summary" built in |
| [Summary kinds chosen by folder](summary-kind-per-folder.md) | open | A recap for Pathfinder, decisions and follow-ups for work |
| [Participant names for a session](participant-names.md) | open | The user lists who was present so the summary can use names |
| [Custom vocabulary per folder](custom-vocabulary.md) | open | A word list that corrects names and jargon |
| [Carry context from the previous session](session-to-session-context.md) | open | "Previously on" for recurring sessions |
| [A diarization evaluation loop](diarization-evaluation-loop.md) | open | Measure speaker turns on real sessions, repeatedly |
| [Alternative diarization architectures](alternative-diarization-architectures.md) | parked | MOSS or Sortformer, or speaker labels from the recognition model |
| [Transcript review and correction](transcript-review-and-correction.md) | open | Seek, merge and split turns, fix mangled names |

## Processing

| Idea | Status | In short |
|---|---|---|
| [Schedule processing around power and thermals](power-aware-processing.md) | open | Wait for idle and plugged in before heavy compute |
| [Follow-ups from the live transcript review](live-transcript-review-follow-ups.md) | open | Eight small residual risks from the PR #8 review; a stuck live-model mark and a lost re-pick first |
| [Process each channel of a stereo note on its own](per-channel-processing.md) | open | Recognize and diarize room and call separately, in place of the fragile mono mix |
| [Faster diarization](faster-diarization.md) | open | Diarize is too slow now; comes before per-channel processing |

## Library and sharing

| Idea | Status | In short |
|---|---|---|
| [Library and browsing](library-and-browsing.md) | open | Search and a wider index; the list, preview, rename and delete shipped |
| [Obsidian vault sync](obsidian-vault-sync.md) | open | Keep sessions in step with a vault |
| [A local CLI](local-cli.md) | open | Command-line access to the session store |

## Interface

| Idea | Status | In short |
|---|---|---|
| [Library and desktop UI polish](ui-polish.md) | in-progress | Stable note selection, keyboard zoom, login startup, and a shared vector mark |
| [Settings attached to the main window](settings-attached-to-main-window.md) | parked | A child window or sheet that moves with the main window |
| [Clickable links in notes](clickable-links-in-notes.md) | open | Open a link in a note's markdown in the default browser |

## Done

Ideas that shipped: first those from the original list that the walking skeleton built, then later plans.

- **Timestamps in the summarization prompt.** Every transcript turn and note reaches the summarizer with its clock position, so a summary can say when something happened.
- **Finer-grained checkpointing.** Transcription checkpoints each window, so a restart redoes only unfinished windows.
- **[UX redesign](ux-redesign.md).** One main window with a Home list and one tabbed session page, a Settings window, markdown notes, titles that name the session folder, and delete to the Trash (`docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md`).
- **A speaker count per session.** An optional count or range per session, with an app-wide default (KD15, R36). What remains is the per-folder default above.
- **[Choose the recognition model](recognition-model-picker.md).** A Settings picker for nine models from Handy's catalog, with pick-to-download and delete (`docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`).
- **[Live transcript while recording](live-transcript-view.md).** A streaming model shows the transcript in the Transcript tab during recording; after Stop only the gaps get a batch pass, and "Transcribe Again…" redoes a saved note (same plan).
- **[Choose the shortcuts](custom-shortcuts.md).** Set, reset or clear the recording shortcut in Settings with no relaunch, and an optional shortcut that shows or hides the window (`docs/plans/2026-09-25-1245-feat-custom-shortcuts-plan.md`).
- **[Choose the summary model from the server's list](summary-model-picker.md).** A searchable Model picker filled from the endpoint's model list, a status line that checks the Base URL and API key, and OpenAI as the Base URL on a new install (`docs/plans/2026-09-25-1244-feat-summary-model-picker-plan.md`).
- **[Acknowledgements and full licences in an About pane](about-acknowledgements-and-licences.md).** Settings → About thanks anarlog and Handy and the core engines, lists the models and major components with their licences, and opens a window with the full licence text of every shipped component, checked for drift in CI (`docs/plans/2026-09-26-1259-feat-about-acknowledgements-and-licences-plan.md`, PR #13).
- **[One library across computers](cross-machine-library.md).** Every computer that shares a synced storage folder lists every note in it, follows changes from the other computers while it runs, and can do every action on every note, Resume included (`docs/plans/2026-09-25-1217-feat-one-library-across-computers-plan.md`).

## Ruled out

These conflict with a boundary in `STRATEGY.md`.

- **Speaker identity:** enrollment, voiceprints, names attached to speakers. Turn boundaries are the requirement; generic labels that drift across a long session are acceptable.
- **Calendar integration,** including auto-start and prefill. Revisit only if a genuinely simple path appears inside an enterprise.
- **Any organization scheme beyond folders:** no linking, no graph, no tags. Obsidian exists.
- **Anything that writes into Longhand from outside:** the only inputs are what you record and what you type.
