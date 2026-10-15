# Shard map

The shard map is the `args` value of the `codebase-sweep` workflow. You build it before the workflow starts. The workflow cannot read files, so everything that it needs must be in `args`.

## Arguments

```json
{
  "mode": "run",
  "runDir": "/abs/path/.context/sweeps/2026-10-16-0930",
  "repoRoot": "/abs/path",
  "skillDir": "/abs/path/.claude/skills/codebase-sweep",
  "profile": "/abs/path/.claude/codebase-sweep-profile.md",
  "lenses": [{ "id": "correctness" }, { "id": "test-fidelity", "model": "sonnet" }],
  "shards": [
    {
      "id": "session-save",
      "category": "own",
      "intent": "Moves a processed session from the working location into the storage folder ...",
      "files": ["crates/session-save/src/lib.rs", "crates/session-save/src/move.rs"],
      "notes": "Tests are in the same files after #[cfg(test)].",
      "lenses": ["correctness", "data-loss", "error-recovery"]
    }
  ],
  "only": ["session-save"],
  "verify": { "highSeverities": ["P0", "P1"], "highVotes": 3, "midSeverities": ["P2"], "midVotes": 2, "lowVotes": 1, "codex": true, "codexModel": "gpt-6-sol" },
  "maxRounds": 2,
  "maxCriticTasks": 12
}
```

| Field | Meaning |
|---|---|
| `mode` | `plan` returns the task list and agent counts and starts no agent. `run` runs the sweep. |
| `runDir` | Absolute path of the run folder. Git must ignore it. |
| `profile` | Absolute path of the project profile, or `null`. |
| `lenses` | Each lens that a shard or the critic can use. The id is a file name in `references/lenses/`. `model` is optional: omit it to use the session model. |
| `shards` | The shards. See below. |
| `only` | Optional. Shard ids for a pilot. The critic can still propose tasks on other shards. |
| `verify` | Optional. The values above are the defaults. |
| `maxRounds` | Round 1 is the shard map. Each later round is one critic and its tasks. `1` turns off the critic. |
| `maxCriticTasks` | The maximum number of finder tasks in one critic round. |

## Shards

| Field | Meaning |
|---|---|
| `id` | Short and unique. It appears in labels and in the report. |
| `category` | `own` (our app code), `borrowed-modified`, `usage`, `test`, or `seam`. |
| `intent` | Two to five sentences: what this code must do, and what must never happen. The finders judge the code against this. Take it from the project's docs, not from the code alone. |
| `files` | Paths from the repository root. A finder reads all of them. |
| `notes` | Optional. Facts that a finder needs: upstream data for borrowed files, the dependencies for a usage shard, the entry points for a seam. |
| `lenses` | Lens ids. One finder runs for each lens. |

## How to divide the code

- **Size.** Aim for 1,500 to 4,000 lines of app code in an `own` shard. A finder must read all the files of its shard. Split a larger module by function, not by a line count.
- **Cohesion.** Put files together that change together and share state. Put a module's tests in a separate `test` shard, but the finders of the `own` shard can still read them.
- **Test shards.** Aim for up to 6,000 lines of test code. They get only the `test-fidelity` lens.
- **Borrowed files.** A copied file without changes goes into a `usage` shard, with the files that call it. A changed file goes into a `borrowed-modified` shard with the upstream data in `notes`. When our changes are most of the file, treat it as `own`.
- **Usage shards.** One shard per important dependency, or per group of small ones. `notes` names the dependency, its version, and where its source is. `files` are our call sites.
- **Seams.** About one seam for each important flow that crosses modules. `files` lists the files of the flow in order. `notes` names the entry point. Seams get the `seam` lens.
- **Skip.** Generated files, vendored copies, build output, and data files.

## Lenses by category

| Category | Lenses |
|---|---|
| `own` | Choose by risk. `correctness` for each shard. Add `concurrency` when the code has threads, tasks, events, or shared state; `data-loss` when it writes, moves, or deletes user data; `error-recovery` when it does I/O or calls devices, models, or the network; `resources` when it holds devices, processes, threads, or large buffers; `ui-state` for interface code; `security` when it handles paths, secrets, untrusted text, downloads, or commands that the interface can call. |
| `borrowed-modified` | `borrowed-diff`. |
| `usage` | `usage`. |
| `test` | `test-fidelity`. |
| `seam` | `seam`. |

Most `own` shards get two to four lenses. A shard with all lenses is a sign that it is too large or that the risk tags are not selective.

## Agent count

Each finder task is one agent. Each new P0 or P1 finding adds `highVotes` agents, plus one for Codex. Each new P2 finding adds `midVotes` agents, plus one for Codex. Each new P3 finding adds `lowVotes` agents. Each later round adds one critic and up to `maxCriticTasks` finders. One agent writes the report. Run the workflow with `mode: "plan"` to get the finder count.
