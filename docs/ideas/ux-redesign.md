---
title: UX redesign
status: open
area: interface
added: 2026-09-23
source: "Phase 2b checkpoint feedback on the walking skeleton"
related: [library-and-browsing, live-transcript-view, recognition-model-picker, transcript-review-and-correction]
---

# UX redesign

Captured 2026-09-23 at the Phase 2b checkpoint of the walking-skeleton plan (`docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`), after the user ran three replayed recordings through the signed bundle. It was first kept as `docs/brainstorms/2026-09-23-ux-redesign-input.md`, the path the plan still cites. Nothing here has been brainstormed yet: it is the brief to hand `ce-brainstorm` when the redesign is picked up. The skeleton's current surfaces stay until that brainstorm produces requirements and a plan.

## The verdict

Nothing in the UI is broken (the one broken item, the sidebar dropping sessions, was fixed in 44eb1f3). The experience feels bad and is not the one the user wants. Most of what follows is structure, not polish.

## What the user objects to

- **Settings live in the wrong place.** Settings and Status are sidebar sections inside the main window. A normal app has a Settings window.
- **Sessions in a sidebar list.** Anarlog does it this way; the user does not like that Anarlog does it.
- **The completed-session view squeezes the notes into a side column.**
- **Note-taking.** The timestamped stream came from choosing "stick timestamps on things" during planning, but it was never meant to be a timestamped stream of consciousness. The intent was a real note-taking experience, more like Anarlog's: type markdown, go back, edit, rearrange. A timestamped quick-entry mode could still be useful, as a separate mode.
- **The word "landed".** It must never appear in the user experience. Use plain words: saved, completed, processed. (Applied to current copy in f51e668.)

## Reference: Wispr Flow's notetaker

The user considers its organisation very good. Longhand should not copy it pixel for pixel, but its approach is the model. The seven screenshots shared in the conversation contain real names and meeting content, so they are described here and kept only in the working copy, in the gitignored `docs/ideas/ux-redesign-screenshots/`: 1–3 a note being recorded (Summary, My thoughts and Transcript tabs), 4–6 a completed note (a long turn split into paragraphs, the transcript's end, the summary), 7 home with a hovered note's preview. A fresh clone will not have them.

**Home: past notes.** A main page lists previous notes grouped by day, each row a title and time. Hovering (or selecting) a row shows a side panel with the note's title, date and a short overview from the summary. Flow also shows upcoming meetings from a calendar and attendee names from Zoom or Teams; Longhand has neither. A speaker count would be a useful stand-in for attendees.

**A completed note.** A page with the title, date, and three tabs: *My thoughts* (your notes), *Transcript*, *Summary*. Summary is the default tab.
- Summary: a small header with read time and a copy button, then the markdown summary.
- Transcript: a header with speaker count and duration, search and copy. It opens scrolled to the end. Each turn is a speaker label over a rounded text block; a long turn is broken into several paragraphs, each its own block. Speakers without a name read Speaker 1, Speaker 2.

**A note being recorded.** The same page with the same three tabs, so recording and reading feel like one place. It opens on Transcript by default (a setting, the user thinks), which shows the transcript in progress and the elapsed time, with a banner saying the transcript and speaker labels improve after the meeting. My thoughts is a markdown editor. Summary says it generates when you stop. A Stop button sits at the bottom.

**Not wanted from Flow:** calendar integration, attendee integration, the AI chat box, sharing and links.

## How it might map onto Longhand

A first sketch, for the brainstorm to accept or overturn:

- **Home is the session list,** grouped by day: title, time, duration, speaker count, and a status for anything still processing or needing attention. Selecting one previews its summary overview. The plan currently puts a Library out of scope (Scope Boundaries); this brings a small one back in.
- **One session page for recording and reading,** with tabs for Summary, Transcript and My notes, and copy on each. Summary is the default once the session is done.
- **The transcript view** uses speaker blocks, opens at the end, and splits long turns into timestamped paragraphs. The 2026-09-23 recordings need this in any case: one bad-audio meeting had a single 5½-minute turn, and the summary anchored four key points to its one timestamp.
- **Real notes:** a markdown editor, editable and rearrangeable at any time, with an optional way to stamp the current time on a line (for example a shortcut) instead of every line being timestamped.
- **Settings in a standard macOS Settings window** (⌘,), with the readiness status folded into it or into the menu bar item.
- **A title per session,** perhaps proposed by the summarizer, so the list and the saved folder read as something better than "Longhand session".
- **Choosing the recognition model** in Settings. The skeleton hard-codes Whisper large-v3-turbo, which is not the model the user will use day to day. The code already carries a model id per setting and per session behind a `Recognizer` trait; the plan defers the picker and a second engine.
- **Resume** of any completed session, with no time limit (decided 2026-09-23). The brainstorm decides where it lives, and whether it is tucked into a menu for older sessions.

## Questions for the brainstorm

1. **The Transcript tab while recording.** Already decided in the plan: a live transcript is a capability of the selected model, not a promise of the product (KD8, R10). When the model streams, a live view is offered; when it does not, the surface says the transcript arrives when processing finishes, as Anarlog does. The view itself was deferred to a later plan for simplicity, and the default model (Whisper large-v3-turbo) does not stream. So the redesign lays out the Transcript tab for both cases, and the brainstorm decides whether the live view comes with the redesign or after it.
2. **Which tab a recording opens on,** given the plan's attention budget (U7's two-tier rail): the recording surface must not pull a participant's eyes back to the screen. Probably My notes, even where a live transcript exists: a scrolling transcript is exactly what pulls eyes back.
3. **What happens to the menu-bar and rail surfaces** from U7 once recording lives on the session page.
4. **Notes model:** do the timestamps that navigation from notes into the transcript relies on survive as optional stamps, or does that navigation change?
5. **How much Library:** list and preview only, or also search, rename and delete? The 16-second test sessions are in the list with no way to remove them.
6. **Where the processing view goes:** a state of the session page, or its own screen as now.

## Constraints that carry over from the plan

- Local-first; nothing leaves the Mac except the summary request to the endpoint the user configured.
- The attention budget wins over added motion or ornament (CLAUDE.md, U7).
- The session folder in the user's chosen destination stays the durable record; the app reads from it.
- UI work follows the `apple-design` skill.
