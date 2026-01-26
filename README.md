# trudger

"trudger slowly and unimaginatively trudges through your bd tasks"

## What it does

- Finds the next `bd` task labeled `trudgeable` (highest priority first).
- Runs Codex solve + review prompts for that task.
- On success, removes the `trudgeable` label and moves on.
- If the task needs a human, it labels it `requires-human` and adds a note.

## Requirements

- `bd` CLI on your PATH
- `codex` CLI on your PATH
- Prompt files:
  - `~/.codex/prompts/trudge`
  - `~/.codex/prompts/trudge_review`

## Usage

```bash
./trudger
```

To see help:

```bash
./trudger --help
```

## Behavior details

- Only tasks labeled `trudgeable` are processed.
- If a task is closed after review, `trudgeable` is removed automatically.
- If a task is labeled `requires-human` after review, the tool:
  - Adds a comment and notes (customizable below)
  - Removes `trudgeable`
  - Adds/keeps `requires-human`

## Configuration

Customize the requires-human messaging with environment variables:

```bash
export TRUDGER_REQUIRES_HUMAN_COMMENT="Requires human input; see Codex review for details."
export TRUDGER_REQUIRES_HUMAN_NOTES="Requires human input; see Codex review for details."
```

## Exit behavior

- Exits `0` when there are no matching tasks left.
- Exits `1` if a task is neither closed nor marked `requires-human` after review.
