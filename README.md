# trudger

trudger slowly and unimaginatively trudges through your br tasks.

## Why

Trudger is more or less a direct response to the experience of trying to build something in Gastown.
It is slower and more serial, but if you have a large number of smaller projects like me, I'm betting you can quite easily just have a constant, slow, serial trudge through all of them, and interact with them mainly through openspec and beads_rust.

## What it does

- Uses `commands.next_task` to select the next task.
- Marks the task `in_progress` via `commands.task_update_in_progress`.
- Runs Codex solve + review prompts for that task.
- On success, invokes `hooks.on_completed`.
- If the task needs a human, invokes `hooks.on_requires_human`.

## Requirements

- `codex` CLI on your PATH
- `yq` on your PATH for config parsing
- Any task system CLIs referenced by your configured commands (for example `bd`, `br`, `bv`)
- Prompt files installed under `~/.codex/prompts/` (see below):
  - `trudge.md`
  - `trudge_review.md`

## Usage

```bash
trudger
```

Use a specific config file:

```bash
trudger --config ./sample_configuration/trudgeable-with-hooks.yml
```

## Configuration

Trudger requires `~/.config/trudger.yml` on startup unless `-c/--config` is provided. If the file is missing, it prints curl commands for sample configs and exits non-zero.
Configuration is parsed with `yq`; unknown top-level keys are logged as warnings and ignored.

Sample configs:
- `sample_configuration/trudgeable-with-hooks.yml`
  - Selects the next ready br task labeled `trudgeable`.
  - On completion, removes `trudgeable`.
  - On requires-human, removes `trudgeable` and adds `human-required`.
- `sample_configuration/robot-triage.yml`
  - Selects tasks via `bv --robot-next`.
  - No label changes (hooks are no-ops).

Example:

```yaml
codex_command: "codex --yolo exec"
commands:
  next_task: 'task_id=$(bd ready --json --label trudgeable --sort priority --limit 1 | jq -r "if type == \"array\" and length > 0 then .[0].id // \"\" else \"\" end"); if [[ -z "$task_id" ]]; then exit 1; fi; printf "%s" "$task_id"'
  task_show: "bd show"
  task_update_in_progress: "bd update"
review_loop_limit: 5
log_path: "./.trudger.log"

hooks:
  on_completed: "bash -lc 'br label remove \"$1\" \"trudgeable\"'"
  on_requires_human: "bash -lc 'br label remove \"$1\" \"trudgeable\"; br label add \"$1\" \"human-required\"'"
```

Notes:
- `codex_command` is used for solve; review uses the same command with `resume --last` appended.
- Required keys (non-empty, non-null): `codex_command`, `review_loop_limit`, `log_path`, `commands.next_task`, `commands.task_show`, `commands.task_update_in_progress`, `hooks.on_completed`, `hooks.on_requires_human`.
- Null values are treated as validation errors for required keys.
- `commands.next_task`, `commands.task_show`, and `commands.task_update_in_progress` are required and must be non-empty.
- `commands.next_task` runs in a shell and the first whitespace-delimited token of stdout is used as the task id.
- `commands.task_show` runs as `<command> <task_id> --json` (task id is the first argument); output is passed to Codex unparsed.
- `commands.task_update_in_progress` runs as `<command> <task_id> --status in_progress` (task id is the first argument); output is ignored.
- `hooks.on_completed` and `hooks.on_requires_human` are required; label updates must happen in hooks if you want them.
- Hook commands honor shell quoting. If a hook contains `$1`/`${1}`, Trudger runs it via `bash -lc` and passes the task id as `$1`; otherwise the task id is prepended as the first argument.

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

- Task selection uses `commands.next_task` and expects the first whitespace-delimited token of stdout to be the task id.
- `commands.task_show` output is treated as free-form task details for Codex.
- Tasks must be in status `ready` or `open` (from `commands.task_show --json`). When selecting via `commands.next_task`, Trudger skips non-ready tasks up to `TRUDGER_SKIP_NOT_READY_LIMIT` (default 5) before idling; manual task IDs still error if not ready.
- If `commands.next_task` exits 1 or returns an empty task id, Trudger exits 0 (no selectable tasks).
- If a task is closed after review, Trudger runs `hooks.on_completed`.
- If a task remains open after review, Trudger runs `hooks.on_requires_human`.

## Exit behavior

- Exits `0` when `commands.next_task` exits `1` (no tasks).
- Exits non-zero when `commands.next_task` fails for any other reason.
- Exits `1` if configuration is missing/invalid or a task lacks status after review.
