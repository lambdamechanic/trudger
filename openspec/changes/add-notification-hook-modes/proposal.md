# Change: Add optional notification hook with configurable scope

## Why
Teams want Trudger to emit external notifications (for example Discord) without coupling that behavior to task-completion hooks only. They need a single optional notification hook and a way to control event volume (all logs vs task boundaries vs run boundaries).

## What Changes
- Add an optional `hooks.on_notification` command hook.
- Add optional `hooks.on_notification_scope` to control notification frequency with three modes:
  - `all_logs`
  - `task_boundaries`
  - `run_boundaries`
- Define notification payload env vars so each notification includes duration, folder, task id, and a human-readable task description.
- Define redaction for `all_logs` notification messages so command strings/args are not forwarded verbatim.
- Include run-exit metadata in notifications with an explicit run end exit code field.
- Define fail-open behavior for notification hook execution (warnings/logging on notification failure, but no task-run abort).

## Dependencies
- Depends on `update-trudger-logging-and-parsing` for stable transition log semantics used by `all_logs` scope.
- Coordinate landing with `add-trudger-wizard-prompt-install` because both changes modify wizard/config/docs surfaces and may conflict in `src/wizard.rs`, `README.md`, and `openspec/specs/trudger/spec.md`.

## Impact
- Affected specs: `trudger`
- Affected code (planned): `src/config.rs`, `src/run_loop.rs`, `src/logger.rs`, `src/shell.rs`, wizard/template config generation, docs, and tests.
