# trudger

trudger slowly and unimaginatively trudges through your bd tasks.

## Why

Trudger is more or less a direct response to the experience of trying to build something in Gastown.
It is slower and more serial, but if you have a large number of smaller projects like me, I'm betting you can quite easily just have a constant, slow, serial trudge through all of them, and interact with them mainly through openspec and beads.

## What it does

- Finds the next `bd` task labeled `trudgeable` (highest priority first).
- Runs Codex solve + review prompts for that task.
- On success, removes the `trudgeable` label and moves on.
- If the task needs a human, it labels it `requires-human`.

## Requirements

- `bd` CLI on your PATH
- `codex` CLI on your PATH
- `jq` on your PATH
- Prompt files installed under `~/.codex/prompts/` (see below)

## Usage

```bash
trudger
```

## Install

Assuming you want `trudger` on your PATH via `~/.local/bin`:

```bash
install -m 0755 ./trudger ~/.local/bin/trudger
```

To see help:

```bash
trudger --help
```

## Prompts

The prompt sources live in `prompts/`. Install them into Codex:

```bash
install -m 0644 prompts/trudge ~/.codex/prompts/trudge
install -m 0644 prompts/trudge_review ~/.codex/prompts/trudge_review
```

## Behavior details

- Only tasks labeled `trudgeable` are processed.
- If a task is closed after review, `trudgeable` is removed automatically.
- If a task is labeled `requires-human` after review, the tool:
  - Removes `trudgeable`
  - Adds/keeps `requires-human`

## Exit behavior

- Exits `0` when there are no matching tasks left.
- Exits `1` if a task is neither closed nor marked `requires-human` after review.
