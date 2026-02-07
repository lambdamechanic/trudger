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

- `bash` on your PATH (configured commands are executed via `bash -lc`)
- `codex` CLI on your PATH
- `jq` on your PATH
- Any task system CLIs referenced by your configured commands (for example `bd`, `br`, `bv`)
- Prompt files installed under `~/.codex/prompts/` (task-processing mode only; see below):
  - `trudge.md`
  - `trudge_review.md`

## Usage

```bash
trudger
```

Generate a config interactively:

```bash
trudger wizard
```

Generate a config at a custom path:

```bash
trudger wizard --config ./trudger.yml
```

Run specific tasks first (positional task ids are not supported):

```bash
trudger -t tr-1 -t tr-2
trudger -t tr-1,tr-2
```

Use a specific config file:

```bash
trudger --config ./sample_configuration/trudgeable-with-hooks.yml
```

Doctor mode (runs `hooks.on_doctor_setup` and validates configured commands against a temporary scratch DB):

```bash
trudger doctor
```

## Configuration

Trudger requires `~/.config/trudger.yml` on startup unless `-c/--config` is provided, which overrides the default path. If the default config file is missing, it prints instructions to run `trudger wizard` (or `trudger wizard --config PATH`) and exits non-zero.
Configuration is parsed natively in Rust; unknown keys at top-level and under `commands`/`hooks` are logged as warnings and ignored.

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
agent_command: 'codex --yolo exec --prompt "$TRUDGER_PROMPT"'
agent_review_command: 'codex --yolo exec --prompt "$TRUDGER_REVIEW_PROMPT"'
commands:
  next_task: 'task_id=$(br ready --json --label trudgeable --sort priority --limit 1 | jq -r "if type == \"array\" and length > 0 then .[0].id // \"\" else \"\" end"); if [[ -z "$task_id" ]]; then exit 1; fi; printf "%s" "$task_id"'
  task_show: 'br show "$TRUDGER_TASK_ID"'
  task_status: 'br show "$TRUDGER_TASK_ID" --json | jq -r "if type == \"array\" then .[0].status // \"\" else .status // \"\" end"'
  task_update_in_progress: 'br update "$TRUDGER_TASK_ID" "$@"'
  reset_task: 'br update "$TRUDGER_TASK_ID" --status open'
review_loop_limit: 5
log_path: "./.trudger.log"

hooks:
  on_completed: 'br label remove "$TRUDGER_TASK_ID" "trudgeable"'
  on_requires_human: 'br label remove "$TRUDGER_TASK_ID" "trudgeable"; br label add "$TRUDGER_TASK_ID" "human-required"'
  on_doctor_setup: 'rm -rf "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads"; cp -R ".beads" "$TRUDGER_DOCTOR_SCRATCH_DIR/"'
```

Notes:
- All configured commands are executed via `bash -lc`.
- `agent_command` is used for solve; `agent_review_command` is used for review (Trudger appends `resume --last` to the review command arguments).
- Required keys (non-empty, non-null): `agent_command`, `agent_review_command`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `commands.reset_task`, `hooks.on_completed`, `hooks.on_requires_human`.
- `log_path` is optional; omit it or set it to an empty string to disable logging.
- `commands.next_task` is required when no manual task ids are provided.
- `hooks.on_doctor_setup` is required only for `trudger doctor`.
- Null values are treated as validation errors for required keys.
- `commands.next_task`, `commands.task_show`, `commands.task_status`, and `commands.task_update_in_progress` must be non-empty when used.
- `commands.next_task` runs in `bash -lc` and the first whitespace-delimited token of stdout is used as the task id.
- `commands.task_show` runs in `bash -lc`; its output is treated as prompt context only and is exposed via `TRUDGER_TASK_SHOW`.
- `commands.task_status` runs in `bash -lc`; the first whitespace-delimited token of stdout is used as the task status (for example `ready`, `open`, or `closed`) and is exposed via `TRUDGER_TASK_STATUS`.
- `commands.task_update_in_progress` runs in `bash -lc`; output is ignored.
- `hooks.on_completed` and `hooks.on_requires_human` are required; label updates must happen in hooks if you want them.
- Commands and hooks receive task context via environment variables instead of positional arguments.
- Trudger may append extra arguments to some commands (for example `commands.task_show` receives `--json` and `commands.task_update_in_progress` receives `--status in_progress` or `--status blocked`); include `$@` in the command string if you need them, but task id is always provided via `TRUDGER_TASK_ID`.
- Environment variables available to commands/hooks include `TRUDGER_TASK_ID` (set when a task is selected), `TRUDGER_TASK_SHOW` (set after `commands.task_show`), `TRUDGER_TASK_STATUS` (set after `commands.task_status`), `TRUDGER_CONFIG_PATH` (always set), `TRUDGER_PROMPT` (solve prompt only; unset during review), and `TRUDGER_REVIEW_PROMPT` (review prompt only; unset during solve).

## Install

Install the Rust binary with cargo (installs to `~/.cargo/bin` by default):

```bash
cargo install --path . --locked
```

Install prompt files under `~/.codex/prompts/`:

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

Legacy: the historical Bash implementation and its old BATS test suite live under `historical/bash/` (deprecated; kept for reference only).

## Prompts

The prompt sources live in `prompts/` and are installed by `./install.sh`.
- Trudger does not perform prompt substitutions; prompt content is delivered via `TRUDGER_PROMPT` and `TRUDGER_REVIEW_PROMPT`.

## Development

Enable the repo git hooks (runs `shellcheck`, `cargo fmt`, `cargo clippy`, and `cargo test` on pre-push):

```bash
git config core.hooksPath .githooks
```

### Coverage

Rust coverage is enforced at 100% (lines + regions) using `cargo llvm-cov`.

Local command:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
cargo llvm-cov --all-targets --ignore-filename-regex 'unit_tests\.rs$' --fail-under-lines 100 --fail-under-regions 100
```

Coverage scope:
- Includes all Rust sources under `src/` (production code), plus any in-file `#[cfg(test)]` modules.
- Excludes `src/unit_tests.rs` via `--ignore-filename-regex 'unit_tests\.rs$'` because it is a test-only harness/fixture module (compiled only under `cfg(test)`) and excluding it keeps coverage focused on production logic.

## Behavior details

- Task selection uses `commands.next_task` and expects the first whitespace-delimited token of stdout to be the task id.
- `commands.task_show` output is treated as free-form task details for the agent and provided via `TRUDGER_TASK_SHOW`.
- Control flow decisions (readiness and post-review status) use `commands.task_status`; `commands.task_show` is not used for status checks.
- Tasks must be in status `ready` or `open` (from `commands.task_status`). When selecting via `commands.next_task`, Trudger skips non-ready tasks up to `TRUDGER_SKIP_NOT_READY_LIMIT` (default 5) before idling; manual task ids (via `-t/--task`) still error if not ready.
- If `commands.next_task` exits 1 or returns an empty task id, Trudger exits 0 (no selectable tasks).
- If a task is closed after review, Trudger runs `hooks.on_completed`.
- If a task remains open after review, Trudger runs `hooks.on_requires_human`.

## Exit behavior

- Exits `0` when `commands.next_task` exits `1` (no tasks).
- Exits non-zero when `commands.next_task` fails for any other reason.
- Exits `1` if configuration is missing/invalid or a task lacks status after review.
