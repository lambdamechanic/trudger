# Change: Refactor task status updates to env-based contract

## Why
Trudger currently assumes `commands.task_update_status` accepts `--status <value>` argv and requires a separate `commands.reset_task` command. That couples Trudger to a `br update`-shaped CLI contract.

## What Changes
- Replace `commands.task_update_status` and `commands.reset_task` with a single required command: `commands.task_update_status`.
- Trudger sets target status via `TRUDGER_TARGET_STATUS` env var instead of appending `--status ...` args.
- Use `commands.task_update_status` for all status transitions (`in_progress`, `blocked`, `open`, `closed` in doctor checks).
- Update templates, sample config, prompts, docs, and tests to the new contract.
- Update local user config (`~/.config/trudger.yml`) to the new command key and env-based status contract.

## Impact
- Affected specs: `trudger`
- Affected code: config parsing/validation, run loop, doctor, shell env contract, wizard/templates, tests, docs
- Breaking: existing configs using `commands.task_update_status` and/or `commands.reset_task` must migrate
