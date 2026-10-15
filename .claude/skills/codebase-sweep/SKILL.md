---
name: codebase-sweep
description: "Sweep a whole codebase for defects with a dynamic workflow: lens finders on each shard, independent votes that try to refute each finding, a coverage critic, and a report. Use only when the user asks for a codebase-wide bug sweep or audit. Not for reviewing a diff or a PR (use ce-code-review)."
argument-hint: "[pilot <shard ids>] [plan]"
disable-model-invocation: true
---

# Codebase sweep

Find real defects across a whole codebase. Many finders examine the code in parallel, each with one lens on one shard. Independent verifiers try to refute each finding. Only findings that a majority of votes confirms reach the confirmed list. The orchestration runs as the saved workflow `codebase-sweep` (`.claude/workflows/codebase-sweep.js`). This skill does the steps that need judgment: it builds the shard map, starts the workflow, and helps the user act on the report.

**Done when:** the report is in the run folder, the user has seen the confirmed findings, and the user has chosen what to do with them and with the lesson candidates.

## Requirements

- The Workflow tool is available, and the user asked for this sweep. Invoking this skill is that request.
- The session's workflow size setting allows the number of agents. A full sweep can use several hundred agents. When the setting is "medium" or smaller, tell the user to raise **Dynamic workflow size** in `/config` and to start a new session, or run a pilot.

## Steps

### 1. Read the profile

Read the project profile at `.claude/codebase-sweep-profile.md` when it exists. It gives the inventory commands, the files to skip, the rules for borrowed code and dependencies, the risk areas, the candidate seams, the decision sources, and the threat model. Without a profile, find these facts yourself and tell the user that the shard map will be less precise.

### 2. Make the run folder

Make `.context/sweeps/<YYYY-MM-DD-HHMM>/` in the repository root. Run `git check-ignore -q <run folder>/x`. When git does not ignore it, stop and tell the user: the run output must never be committed.

### 3. Take the inventory

Run the profile's inventory commands. Divide the tracked source files into app code, test code, borrowed code (copied or changed), generated files, and files to skip. Count the lines of each group. Use subagents (for example `Explore`) to read the intent of large areas from the project docs, so that this context stays small.

### 4. Build the shard map

Read `references/shard-map.md` and follow it. Write the map to `<run folder>/shard-map.json`. Include the profile's candidate seams that still match the code, and add seams that you find.

### 5. Confirm with the user

Run the workflow with `mode: "plan"` to get the finder count. Then show the user, in a short table, the shards with their categories, sizes and lenses, the seams, the finder count, and an estimate of the total agent count with the method in `references/shard-map.md`. Ask whether to run a pilot or the full sweep. For a pilot, recommend two or three shards with different risks and one seam, and set `only`.

When the user gave `plan` as an argument, stop after this step.

### 6. Run the workflow

Start the Workflow tool with `name: "codebase-sweep"` and the shard map as `args` (a JSON object, not a string). When the tool says that the name is not found (a session loads saved workflows when it starts), use `scriptPath` with the absolute path of `.claude/workflows/codebase-sweep.js` instead. It runs in the background. Do not poll it. When the user asks about progress, tell them to use `/workflows`.

When the run stops early, resume it with `resumeFromRunId` and the script path that the tool result gave. The agent calls that did not change return their cached results.

### 7. Show the result

The harness does not let a subagent write a report file, so the report agent returns the text in `report_markdown`, and the workflow result gives only its length. Write it to `<run folder>/report.md` with a script that reads the last `report_markdown` from the run's `journal.jsonl` (the path is in the tool result), so the text does not go through this context. Then read the summary sections of the report. Tell the user the counts and each confirmed finding in one line (id, severity, title, `file:line`). Give the path of the report. Do not paste the full report.

### 8. Act on the findings

Ask the user what to do with the confirmed findings, for example: fix now, record for later, or ignore. Fixes follow the project's normal rules for changes. A disputed finding needs more evidence before a fix; offer to examine it with a debugging skill.

### 9. Lessons

The report has lesson candidates in two groups. Ask the user which ones to record:

- **Changes to the sweep.** Edit the lenses, the rules, the profile, or the defaults. Use the rejection patterns: a lens that produced many rejections of one class needs a change.
- **Project lessons.** Record each chosen lesson in the project's solutions folder, in the way that `ce-compound` does. Do not record unfixed defects in a tracked file.

## Rules

- The run folder is never committed. Only lessons that the user chooses leave it.
- Finders and verifiers are read-only. Do not change this in a prompt.
- Do not change an outcome that the votes decided. When you disagree, say so to the user next to the outcome.
- Tell the user about any coverage that the run did not get: failed finders, votes from Codex that did not run, and what the critic did not cover.
