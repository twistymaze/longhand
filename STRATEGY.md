---
name: Longhand
last_updated: 2026-09-21
---

# Longhand Strategy

## Purpose

Sometimes you're in a meeting or a group situation where you want to engage in the conversation and not stay super focused on taking manual notes, but you need notes after the fact to recall the specifics and not rely on feeble human memory. The conversations most worth capturing are the ones you won't hand to a third party whose cloud holds the database, and the local alternative that exists is getting complicated and is built around calendar-driven virtual meetings.

## Positioning

The transcript is the system of record, so the notes can stay lazy: a scratchpad with basic markdown editor tooling, combined with a solid local transcript that supplies the details you didn't type. The store stays on your machine — inference is a swappable endpoint you point at something you trust, and local ASR is the default because local models are good enough and cloud ASR isn't worth the bandwidth or cost, not because audio may never leave. Complexity is allowed; it lives outside the app, reached through your vault or the plain files in the storage folder rather than built into the UI. The one exception is a small amount of organization: note types and tags, kept in those same plain files.

## Users

**Primary:** An engineering manager, also a game master running Pathfinder on Friday nights. They're hiring Longhand to recall what was discussed and agreed to days later without having been the one taking minutes, across sessions from 30 minutes to four hours, in a room or on a call.

**Secondary:** Peer managers with the same meeting load and the same confidentiality constraint. The repository is public to follow, but it does not take outside contributions.

## Boundaries

- **No provider is forced on you.** The system of record — audio, transcripts, and notes — is held on your machine, and no feature depends on a third party holding it. Which providers see your data is your choice, made per stage and visible in the app: local by default, pointable at an endpoint you trust. A cloud recognition or summarization option is consistent with this boundary; a default that quietly ships your data somewhere is not.
- **No calendar integration.** Not auto-start, not prefill. Calendar access inside an enterprise is complicated; revisit only if a genuinely simple path appears.
- **No speaker identity.** Diarization marks that the speaker changed so an LLM can follow the turn-taking. No voiceprints, no enrollment, no names.
- **Organization stays small.** A note has one note type, which is also its folder on disk, and any number of tags. The app can apply, create, show, rename, delete and filter them, and nothing more. No inter-note linking, no graph, no tag pages or other views across notes — Obsidian already exists and Longhand hands off to it.
- **Nothing writes into Longhand.** The only inputs are what you record and what you type. A recording you made on another device counts as what you record. An edit that you ask an agent to make to your notes, such as tags on old notes, counts as what you type. The plain files in the storage folder move data out to your agents and notes, for example through a storage folder inside an Obsidian vault; no integration pulls outside context in.
- **No distribution machinery until someone asks.** Cross-platform correctness from day one; installers and update paths only when a real person wants one.
- **No time limits.** A four-hour session is a supported case, not an edge case.

_Resist a change when:_ it makes Longhand the place your knowledge lives, or makes something outside your machine unavoidable. Expose the data and let something else do it.

## Key metrics

- **Still in use at one month** — you haven't drifted back to the tools that annoy you. Observational; you'd know.
- **Coverage** — notes exist from most meetings you attend, and Friday Pathfinder sessions are captured end to end. Visible in the folders.
- **Time from "record this" to recording** — keyboard-shortcut fast, not 25 seconds of spin-up. You'd feel it the first time it's slow.
- **Shareable without apology** — you'd hand it to a colleague without embarrassment. Binary.

## Tracks

### Platform

The cross-platform desktop shell that makes Longhand runnable at all — macOS first, Windows without a rewrite.

_Why it serves the approach:_ Nothing ships until this does, and the reference projects already solve it, so it should stay plumbing rather than become a project of its own.

### Capture

Mic and system audio, local ASR, VAD, and diarization that run for hours without dropping anything, in a room or on a call.

_Why it serves the approach:_ The transcript is the system of record; if it's unreliable or time-capped, the lazy-notes bet collapses.

### Session surface

What you touch while half-listening: start recording in a keystroke, a markdown scratchpad, the live transcript, and the summary that combines them.

_Why it serves the approach:_ Engaging in the conversation is the whole point, so anything that pulls attention back to the app defeats it.

### Library and handoff

Finding the note from three weeks ago, plus getting data out — plain files in the storage folder that an agent or an Obsidian vault can read directly.

_Why it serves the approach:_ This is the escape hatch that lets the app stay simple: complicated things happen outside, on your data. Note types and tags help you find a group of notes in the app and on disk; anything beyond that happens in your vault.
