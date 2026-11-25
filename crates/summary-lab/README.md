# Summary lab

The summary lab is a dev tool. It compares summaries from the summary agent with today's summaries on your own notes. The app does not use it, and it is not in the app build.

## Before you start

- **Your notes leave your computer.** A run sends the transcripts and manual notes of the notes that you choose to the models under test and to the two judge models, through OpenRouter. The lab asks OpenRouter for hosts that do not store request data. Set `zdr` to also ask for zero data retention. A model that has no such host does not run.
- **The run folder holds real note text.** Results go to `.context/summary-lab/` in the current folder. Never commit, upload or share this folder.
- The lab reads your storage folder and never changes it. It works on a copy (the snapshot) in the run folder.

## Keys

| Variable | Use |
|---|---|
| `OPENROUTER_API_KEY` | Every model and both judges, unless the model starts with `litellm:` |
| `LITELLM_BASE_URL`, `LITELLM_API_KEY` | Models that start with `litellm:`, on your LiteLLM server |

The lab reads each value from the environment, else from a `.env` file in the current folder or a folder above it. It never reads the Keychain, and it never writes a key to a run file.

## Commands

Run each command from the repository folder.

```sh
# Which models have a host that runs tool calls and stores no data
cargo run -p summary-lab -- check-hosts --models crates/summary-lab/models/check-hosts.txt

# Call one note tool, as the agent does
cargo run -p summary-lab -- tool list_notes '{"tags": ["Person::Nathan"]}' --root <storage folder>
cargo run -p summary-lab -- tool get_note '{"note_id": "<id>", "transcript": {"from_line": 1, "to_line": 40}}' --root <storage folder>

# Plan a run: prints the notes, the skipped notes and the most model calls
cargo run -p summary-lab -- run --config lab.json --dry-run

# Run, judge and write the report
cargo run -p summary-lab -- run --config lab.json

# Continue a run that stopped: writes only the missing results
cargo run -p summary-lab -- run --config lab.json --resume

# Write the report again from the files, with no network
cargo run -p summary-lab -- report <run name>

# One table from runs with the same notes and models
cargo run -p summary-lab -- compare <run name> <run name>

# Judge runs with the same notes and models in the same requests: the
# reference run's baselines and agent summary k of each run, for each note,
# model and k. The report gives absolute scores, length, error counts and
# the difference to the first run for each note, with a 90% interval.
cargo run -p summary-lab -- versus --name <name> --run <reference run> --run <run>
```

`--out <folder>` puts the run folders in another folder than `.context/summary-lab`.

## Configuration

A run reads a JSON file. Only `name`, `storage`, `notes` and `models` are necessary.

```json
{
  "name": "haiku-single-1",
  "storage": "/Users/me/Longhand",
  "notes": {
    "ids": ["<note id>", "<note id>"],
    "list": {"tags": ["Project::Atlas"], "since": "2026-10-01", "limit": 10}
  },
  "models": [
    "anthropic/claude-haiku-5.5",
    {"model": "openai/gpt-oss-20b", "context_tokens": 16000},
    "litellm:halogen-qwen3.8-flash-next"
  ],
  "shape": "single",
  "runs": 3,
  "judges": ["anthropic/claude-opus-5.5", "openai/gpt-6.2-sol"]
}
```

