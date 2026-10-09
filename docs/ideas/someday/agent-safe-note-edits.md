---
title: Safe edits by agents
description: "Longhand puts a skill for agents in the storage folder and tolerates the outside edits that the skill permits (tags, types, summary prompts), so the user can ask an agent to tag notes or design a prompt"
area: library-and-sharing
added: 2026-10-08
source: "conversations on 2026-10-08 and 2026-10-09 about an AGENTS.md file and a hand-made skill in the storage folder"
related: [suggested-tags, library-and-browsing, agentic-summary, summary-prompt-by-note-type, regenerate-with-another-prompt]
---

# Safe edits by agents

## Idea

The user can ask an agent (Claude Code, Codex) to read and change data in the storage folder. Examples: the agent finds old notes about a person and adds the `Person::` tag to them, or it helps the user design a better summary prompt. Three parts make this safe:

1. **A skill in the storage folder.** Longhand writes `.agents/skills/longhand-notes/` into the storage folder, and `.claude/skills` links to it. The skill tells the agent the layout of the folder, which data has a fixed structure and which data is the user's own (types, tags, titles, prompts), how to find and read notes, and the steps for each permitted edit.
2. **Longhand tolerates the permitted edits.** Longhand's checks and scans accept the edits that the skill permits. Where an agent is likely to make a mistake, Longhand repairs the result or shows a clear problem, and never loses data silently.
3. **Maybe a CLI** for the edits that need Longhand's own rules (see Notes).

## Why

The storage folder is plain files so that agents can do complicated work outside the app (`STRATEGY.md`). Reads are safe today. Some edits that look safe cause problems.

Summary prompts are a special case. They are LLM prompts, and an LLM is good at helping to design an LLM prompt. The user wants an agent to read sample transcripts and summaries, propose a prompt, and change it after the user's feedback. Longhand does not need to support every prompt operation through an agent. It must support the operations that this design work needs, and make the others hard to get wrong.

## Current state

On 2026-10-09 the user put a hand-made version of the skill in their own storage folder, to test it outside the normal development process. When it works well, this idea packages it in the app. The hand-made skill has:

- `SKILL.md` with the rules and a table that tells the agent which reference file to read for which task.
- Reference files: `layout.md`, `files.md`, `answering.md`, `editing.md`, `tags.md`, `types.md`, `prompts.md`.
- Two scripts, standard library only, for `python3` or `uv run`. `list_notes.py` lists notes with filters. `lint_notes.py` finds problems, and it compares the folder with a snapshot to prove that an agent changed only what it meant to change. It does not write `__pycache__` into the synced folder.

The skill permits, at the user's request: tag edits in `session.md`, type changes (one `mv` of the note folder), and edits to the body of prompt files or new prompt files. It sends prompt renames, prompt deletes, the active prompt and everything else to the Longhand UI.

## Problems found, and changes to Longhand

### Notes

The checks below are from the code after PR #45.

- **The `session.md` fingerprint includes the body and every field except `longhand_tags` and `longhand_pending_type`** (`session_fingerprint`, `crates/session-core/src/folder.rs`). A change to another field or to the body can make Transcribe Again, Regenerate Summary and Resume refuse to run.
  - *Change:* make the fingerprint cover only the fields that Longhand owns, as parsed values. Longhand makes the body from those fields, so the body does not need to count.
- **A frontmatter that does not parse** makes the note a problem folder until a person corrects the file.
  - *Change:* show a notice that names the file and the line.
- **Two tags with one tag key** can be in the file after an outside edit. Longhand does not make this state itself. It is not known if a read merges them.
  - *Change:* a read merges them, as the app does for its own edits.
- **An outside write at the same time as a Longhand write on another computer** loses one write. The sync service keeps the last file.
  - *Change:* none in Longhand. The skill skips busy notes (fresh in-use marker, `result: pending`).
- **`transcript.md` or `summary.md` with a changed fingerprint** makes the note wait for about one minute before Longhand accepts the edit (`CONCEPTS.md`, "Waiting for sync"). Transcript and summary edits are out of scope for the skill.

### Summary prompts

From `crates/shared-settings/src/prompts.rs`:

- **Copy and edit.** An agent that copies a prompt file to make a new prompt keeps the `id`. The scan treats two files with one id as one prompt and a conflict copy. When the user next edits that prompt in Longhand, Longhand can delete a copy that has the same text.
  - *Change:* treat a copy with one id as a new prompt when the user clearly made a new prompt, for example when its `name` differs and its file name matches that name. Write a new id into it at the next Longhand write, so every computer agrees.
- **A file with no id.** The scan makes an id from the file name, so a rename of the file changes the id. A setting that names the old id then points to nothing.
  - *Change:* write an id into a file that has none at the next Longhand write.
- **Rename.** The `id` keeps a prompt's identity, so a change of `name` in the file can work today. The file name then does not match the name, and the scan treats the file as a weaker copy.
  - *Change:* support a rename by an edit of `name`. Longhand renames the file to match at its next write and keeps the id.
- **Delete.** An outside delete of a prompt file leaves the active prompt, and the prompt of each note type (the map in the summary prompt by note type plan, KTD1 and KTD6), pointing to a missing id. Summaries then fall back to the default prompt, and the user is not told.
  - *Change:* when a setting names a prompt that is gone from every computer's folder, Longhand shows it in Settings and clears the setting with the same rules as an in-app delete. Another possible change: a deleted prompt goes to a `.longhand/prompts/.deleted/` folder, so the delete can be undone.
- **Limits.** A prompt text over 4,000 characters or title instructions over 1,000 characters can come from an outside edit. It is not known what Longhand does with such a file.
  - *Change:* show the file as over the limit in Settings, and do not cut it silently.
- **Test a draft prompt.** An agent cannot run Longhand's summary step. To test a draft prompt, it can only imitate the step with its own model, and that misses the fixed rules, the Background, the user's endpoint and the model.
  - *Change:* a way to run Summarize with a draft prompt on one note and get the result without saving it: a CLI command, or the "Regenerate With" idea with a preview. This is the most useful change for prompt design.

## Notes

**Skill or CLI:**

- A skill alone depends on the agent following the steps. It works with no install and on every computer that shares the folder. The lint script catches most mistakes after the fact.
- A CLI (for example, subcommands of the app binary) can use Longhand's own code: tag keys, the store lock, in-use markers, the read-modify-write of `session.md`, type moves, prompt ids and Summarize. It can refuse an edit that is not safe. The agent needs Longhand installed on that computer.
- A mixed approach: the skill uses the CLI when it is available, and the file steps when it is not. A dry-run Summarize needs the CLI or the app.

**Open questions:**

- Which other edits does the skill permit later? Possible: title, `notes.md`, `background.md`.
- How does Longhand update its skill folder without removing the user's own changes? One possible answer: Longhand owns `longhand-notes/` and replaces it when its version changes, and the user's own skills go in other folders.
- Some sync services do not keep symbolic links (OneDrive, Google Drive). Resilio Sync is not tested yet. A second copy of the skill can replace the link.
- Does Longhand also write an `AGENTS.md` that points to the skill? The user's hand-made folder has one, because some agents do not load skills outside a git repository.

**Strategy check:** done on 2026-10-08. The user agreed that an agent edit that the user asks for counts as what the user types. The boundary "Nothing writes into Longhand" in `STRATEGY.md` is about outside context that an integration pulls in. `STRATEGY.md` now says this.
