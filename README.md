# trudger

trudger slowly and unimaginatively trudges through your bd tasks.

## Why

Trudger is more or less a direct response to the experience of trying to build something in Gastown.
It is slower and more serial, but if you have a large number of smaller projects like me, I'm betting you can quite easily just have a constant, slow, serial trudge through all of them, and interact with them mainly through openspec and beads.

## What it does

- Finds the next `bd` task labeled `trudgeable` (highest priority first).
- Marks the task `in_progress`.
- Runs Codex solve + review prompts for that task.
- On success, removes the `trudgeable` label and moves on.
- If the task needs a human, it labels it `requires-human`.

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

Trudger reads `~/.config/trudger.yml` on startup. If the file is missing, it prints a warning and uses defaults.

Defaults:
- `codex_command`: `codex --yolo exec`
- `next_task_command`: empty (uses `bd ready` selection)
- `review_loop_limit`: `5`
- `log_path`: `./.trudger.log`
- `hooks.on_completed`: empty
- `hooks.on_requires_human`: empty
- `labels.trudgeable`: `trudgeable`
- `labels.requires_human`: `requires-human`

Example (defaults shown):

```yaml
codex_command: "codex --yolo exec"
next_task_command: ""
review_loop_limit: 5
log_path: "./.trudger.log"

hooks:
  on_completed: ""
  on_requires_human: ""

labels:
  trudgeable: "trudgeable"
  requires_human: "requires-human"
```

Notes:
- `codex_command` is used for solve; review uses the same command with `resume --last` appended.
- `next_task_command` runs in a shell and the first whitespace-delimited token of stdout is used as the task id.
- When hooks are configured, label updates are skipped. When hooks are not configured, labels are used if present.
- Set label values to empty strings to disable label behavior.

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

- Task selection uses `next_task_command` when configured, otherwise `bd ready` with optional label filtering.
- If a task is closed after review, Trudger either runs `on_completed` or removes the trudgeable label (when configured).
- If a task is marked requires-human after review, Trudger either runs `on_requires_human` or updates labels (when configured).

## Exit behavior

- Exits `0` when there are no matching tasks left.
- Exits `1` if a task is neither closed nor marked `requires-human` after review.
