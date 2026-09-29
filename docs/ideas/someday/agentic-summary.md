---
title: A summary agent with tools
description: "The summary model can read earlier notes, search the library, read parts of the transcript, and read context about the user"
area: transcript-and-summary
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28; includes the earlier idea 'carry context from the previous session'"
related: [summary-prompt-by-note-type, note-folders, library-and-browsing]
---

# A summary agent with tools

## Idea
The summary model works as an agent. It can call tools while it writes the summary, in place of one call with the full transcript:

- **Related earlier notes:** find and read earlier notes of the same series, folder or topic. For a Pathfinder campaign this gives "previously on", and names and threads that stay the same from one session to the next. (This was the separate idea "carry context from the previous session".)
- **Search the library:** find other notes that mention a name or a topic in this note.
- **Read parts of this note:** read ranges of the transcript and the notes when it needs them, so that a long note does not have to fit in one prompt.
- **Context about the user:** text that the user writes in Settings, such as their name, their role and the names and terms that often occur. Possibly the agent keeps it up to date, like a memory; that is an open question.

## Why
- **Transcripts with errors.** The most important goal. The transcript often spells names and terms wrong. Earlier notes, the library and the user's context let the agent see what a wrong word probably means.
- **Continuity:** names, threads and open items from earlier notes.
- **Long notes:** reading on demand can give a better summary than the current method, which splits the transcript and merges the parts.

## Notes / Current state
- Today Summarize makes one chat-completion call to the chosen endpoint. When the transcript is too long, it splits the transcript into chunks and merges them hierarchically (KTD16 in the walking skeleton plan). The model has no tools.
- Every tool reads only local data: this note, other notes, and text that the user typed. That stays inside the `STRATEGY.md` boundary "Nothing writes into Longhand". The tools send more of the user's notes to the endpoint, so the processing page must say so, as it does for the transcript today.
- Not every OpenAI-compatible server supports tool calls. Keep the current method as the fallback.
- The fixed rules of the summary prompts (no "Speaker N", no guessed names, the transcript as quoted data) still apply.
- Out of scope: tools that reach outside the machine, such as a calendar, and a general plug-in system for tools (MCP or other). The user wants to keep those as a separate idea. They touch the Ruled out items "Calendar integration" and "Anything that writes into Longhand from outside".
