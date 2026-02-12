## 1. Config + Validation
- [x] 1.1 Add `hooks.on_notification` as an optional non-empty string config key.
- [x] 1.2 Add `hooks.on_notification_scope` as an optional enum config key with allowed values `all_logs`, `task_boundaries`, `run_boundaries` and default `task_boundaries` when notification hook is configured.
- [x] 1.3 Update unknown-key allowlists and validation paths so the new keys are treated as known.

## 2. Notification Dispatch
- [x] 2.1 Implement notification dispatch that executes `hooks.on_notification` (when configured) with no positional args. Depends on: 1.1, 1.2.
- [x] 2.2 Emit notifications for run boundaries (`run_start`, `run_end`) and task boundaries (`task_start`, `task_end`) with duration and folder context; emit `run_end` at absolute end of teardown. Depends on: 2.1.
- [x] 2.3 Implement `all_logs` mode so each transition log message can trigger a notification event, excluding notification-internal transitions to prevent recursion. Depends on: 2.1.
- [x] 2.4 Ensure notification hook failures are non-fatal and recorded as warnings/transitions. Depends on: 2.1.
- [x] 2.5 Implement best-effort `run_end` emission across all termination paths that can execute user code (normal, handled interrupt/signal, and error exits). Depends on: 2.1.

## 3. Payload Contract
- [x] 3.1 Provide `TRUDGER_NOTIFY_*` env vars including event name, duration, folder, run exit code (`run_end` only), task id, and human-readable task description. Depends on: 2.1.
- [x] 3.2 Extract task description from the first non-empty line of `commands.task_show` output (trimmed), with empty fallback. Depends on: 2.2.
- [x] 3.3 In `all_logs` mode, populate `TRUDGER_NOTIFY_MESSAGE` from transition text with `command=` and `args=` field values redacted to `[REDACTED]`. Depends on: 2.3.

## 4. Wizard + Templates + Docs
- [x] 4.1 Update wizard/template handling to preserve and round-trip notification keys. Depends on: 1.3. Coordination edge: `add-trudger-wizard-prompt-install`.
- [x] 4.2 Update README/sample config docs with notification examples for each scope mode. Depends on: 2.2, 2.3, 3.1.

## 5. Tests + Validation
- [x] 5.1 Add unit/integration coverage for config parsing, scope behavior, payload fields (including `TRUDGER_NOTIFY_EXIT_CODE`), redaction behavior, and non-fatal notification failures. Depends on: 1.1-4.1.
- [x] 5.2 Run Rust quality gates (`cargo nextest run` preferred; fallback `cargo test`, plus fmt/clippy as used in repo). Depends on: 5.1.
- [x] 5.3 Run `openspec validate add-notification-hook-modes --strict --no-interactive`. Depends on: spec/task/proposal updates complete; can run in parallel with 5.2 once implementation is done.

## 6. Parallelism Notes
- 1.1, 1.2, and 1.3 can run in parallel.
- 2.2 and 3.1 can run in parallel after 2.1.
- 4.1 and code-path work (2.x/3.x) can run in parallel after 1.3, except for merge coordination with `add-trudger-wizard-prompt-install`.
