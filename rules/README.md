# Rule ledger

Each file in this folder records one mistake family: a class of mistake that agents or people made before, and the check that now stops it. `node scripts/check-rules.ts` reads these files, and CI runs it on each pull request and each push to `main`.

## Fields

A family file is `rules/<family>.toml`.

| Field | Meaning |
|---|---|
| `family` | The family id. It is the same as the file name without `.toml`. |
| `rule` | One sentence: what to do. |
| `state` | `enforced`: a check holds the rule. `judgment`: no check can hold the rule, and the rule stays as text. |
| `enforced_by` | For `enforced`: the command that holds the rule. |
| `lessons` | Optional. Paths of the lesson documents that give the reason. |
| `judgment` | For `judgment`: one sentence on why no check can hold the rule. |
| `[[negative]]` | For `enforced`: one table for each known form of the mistake. |

A negative example has three fields:

| Field | Meaning |
|---|---|
| `apply` | A patch file in `rules/negatives/` that puts the mistake back. |
| `run` | The command that must fail when the patch is applied. |
| `expect` | Text that the failing output must contain. |

A patch adds a new file where the rule allows it, for example a new component or a new test file. It edits an existing file only when the rule is about that file. Then normal changes to busy files do not break the patch. To make a patch, make the mistake in a clean tree and run `git diff > rules/negatives/<name>.patch` (with `git add -N` first for a new file).

## How the check runs

The script applies each patch to the working tree with `git apply` (no 3-way merge, so the result is the same in a full clone and in the CI checkout with no history), runs the command, and restores the files of the patch. It reads the committed tree, so it refuses to run when the working tree has changes. Commit first, then run it. A command that changes other files fails the run, and the script keeps those changes. A command that runs longer than 30 minutes is stopped. On Ctrl+C, the script restores the tree and stops.

- `node scripts/check-rules.ts` checks the ledger and runs every negative example.
- `node scripts/check-rules.ts --validate` checks the ledger only.
- `node scripts/check-rules.ts --family <id>` runs one family.

A failure says one of these things:

- The family file is not valid. The message names the file and the field.
- The patch does not apply. Regenerate the negative example against the current code.
- The command passed with the patch applied. The check no longer catches its mistake: fix the check, not the negative example.
- The command failed, but without the expected text. The check fails for a different reason.

## Exception lists

A scan or a lint holds its own list of known exceptions, each with a reason. The list can only get shorter: a scan fails on a new hit with no entry, and on an entry that no longer matches. In Rust, an exception is `#[expect(clippy::disallowed_methods, reason = "...")]`, which fails when it is no longer needed.

## Lessons name their check

A lesson in `docs/solutions/` gives no signal when an agent does not read it. So `--validate` fails on a lesson that names no check. A lesson names its check in one of two ways:

- A family lists the lesson in `lessons`.
- The lesson's front matter has `check: none: <reason>`, for a first-time mistake with no low-cost check.

`rules/lessons-without-check.txt` lists the lessons from before this rule. The list can only get shorter: the script fails on an entry that names its check now, or that no longer exists. The `correct-lesson` skill gives the steps that turn a lesson into a check.
