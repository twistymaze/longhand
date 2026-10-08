---
title: Safe note edits by agents
description: "Longhand puts a skill for agents in the storage folder and makes its file checks tolerate safe outside edits, so an agent can add tags or change types on many notes"
area: library-and-sharing
added: 2026-10-08
source: "conversation on 2026-10-08 about an AGENTS.md file in the storage folder and agent tag edits"
related: [suggested-tags, library-and-browsing, agentic-summary]
---

# Safe note edits by agents

## Idea

The user can ask an agent (Claude Code, Codex) to do work on many notes in the storage folder. For example, the agent finds old notes about a person and adds the `Person::` tag to them. Two parts make this safe:

1. **A skill in the storage folder.** Longhand writes `.agents/skills/longhand-notes/SKILL.md` into the storage folder. `.claude/skills` links to the same skill. The skill tells the agent the layout of the folder, which data has a fixed structure and which data is the user's own (types, tags, titles), and how to find and read notes. It also gives the steps for each edit that is permitted.
2. **File checks that tolerate safe edits.** Longhand's checks accept the edits that the skill permits. An edit that is not safe gives a clear problem and no silent damage.

A CLI is a possible third part, or a replacement for the edit steps in the skill (see Notes).

## Why

The storage folder is plain files so that agents can do complicated work outside the app (`STRATEGY.md`). Reads are safe today. An agent can already add tags in `session.md`, and Longhand reads them. But some edits that look safe cause problems. The checks below are from the code after PR #45:

- **The `session.md` fingerprint includes the body and every field except `longhand_tags` and `longhand_pending_type`** (`session_fingerprint`, `crates/session-core/src/folder.rs`). An agent that changes another field, or the text under the frontmatter, changes the fingerprint. Transcribe Again, Regenerate Summary and Resume then can refuse to run on that note.
- **A frontmatter that does not parse** makes the note a problem folder until a person corrects the file.
- **Two tags with one tag key** (`tag_key`: no difference of case and spaces) can be in the file after an outside edit. Longhand does not make this state itself. It is not known if a read merges the two tags.
- **An outside write at the same time as a Longhand write on another computer** loses one of the two writes. The sync service keeps the last file.
- **`transcript.md` or `summary.md` with a changed fingerprint** makes the note wait for about one minute before Longhand accepts the edit (`CONCEPTS.md`, "Waiting for sync").

## Notes / Current state

**Possible changes to the checks:**

- Make the `session.md` fingerprint cover only the fields that Longhand owns, as parsed values. Longhand makes the body from those fields, so the body does not need to count. A YAML layout change or a user field then never fails a run.
- Make a read merge two tags with one tag key, as the app does for its own edits.
- Show a clear notice for a `session.md` that does not parse. The notice names the file and the line.
- Give the skill a check that it runs before it writes. The check finds a fresh `.longhand-in-use-*` marker or `result: pending`, and the agent then skips the note.

**Skill or CLI:**

- A skill alone depends on the agent following the steps. It works with no install and on every computer that shares the folder.
- A CLI (for example, subcommands of the app binary) can use the same Rust code as the app: tag keys, the store lock, in-use markers, the read-modify-write of `session.md` and type moves. It can refuse an edit that is not safe. The agent needs Longhand installed on that computer.
- A mixed approach: the skill uses the CLI when the CLI is available, and the file steps when it is not.

**Open questions:**

- Which edits does the skill permit? Possible list: tags, type (a move between type folders), title, and `notes.md`. Transcript and summary edits are probably out of scope.
- Where does a storage folder keep its version of the skill, and how does Longhand update it without removing the user's own changes? One possible answer: Longhand owns its skill folder and replaces it when its version changes. The user's own skills go in other folders.
- Some sync services do not keep symbolic links (OneDrive, Google Drive). Resilio Sync was not tested. A second copy of the skill, or a small `CLAUDE.md` with `@AGENTS.md`, can replace the link.
- Does Longhand also write an `AGENTS.md` for read-only questions? The user has one by hand in their storage folder now.

**Strategy check:** done on 2026-10-08. The user agreed that an agent edit that the user asks for counts as what the user types. The boundary "Nothing writes into Longhand" in `STRATEGY.md` is about outside context that an integration pulls in. `STRATEGY.md` now says this.
