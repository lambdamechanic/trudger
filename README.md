# trudger

trudger slowly and unimaginatively trudges through your bd tasks.

## Why

Trudger is more or less a direct response to the experience of trying to build something in Gastown.
It is slower and more serial, but if you have a large number of smaller projects like me, I'm betting you can quite easily just have a constant, slow, serial trudge through all of them, and interact with them mainly through openspec and beads.

## What it does

- Uses the configured `next_task_command` to select the next task.
- Marks the task `in_progress`.
- Runs Codex solve + review prompts for that task.
- On success, invokes `hooks.on_completed`.
- If the task needs a human, invokes `hooks.on_requires_human`.

## Requirements

- `bd` CLI on your PATH
- `codex` CLI on your PATH
- `jq` on your PATH
- Prompt files installed under `~/.codex/prompts/` (see below):
  - `trudge.md`
  - `trudge_review.md`

## Usage

```bash
trudger
```

## Configuration

Trudger requires `~/.config/trudger.yml` on startup. If the file is missing, it prints curl commands for sample configs and exits non-zero.

Sample configs:
- `sample_configuration/trudgeable-with-hooks.yml`
  - Selects the next ready bd task labeled `trudgeable`.
  - On completion, removes `trudgeable`.
  - On requires-human, removes `trudgeable` and adds `human-required`.
- `sample_configuration/robot-triage.yml`
  - Selects tasks via `bv --robot-triage`.
  - No label changes (hooks are no-ops).

Example:

```yaml
codex_command: "codex --yolo exec"
next_task_command: "bd ready --json --label trudgeable --sort priority --limit 1 | jq -r 'if type == \"array\" and length > 0 then .[0].id // \"\" else \"\" end'"
review_loop_limit: 5
log_path: "./.trudger.log"

hooks:
  on_completed: "bash -lc 'bd label remove \"$1\" \"trudgeable\"'"
  on_requires_human: "bash -lc 'bd label remove \"$1\" \"trudgeable\"; bd label add \"$1\" \"human-required\"'"
```

Notes:
- `codex_command` is used for solve; review uses the same command with `resume --last` appended.
- `next_task_command` runs in a shell and the first whitespace-delimited token of stdout is used as the task id.
- `hooks.on_completed` and `hooks.on_requires_human` are required; label updates must happen in hooks if you want them.

## Install

Assuming you want `trudger` on your PATH via `~/.local/bin`:

```bash
./install.sh
```

To overwrite existing prompts:

```bash
./install.sh --force
```

To see help:

```bash
trudger --help
```

## Prompts

The prompt sources live in `prompts/` and are installed by `./install.sh`.

## Behavior details

- Task selection uses `next_task_command` and expects the first whitespace-delimited token of stdout to be the task id.
- If a task is closed after review, Trudger runs `hooks.on_completed`.
- If a task remains open after review, Trudger runs `hooks.on_requires_human`.

## Exit behavior

- Exits `0` when there are no tasks returned by `next_task_command`.
- Exits `1` if configuration is missing/invalid or a task lacks status after review.
