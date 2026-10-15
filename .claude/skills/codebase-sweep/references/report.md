# Report of a sweep

You make two outputs: the text of `report.md` for a person, and the file `findings.json` in the run folder for later steps. Write only `findings.json`. Do not write `report.md`: the harness does not let a subagent write a report file. Return its full text in `report_markdown`; the skill writes the file.

## Inputs

- `summary`: counts for the run.
- `coverage`: one row per finder task.
- `records`: one per verified finding. Each has the `finding`, the `votes`, the `outcome` (`confirmed`, `disputed`, or `rejected`), and the final `severity`.

## Numbers

Sort the confirmed findings by severity, then by file. Give them ids `C1`, `C2`, and so on. Sort the disputed findings the same way and give them ids `D1`, `D2`, and so on. Rejected findings get no id.

## report.md

Use this structure. Use short, plain sentences.

```markdown
# Codebase sweep <date of the run folder>

<one paragraph: shards, finder tasks, findings verified, confirmed, disputed, rejected; and whether this was a pilot (summary.only)>

## Confirmed

### C1 · P0 · <title>
`<file>:<line>` · lens <lens> · votes <n confirmed>/<n votes> (<voters>) · also found by <tasks, or "none">

**Trigger.** <trigger>
**Failure.** <failure>
**Evidence.** <the most important evidence lines from the confirming votes>
**Suggested fix.** <when present>
**Proof.** Not reproduced. <!-- the reproduce step is not part of this version -->
**Same cause as.** <ids of other findings with the same root cause or the same fix, or omit the line>  <!-- a note only; each finding keeps its own id and outcome -->

## Disputed

<the same layout, plus one line per vote: voter, status, reason>

## Rejected

<a table: title, file:line, lens, main reason>

## Rejection patterns

<group the rejected findings by the reason class, for example "quote not found", "guard in caller", "no real path", "intended (docs/solutions/...)". Give the count of each class and the lenses that produced them. These patterns show where a lens or the finder rules need a change.>

## Coverage

<finder tasks per round; failed finders; a list of what finders said they did not cover; votes from Codex that did not run>

## Lesson candidates

<lessons that could be worth recording, in two groups:
1. Changes to the sweep: lenses, rules, profile, thresholds.
2. Project lessons for docs/solutions/: a defect class that several findings share, or a non-obvious fact about the code that the verifiers established.
Propose only. The person chooses what to record.>
```

## findings.json

A JSON object with `summary` and a list `findings`. Each item has the id (or `null` for rejected), `outcome`, `severity`, the finding fields, and the votes. Copy the values from `records`; do not change them.

## Rules

- Do not change an outcome or a severity. Report what the votes decided.
- Do not add findings.
- Use the run folder date from the path of the run folder.