| Field | Meaning | Default |
|---|---|---|
| `name` | The run folder name: letters, digits, `.`, `_` and `-` | |
| `storage` | Your storage folder | |
| `notes.ids` | Note ids to use | |
| `notes.list` | A `list_notes` input. With `ids`, a note must be in both | |
| `models` | Model ids. An object can set `context_tokens`, for example the size that a local Ollama or LM Studio server gives a small model, and `output_tokens`, the most output tokens that the server sends in one reply, when the server enforces less than it reports | |
| `shape` | `auto`: `single` when the whole transcript fits in the room for the tool results of one turn, else `readers`, for each note and model. `single`: one agent with the note tools. `readers`: a lead agent also gives line ranges to reader agents, each in a new context, which get the lines and return notes. `steps`: one step for each transcript part, then a merge step. `timeline`: code-driven steps. Agents with no tools write the timeline of each transcript part in order, with the manual notes and the people list of the part before. Then one writer, or a writer for each group of parts and a merge step, writes the summary. The writers can call `get_note` to check a point | `auto` |
| `review` | After the summary, auditors check it against the transcript, part by part, for missed, unsupported, misattributed, out-of-order and noise-as-fact errors. A reviser then gives edits, and the code applies them | false |
| `existing_summary` | Keep each note's summary in the snapshot, so that the agent can read it, as when the app makes a summary again | false |
| `baselines` | Baselines for each note and model. With more than one, the loss uses their mean | 1 |
| `strategy` | The reading instructions of the agent | the built-in strategy |
| `history` | `"compact"`: when the context is nearly full, the model writes notes that replace the older messages. `"keep_all"`: a request that does not fit ends the run. `{"trim": n}`: keep only the last n tool results whole | `compact` |
| `prompt` | A summary prompt id | the prompt of each note's type, as in the app |
| `runs` | Agent runs for each note and model | 3 |
| `limits` | `model_calls`, `tool_calls` and `malformed_steps` of each agent run. These stop a model that repeats itself; they do not limit cost. `reply_tokens` sets a fixed reply allowance | 100, 200, 3. The reply allowance is the model's output maximum, cut to the room that the request leaves, as in the app |
| `reader_limits` | The same limits for each reader of the `readers` shape | the same defaults |
| `max_chars` | The largest size of one `get_note` result | 25,000 tokens (about the cap of Claude Code's file read tool), at most a quarter of the model's context |
| `token_cap` | The run stops when the agent turns pass this number of tokens | none |
| `zdr` | Ask for hosts with zero data retention | false |
| `judges` | The judge models. `[]` judges nothing | Claude Opus 5.5 and GPT-6.2 Sol |
| `seed` | The seed of the summary order for the judges | chosen and recorded |

The context budget of a model is `context_tokens` when you set it, else the model's input limit in Rig's catalog, else what the model's server reports, else 128,000 tokens.

## What a run does

1. It selects the notes. A note that is busy, has no transcript, has no speech, or cannot be read is skipped. The manifest gives the reason.
2. It copies the selected notes and the `.longhand` settings into the snapshot. It does not copy symlinks or audio.
3. For each note and model, it writes the baseline: today's summary from the app's own summary code, with the same context budget. A later run uses the same baseline again while the note, the model, the budget and the summary prompt do not change. A change to the agent's strategy or shape does not make a new baseline.
4. It runs the agent the configured number of times.
5. Each judge gets the transcript, the manual notes and all the summaries of one note and model, with no labels and in an order from the seed. It scores coverage, faithfulness and overall quality from 1 to 10.
6. It writes `report.md` and `report.json`.

## The run folder

```text
.context/summary-lab/
  <name>/manifest.json        configuration, notes, skipped notes, budgets, seed
  <name>/snapshot/            the copy of your notes
  <name>/results/<note>/<model>/baseline.json
  <name>/results/<note>/<model>/agent-1.json    outcome, summary, each turn, tokens, cost, hosts
  <name>/results/<note>/<model>/judge-<judge>.json
  <name>/report.md
  <name>/report.json
  baselines/                  baselines that later runs use again
```

## Reading the report

- **Loss** is the baseline's overall score less the mean overall score of the agent runs, for each judge. A positive loss means the judge gave the agent summaries lower scores than today's summary from the same model.
- **No summary** is the part of the agent runs that ended with no summary. The outcomes give the reason: `tool_call_limit`, `context_exceeded`, `malformed_calls`, `no_tool_support`, `no_summary`, `cut_off` or `transport`.
- A missing judge result shows as missing. It is never a score of 0.
- The report gives measurements only. It does not say whether a model is good enough.
- The baseline does not have the "Recognition limitation" line that the app adds from recognition data.
