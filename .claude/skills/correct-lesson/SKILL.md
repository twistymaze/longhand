---
name: correct-lesson
description: Turn a mistake into a check, with pstack's /correct method scoped to that mistake. Use after ce-debug fixes a defect, after ce-compound writes a lesson, and after you accept a review finding or a correction from the user, as CLAUDE.md "Lessons become checks" requires. Pass the mistake and the lesson path.
---

# Correct a lesson

This skill lets an agent run pstack's `/correct` method. The pstack skill has `disable-model-invocation: true`, so only the user can start it by name. The user wants this step to run each time without a manual command, so this project skill loads the same instructions.

## 1. Load pstack's /correct

Find the installed copy and read the whole file before you continue:

```bash
ls -t ~/.claude/plugins/cache/*/pstack/*/skills/correct/SKILL.md | head -n 1
```

Follow its method ("Fix each class at the highest level that works", "Fix and prove", "Keep the rule table") with these limits:

- **Scope.** Work only on the mistake that the caller passed. Do not run a repo-wide sweep of commits and reviews. "A class counts once it has happened twice": search `docs/solutions/` and `rules/` for an earlier case of the same class.
- **Rule table.** In this repo the rule table is the `rules/` ledger (`rules/README.md`), not a table in the instruction file.

If no installed copy exists, use the order below without it, and tell the user that pstack is not installed.

## 2. Apply it in this repo

1. Pick the highest level that works: a structure or type that makes the mistake impossible, then a lint whose message names the fix, then a test, then a runtime check, then a gate (CI step or hook). Use prose only for a real judgment.
2. Prove the check: it passes on the current tree, and it fails on the mistake. Make the mistake in the working tree, save it with `git diff > rules/negatives/<name>.patch` (`git add -N` first for a new file), then restore the tree.
3. Add or update the family in `rules/<family>.toml` with `enforced_by`, the negative example, and the lesson path in `lessons`. A first-time mistake with no low-cost check needs no family: add `check: none: <reason>` to the lesson's front matter instead.
4. Commit, then run `node scripts/check-rules.ts --family <family>` and `node scripts/check-rules.ts --validate`. The validate step fails on a lesson in `docs/solutions/` that names no check.

## Reply

For each mistake class: the evidence, the level you picked, why a higher level did not work, and the family or `check: none` reason.
