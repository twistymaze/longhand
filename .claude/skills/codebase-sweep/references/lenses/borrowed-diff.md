# Lens: borrowed-diff

The shard holds files that the project copied from another project and then changed. The upstream code is not the subject of this sweep. Our changes, and the places where our changes meet the upstream code, are the subject.

## Method

1. For each file, the shard gives the upstream clone, the revision, and the source path. Get the upstream version with the command that the profile gives (usually `git -C <clone> show <revision>:<source path>`).
2. Compare it with our file. Examine only:
   - lines that we added or changed;
   - upstream lines that our changes now call in a new way, with new inputs, or on a new thread;
   - upstream assumptions that our changes break, for example a function that upstream calls one time and we call in a loop.
3. Apply the questions of the `correctness`, `concurrency`, `data-loss`, and `error-recovery` lenses to those parts.

## Do not report

- Defects in upstream lines that our changes do not touch and do not use in a new way. When such a defect is serious, mention it in `coverage.not_covered` as a note for the `usage` lens.
- Differences in style between our code and upstream code.

When the upstream version is not available, say so in `coverage.not_covered` and examine the full file with the lenses above.
