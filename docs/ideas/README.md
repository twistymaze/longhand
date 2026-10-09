# Feature ideas

Feature ideas for Longhand, one file for each idea. The folder of an idea is its status. Only an idea in `next/` is a promise to do the work. Keep all idea files in git, in all folders.

| Folder | Meaning |
|---|---|
| `next/` | Confirmed. We will do it when there is time. An idea in `next/` with a `plan:` field is in progress. |
| `someday/` | Possibly. Not confirmed as a priority. |
| `parked/` | Considered, and not done now on purpose. The file has a `## Why parked` section. |
| `done/` | Shipped. The file stays, because it records where the work came from. |

## See the ideas

Do not keep an index by hand. The script reads only the frontmatter:

```sh
bun scripts/ideas.ts                      # In progress, Next, Someday and Parked; a count of Done
bun scripts/ideas.ts someday parked       # only these folders
bun scripts/ideas.ts --area capture       # only one area
bun scripts/ideas.ts done                 # the shipped ideas
```

To learn about one idea, read its `description:` first. Read the full file only when you need it.

## Keep the ideas up to date

Add ideas as they come up in any session, not only in planning.

- **A new idea** gets its own file, `docs/ideas/someday/<kebab-case-slug>.md`, in the format below. Put it in `next/` only when the user confirms it as a priority. First, make sure that the idea is new. `bun scripts/ideas.ts` lists only the open ideas, so also search `docs/ideas/done/`, `docs/plans/` and `docs/solutions/` for its key words. Work that shipped from a plan with no idea file is only in those folders. If the idea extends an existing one, add a note to that file. If shipped work already solves it, tell the user and do not file it.
- **One idea for each file.** A file holds one well-formed idea, or a small set of closely related ideas in one area. It is not a list of small unrelated tasks, such as the residual findings of a code review. Those findings go to the PR, to a plan, or to a separate idea file each.
- **An idea only mentioned in passing** in a plan, brainstorm, review or conversation also gets a file here, so that the list stays complete.
- **To change the status,** move the file with `bun scripts/ideas.ts move <slug> <folder>`. The script runs `git mv` and changes every link to the file in `docs/` and the root documents. Do not move an idea file by hand.
- **When an idea is picked up,** move it to `next/` if it is not there, and add `plan: docs/plans/<plan>.md`. A plan that starts with no idea file, such as a fix from a conversation, also gets an idea file in `next/` with its `plan:` field, so that the work is in `done/` after it ships.
- **When it ships,** move it to `done/` in the same session that merges the PR, and write the PR in its file. This is a step in [After a merge](../development-workflow.md#after-a-merge). `bun scripts/ideas.ts check` finds an idea that is not in `done/` after the branch of its plan merged.
- **When the user or shipped work makes an idea unnecessary,** move it to `done/` with a short `## Outcome` section that tells why, or park it with its reason.
- **When you park an idea,** write why in a `## Why parked` section.
- **An idea that conflicts with a boundary** in `STRATEGY.md` goes to Ruled out below with its reason. It gets no file, so that nobody works out the reasoning again.
- **Before a commit,** run `bun scripts/ideas.ts check`. It finds missing frontmatter fields, unknown areas, parked ideas with no reason, `related:` slugs that do not exist, broken links to idea files, and open ideas whose plan branch merged.

Each idea file has this frontmatter and a short body:

```markdown
---
title: A plain-language name
description: "One line: what the idea is, for the list"
area: capture           # capture | transcript-and-summary | processing | library-and-sharing | interface | distribution
added: 2026-09-24
source: "where it came up (a plan section, a brainstorm, a conversation)"
related: [other-idea-slug]
plan: docs/plans/<plan>.md   # only when the idea is in progress or done
---

# A plain-language name

## Idea
What it is, in a few sentences.

## Why
The problem it solves, or who asked. (A parked idea also has `## Why parked`.)

## Notes / Current state
Open questions, dependencies, what already exists.
```

Private material for an idea, such as screenshots with real names, goes in a gitignored `<slug>-screenshots/` folder beside the idea file (see `done/ux-redesign.md`). Never commit it.

### How this relates to the compound-engineering folders

- `docs/ideation/` is where `ce-ideate` writes each ranked ideation session. `ce-ideate` does not read this folder, and it does not file its survivors here. For each run:
  - **Before the run,** run `bun scripts/ideas.ts someday next parked done` and give the output to `ce-ideate` as context. Tell it that these ideas are known, and that it must not suggest them again unless it adds something new. Tell it that a parked idea stays parked for the reason in its file.
  - **After the run,** give each survivor that is not brainstormed its own file in `someday/`, with `source:` set to the path of the ideation file. If a survivor extends an existing idea, add a note to that file.
- `docs/plans/` is where `ce-brainstorm` writes the requirements it produces, as a requirements-only plan, and where `ce-plan` adds the implementation plan. Picking an idea up usually means brainstorming it, which creates that plan; link it from the idea's `plan:` field.
- `docs/brainstorms/` is a legacy location that current compound-engineering skills only read old `*-requirements` files from. Don't put idea briefs there; an idea's brief lives in its file here.
- `docs/solutions/` holds learnings from solved problems.

This folder is our own convention, not something compound engineering defines, so it has to keep fitting the plugin as the plugin changes. For example, a future version might give ideas a location of its own, move `ideation/` or where brainstorm output goes, or have a skill read or write backlog items. Whenever something like that surfaces, or the two start to disagree, raise it and resolve it: move this folder to the plugin's convention, or change how it links to the plugin's folders. Don't work around it quietly.

## Ruled out

These conflict with a boundary in `STRATEGY.md`.

- **Speaker identity:** enrollment, voiceprints, names attached to speakers. Turn boundaries are the requirement; generic labels that drift across a long session are acceptable.
- **Calendar integration,** including auto-start and prefill. Revisit only if a genuinely simple path appears inside an enterprise.
- **Any organization beyond note types and tags:** no linking, no graph, no tag pages or other views across notes. Obsidian exists.
- **Anything that writes into Longhand from outside:** the only inputs are what you record (in Longhand or on another device) and what you type. An edit that you ask an agent to make counts as what you type; an integration that pulls in outside context does not.
