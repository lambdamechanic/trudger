## Context
We want to add subcommands (starting with `doctor`) without colliding with manual task id positional arguments.

## Goals / Non-Goals
- Goals:
  - Make the CLI unambiguous for subcommands.
  - Support explicit manual task ids via a flag.
  - Preserve existing semantics: manual task ids (when provided) run before `commands.next_task` selection.
- Non-Goals:
  - Implement doctor mode itself (handled by `add-trudger-doctor-scratch-db`).
  - Rework the shell wrapper CLI contract.

## Decisions
- Reserve positional arguments for subcommands.
- Manual task ids are passed via `-t/--task`.
  - `-t` may be repeated.
  - Each `-t` value may contain comma-separated ids.
  - Order is preserved based on appearance.
  - Task ids are trimmed for surrounding whitespace; empty comma segments are rejected.
- Positional task ids are a hard error with a migration hint.

## Implementation Notes
- Prefer a standard Rust argument parser (`clap`) to avoid bespoke parsing logic and to keep help/usage consistent as subcommands grow.

## Migration Plan
- Update docs to show `trudger -t tr-1 -t tr-2` and `trudger -t tr-1,tr-2`.
- Provide an error message for positional task ids that tells the user to switch to `-t`.
