# trudger Specification

## Purpose
Trudger is a generic task-processing loop for beads-backed task systems (for example `br`). It selects ready tasks, runs an agent solve + review loop, and verifies tasks are closed or escalated for human input.

## Requirements
### Requirement: Trudger entrypoint
The system SHALL provide an executable `trudger` entrypoint. The canonical implementation SHALL be the Rust binary.

#### Scenario: Entrypoint is executable
- **WHEN** a user runs `trudger --help`
- **THEN** the program starts without requiring an explicit shell invocation

### Requirement: Repo-root entrypoint is a shim
If a repository-root `./trudger` entrypoint is provided, it SHALL be a minimal shim that delegates to the Rust implementation and SHALL NOT contain the task-processing loop logic.

#### Scenario: Bash shim delegates
- **GIVEN** the repo contains `./trudger`
- **WHEN** a user runs `./trudger --help`
- **THEN** the Rust implementation is executed

### Requirement: Configuration loading
The system SHALL load configuration from `~/.config/trudger.yml` by default and SHALL allow overriding the config path via `-c/--config PATH`. The system SHALL exit with a clear error if the selected config file is missing.

#### Scenario: Missing default config prints bootstrap instructions
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints bootstrap instructions for sample configs

#### Scenario: Missing explicit config prints missing-path error
- **GIVEN** a user provides `-c/--config PATH`
- **AND** PATH does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints a clear missing config error that includes PATH

### Requirement: Configuration parsing
The system SHALL parse `~/.config/trudger.yml` using native Rust YAML parsing and SHALL treat null values as validation errors. The system SHALL NOT require external YAML/JSON parsing utilities (for example `yq` or `jq`) to load the configuration.

#### Scenario: Null config value
- **WHEN** a required config value is present but null
- **THEN** the system exits non-zero with a clear error naming the field

#### Scenario: Config parse failure
- **GIVEN** the config file exists but contains invalid YAML
- **WHEN** the system loads configuration
- **THEN** it exits non-zero with a clear parse error that names the config path

#### Scenario: Config parsing without external tools
- **GIVEN** `yq` and `jq` are not installed
- **WHEN** the config file is valid
- **THEN** the system loads and validates the configuration successfully

### Requirement: Configuration validation
The system SHALL require `agent_command`, `agent_review_command`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, and `hooks.on_requires_human` to be present and non-empty.

`commands.next_task` SHALL be required only when no manual task ids are provided.

`log_path` SHALL be optional; when it is missing or empty, logging is disabled.

`hooks.on_notification` SHALL be optional; when present, it MUST be non-empty.

`hooks.on_notification_scope` SHALL be optional; when present, it MUST be one of `all_logs`, `task_boundaries`, or `run_boundaries`. If `hooks.on_notification` is configured and `hooks.on_notification_scope` is omitted, the system SHALL default to `task_boundaries`.

If `hooks.on_notification_scope` is configured while `hooks.on_notification` is not configured, the system SHALL continue startup and SHALL emit a warning that `hooks.on_notification_scope` is ignored.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the system exits non-zero with a clear error naming the missing field

#### Scenario: Invalid notification scope errors
- **GIVEN** `hooks.on_notification_scope` is set to an unsupported value
- **WHEN** Trudger validates configuration
- **THEN** it exits non-zero with a clear error naming `hooks.on_notification_scope`

#### Scenario: Notification scope without hook warns and is ignored
- **GIVEN** `hooks.on_notification_scope` is configured
- **AND** `hooks.on_notification` is missing
- **WHEN** Trudger validates configuration
- **THEN** Trudger continues startup
- **AND** it emits a warning that `hooks.on_notification_scope` is ignored

### Requirement: Unknown config keys
The system SHALL emit warnings for unknown config keys at top-level and under `commands`/`hooks`, and continue.

#### Scenario: Unknown config key
- **WHEN** the config contains an unknown key
- **THEN** the system prints a warning naming the key and continues startup

### Requirement: Prompt file presence
The system SHALL verify that `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist before starting task-processing work and exit with a clear error message if either is missing. The system SHALL NOT require prompt files to run `trudger doctor`.

#### Scenario: Prompt file missing
- **GIVEN** one or both prompt files do not exist
- **WHEN** task-processing work starts
- **THEN** the system exits with a clear error indicating the missing prompt file path

#### Scenario: Doctor mode does not require prompts
- **GIVEN** one or both prompt files do not exist
- **WHEN** a user runs `trudger doctor`
- **THEN** the system does not error due to missing prompt files

### Requirement: Manual task ids via -t
The system SHALL accept manual task ids via `-t/--task` options. The `-t/--task` option MAY be provided multiple times, and each value MAY contain comma-separated task ids. The system SHALL process the manual task ids in the order specified and SHALL process them before selecting tasks via `commands.next_task`.

Manual task ids SHALL be validated as safe tokens: non-empty, at most 200 characters, starting with an ASCII letter or digit, and containing only ASCII letters/digits plus `-`, `_`, `.`, `:`.

#### Scenario: Repeated -t preserves order
- **WHEN** a user runs `trudger -t tr-1 -t tr-2`
- **THEN** the system processes `tr-1` before `tr-2`

#### Scenario: Comma-separated -t preserves order
- **WHEN** a user runs `trudger -t tr-1,tr-2`
- **THEN** the system processes `tr-1` before `tr-2`

#### Scenario: Mixed -t forms preserve order
- **WHEN** a user runs `trudger -t tr-1,tr-2 -t tr-3`
- **THEN** the system processes `tr-1`, then `tr-2`, then `tr-3`

#### Scenario: Comma-separated task ids trim whitespace
- **WHEN** a user runs `trudger -t "tr-1, tr-2"`
- **THEN** the system processes `tr-1` and `tr-2` (with surrounding whitespace trimmed)

#### Scenario: Empty comma segments error
- **WHEN** a user runs `trudger -t "tr-1,,tr-2"`
- **THEN** the system exits non-zero with a clear error indicating an empty task id was provided

#### Scenario: Invalid manual task id errors
- **WHEN** a user runs `trudger -t "tr-1;rm -rf /"`
- **THEN** the system exits non-zero with a clear error indicating the task id contains invalid characters

### Requirement: Positional task ids are rejected
The system SHALL NOT accept positional task ids. If unexpected positional arguments are provided, the system SHALL exit non-zero with a clear migration error instructing the user to use `-t/--task`.

#### Scenario: Positional task id errors
- **WHEN** a user runs `trudger tr-1`
- **THEN** the system exits non-zero with an error instructing the user to use `-t/--task`

### Requirement: Doctor entrypoint
The system SHALL provide a `trudger doctor` mode that initializes a scratch task database before running any doctor checks.

#### Scenario: Doctor mode uses temporary scratch directory
- **WHEN** a user runs `trudger doctor`
- **THEN** the system creates a temporary scratch directory
- **AND** it executes `hooks.on_doctor_setup` from the invocation working directory with `TRUDGER_DOCTOR_SCRATCH_DIR` set to the scratch directory

#### Scenario: Doctor mode completes after checks
- **GIVEN** `hooks.on_doctor_setup` exits with code 0
- **AND** doctor checks succeed
- **WHEN** a user runs `trudger doctor`
- **THEN** the system exits with code 0

### Requirement: Doctor hook validation
When running doctor mode, the system SHALL require `hooks.on_doctor_setup` to be present and non-empty.

#### Scenario: Doctor hook missing
- **GIVEN** `hooks.on_doctor_setup` is missing or empty
- **WHEN** Trudger runs in doctor mode
- **THEN** it exits non-zero with a clear error naming `hooks.on_doctor_setup`

### Requirement: Doctor check working directory
When doctor mode runs doctor checks, the system SHALL execute those checks with the scratch directory as the working directory. Doctor checks SHALL NOT require `TRUDGER_DOCTOR_SCRATCH_DIR` to be set and SHALL assume the scratch directory is their working directory.

#### Scenario: Checks run from scratch directory
- **GIVEN** doctor checks are executed
- **WHEN** Trudger runs in doctor mode after setup
- **THEN** doctor checks are executed from the scratch directory as the working directory

### Requirement: Doctor scratch cleanup
The system SHALL clean up the temporary scratch directory created for doctor mode on both success and failure, and SHALL exit non-zero if scratch cleanup fails.

#### Scenario: Doctor cleanup on success
- **GIVEN** doctor mode succeeds
- **WHEN** Trudger exits
- **THEN** it removes the temporary scratch directory before exiting

### Requirement: Command execution environment
The system SHALL execute configured commands and hooks without positional task arguments and SHALL provide task context via environment variables. When a task context exists, the system SHALL set `TRUDGER_TASK_ID`. After task show output is available, the system SHALL set `TRUDGER_TASK_SHOW`. After task status is available, the system SHALL set `TRUDGER_TASK_STATUS`. The system SHALL always set `TRUDGER_CONFIG_PATH` to the active config path.

Agent commands SHALL receive the relevant prompt content via `TRUDGER_PROMPT` (solve) or `TRUDGER_REVIEW_PROMPT` (review); the non-relevant prompt env var SHALL be unset.

Before spawning configured commands/hooks, the system SHALL truncate individual `TRUDGER_*` environment variable values that exceed 64 KiB (bytes) at a UTF-8 character boundary to reduce the risk of command spawn failures (E2BIG). When truncation occurs, the system SHALL print a warning and (when `log_path` is configured) log an `env_truncate` transition.

#### Scenario: Command environment provided
- **WHEN** Trudger executes a configured command or hook
- **THEN** it passes task context via `TRUDGER_*` environment variables
- **AND** it does not pass the task id as a positional argument

#### Scenario: Oversized env values are truncated and warned
- **WHEN** `TRUDGER_PROMPT` exceeds the truncation threshold
- **THEN** Trudger truncates it before spawning the command
- **AND** it prints a warning indicating truncation occurred

### Requirement: Task selection
The system SHALL select the next task by running the configured `commands.next_task` command, then evaluate readiness by running `commands.task_status`, and process one task at a time.

The first whitespace-delimited token of stdout from `commands.next_task` SHALL be treated as the task id and SHALL be validated as a safe token (see manual task id validation). If the token is invalid, the system SHALL exit non-zero with a clear error.

#### Scenario: No selectable tasks
- **WHEN** `commands.next_task` returns an empty result or exits with code 1
- **THEN** the system exits with status 0

#### Scenario: Task not ready is skipped
- **WHEN** `commands.next_task` returns a task whose `commands.task_status` result is not `ready` or `open`
- **THEN** the system skips it and retries up to `TRUDGER_SKIP_NOT_READY_LIMIT` before idling

#### Scenario: Invalid next_task output errors
- **WHEN** `commands.next_task` returns a first token that contains invalid characters
- **THEN** the system exits non-zero with a clear error indicating the task id is invalid

### Requirement: Agent prompt execution
For each selected task, the system SHALL execute the configured `agent_command` for the solve step and `agent_review_command` for the review step. The system SHALL load prompt content from `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` without performing `$ARGUMENTS` or `$TASK_SHOW` substitutions, and SHALL provide the prompt content via `TRUDGER_PROMPT` (solve) and `TRUDGER_REVIEW_PROMPT` (review) environment variables alongside task context (`TRUDGER_*`).

#### Scenario: Agent solve + review
- **WHEN** a task is selected
- **THEN** the system invokes `agent_command` with the trudge prompt content
- **AND** the system invokes `agent_review_command` with the review prompt content

#### Scenario: Prompt context via env vars
- **WHEN** Trudger invokes the agent commands
- **THEN** task context is provided via `TRUDGER_*` environment variables
- **AND** the relevant prompt env var is set (`TRUDGER_PROMPT` for solve, `TRUDGER_REVIEW_PROMPT` for review) while the other is unset
- **AND** prompt templates are not substituted by Trudger

### Requirement: Task show output handling
The system SHALL treat `commands.task_show` output as free-form prompt content and SHALL NOT parse it for control flow decisions. The system SHALL provide the output via `TRUDGER_TASK_SHOW` for agent commands and hooks.

#### Scenario: Show output is prompt-only
- **GIVEN** `commands.task_show` is configured
- **WHEN** Trudger renders prompts for a task
- **THEN** it provides the show output via `TRUDGER_TASK_SHOW` without parsing task status

### Requirement: Hook execution semantics
The system SHALL execute hooks without positional task arguments and SHALL provide task context via `TRUDGER_*` environment variables.

#### Scenario: Hook receives env vars
- **WHEN** a hook command executes
- **THEN** it receives `TRUDGER_TASK_ID` and other task context via environment variables
- **AND** no positional task id argument is passed

### Requirement: Optional notification hook dispatch
The system SHALL support an optional notification hook command at `hooks.on_notification`.

When `hooks.on_notification` is not configured, the system SHALL NOT emit notification-hook invocations.

Notification hook dispatch SHALL apply only to task-processing mode (`trudger` default run mode). `trudger wizard` and `trudger doctor` SHALL NOT emit notification-hook invocations.

When `hooks.on_notification` is configured, the system SHALL invoke it according to `hooks.on_notification_scope`:
- `all_logs`: invoke once per transition log message, including when `log_path` is disabled.
- `task_boundaries`: invoke on task start and task end.
- `run_boundaries`: invoke on Trudger run start and run end.

Notification hook invocations SHALL execute without positional arguments.

If the notification hook exits non-zero or fails to spawn, the system SHALL continue task processing and SHALL emit a warning/transition describing the notification failure.

When `hooks.on_notification_scope` is `all_logs`, transitions generated by the notification subsystem itself (for example notification failure transitions) SHALL NOT trigger additional notification dispatch.

#### Scenario: Notification hook omitted
- **GIVEN** `hooks.on_notification` is not configured
- **WHEN** Trudger runs
- **THEN** no notification hook command is invoked

#### Scenario: Notification hook is run-mode only
- **GIVEN** `hooks.on_notification` is configured
- **WHEN** a user runs `trudger doctor` or `trudger wizard`
- **THEN** no notification hook command is invoked

#### Scenario: Task boundary notifications
- **GIVEN** `hooks.on_notification` is configured
- **AND** `hooks.on_notification_scope` is `task_boundaries`
- **WHEN** a task starts and later ends
- **THEN** the notification hook is invoked once at start and once at end for that task

#### Scenario: Run boundary notifications
- **GIVEN** `hooks.on_notification` is configured
- **AND** `hooks.on_notification_scope` is `run_boundaries`
- **WHEN** Trudger starts and later exits
- **THEN** the notification hook is invoked once at run start and once at run end

#### Scenario: All-log notifications
- **GIVEN** `hooks.on_notification` is configured
- **AND** `hooks.on_notification_scope` is `all_logs`
- **WHEN** Trudger emits transition log messages
- **THEN** the notification hook is invoked once per transition log message

#### Scenario: All-log notifications still run with log file disabled
- **GIVEN** `hooks.on_notification` is configured
- **AND** `hooks.on_notification_scope` is `all_logs`
- **AND** `log_path` is omitted or empty
- **WHEN** Trudger emits transition log messages
- **THEN** the notification hook is invoked once per transition log message

#### Scenario: Notification failures are non-fatal
- **GIVEN** `hooks.on_notification` is configured
- **AND** a notification hook invocation fails
- **WHEN** Trudger is processing a task
- **THEN** task processing continues
- **AND** the failure is surfaced via warning/transition output

#### Scenario: Notification failure does not recurse in all_logs mode
- **GIVEN** `hooks.on_notification_scope` is `all_logs`
- **AND** a notification hook invocation fails
- **WHEN** Trudger records a notification-failure transition
- **THEN** that transition does not trigger another notification-hook invocation

### Requirement: Notification payload contract
For each notification hook invocation, the system SHALL provide notification payload fields via `TRUDGER_NOTIFY_*` environment variables.

The payload SHALL include:
- `TRUDGER_NOTIFY_EVENT` (one of `run_start`, `run_end`, `task_start`, `task_end`, `log`)
- `TRUDGER_NOTIFY_DURATION_MS` (elapsed duration in milliseconds; see event-specific rules below)
- `TRUDGER_NOTIFY_FOLDER` (absolute invocation working directory)
- `TRUDGER_NOTIFY_EXIT_CODE` (run exit code for `run_end`; unset otherwise)
- `TRUDGER_NOTIFY_TASK_ID` (task id when applicable; empty otherwise)
- `TRUDGER_NOTIFY_TASK_DESCRIPTION` (human-readable task description when available; empty otherwise)
- `TRUDGER_NOTIFY_PAYLOAD_PATH` (path to a JSON file containing the same payload fields)

Event-specific duration rules SHALL be:
- `run_start`: `0`
- `run_end`: elapsed milliseconds since `run_start`
- `task_start`: `0`
- `task_end`: elapsed milliseconds since matching `task_start`
- `log`: elapsed milliseconds since `run_start`

Human-readable task description SHOULD be derived from the most recent `commands.task_show` output for that task. When that output is JSON, the system SHOULD prefer `title`/`summary` fields (including nested `fields.summary`) before falling back to `description` or plain text.

When `hooks.on_notification_scope` is `all_logs`, the payload SHALL also include `TRUDGER_NOTIFY_MESSAGE` containing a redacted transition message. Redaction SHALL replace values of `command=` and `args=` segments with `[REDACTED]`. Control characters SHALL be escaped consistently with transition logging.

`TRUDGER_NOTIFY_*` values SHALL be subject to the same environment-size truncation protections as other Trudger-provided environment variables.

#### Scenario: Task notification includes required fields
- **GIVEN** a task notification is emitted
- **WHEN** the notification hook executes
- **THEN** it receives duration, folder, task id, and task description via `TRUDGER_NOTIFY_*` environment variables

#### Scenario: Run boundary notification has empty task fields
- **GIVEN** a run-boundary notification is emitted outside task context
- **WHEN** the notification hook executes
- **THEN** `TRUDGER_NOTIFY_TASK_ID` and `TRUDGER_NOTIFY_TASK_DESCRIPTION` are empty

#### Scenario: All-log payload includes transition message
- **GIVEN** notification scope is `all_logs`
- **WHEN** a transition log message is emitted
- **THEN** `TRUDGER_NOTIFY_MESSAGE` is set to that transition message for the notification hook invocation

#### Scenario: All-log payload redacts command and args fields
- **GIVEN** notification scope is `all_logs`
- **AND** a transition message includes `command=` and `args=` fields
- **WHEN** Trudger prepares the notification payload
- **THEN** `TRUDGER_NOTIFY_MESSAGE` replaces both field values with `[REDACTED]`

#### Scenario: Run and task start durations are zero
- **GIVEN** a `run_start` or `task_start` notification is emitted
- **WHEN** the notification hook executes
- **THEN** `TRUDGER_NOTIFY_DURATION_MS` is `0`

#### Scenario: Run end payload includes exit code
- **GIVEN** a `run_end` notification is emitted
- **WHEN** the notification hook executes
- **THEN** `TRUDGER_NOTIFY_EXIT_CODE` is set to the run exit code

#### Scenario: Non-run-end payload leaves exit code unset
- **GIVEN** a notification event other than `run_end` is emitted
- **WHEN** the notification hook executes
- **THEN** `TRUDGER_NOTIFY_EXIT_CODE` is unset

#### Scenario: Notification payload values are truncated when oversized
- **GIVEN** a `TRUDGER_NOTIFY_*` payload value exceeds env-size limits
- **WHEN** Trudger executes the notification hook
- **THEN** Trudger truncates the value at a UTF-8 boundary before spawn
- **AND** it emits truncation warnings/transitions consistent with existing env-truncation behavior

### Requirement: Task closure on success
When the review step indicates the task is closed, the system SHALL invoke `hooks.on_completed`.

#### Scenario: Task closed after successful review
- **WHEN** `commands.task_status` returns `closed` after the review step
- **THEN** `hooks.on_completed` is executed for that task

### Requirement: Requires-human escalation
When the review step indicates the task is `blocked`, the system SHALL invoke `hooks.on_requires_human`.

#### Scenario: Task blocked after review
- **WHEN** `commands.task_status` returns `blocked` after the review step
- **THEN** `hooks.on_requires_human` is executed for that task

### Requirement: Review loop limit exhaustion
When the review step indicates the task is neither `closed` nor `blocked`, the system SHALL retry solve + review until `review_loop_limit` is exhausted. If the task is still not `closed` after exhausting the limit, the system SHALL mark the task `blocked` and invoke `hooks.on_requires_human`.

#### Scenario: Review loop retries until closed
- **GIVEN** a task remains `open` after review
- **WHEN** the solve + review loop continues
- **THEN** Trudger retries until the task becomes `closed` or the limit is exhausted

#### Scenario: Review loop exhaustion marks blocked
- **GIVEN** a task never becomes `closed` after review
- **WHEN** `review_loop_limit` is exhausted
- **THEN** Trudger updates the task status to `blocked`
- **AND** it invokes `hooks.on_requires_human`

### Requirement: Agent update verification
After the review step, the system SHALL verify that the task has a non-empty status (from `commands.task_status`) and error if status is missing.

#### Scenario: Missing status after review
- **WHEN** the review step completes and `commands.task_status` returns an empty result
- **THEN** the system exits with a non-zero status

### Requirement: Execution logging
When `log_path` is configured, the system SHALL log command start/exit and quit reasons as single-line entries, escaping control characters (newlines, carriage returns, tabs) in logged values. The system SHALL log the full configured command strings and arguments without redaction.

#### Scenario: Command logging includes full command
- **WHEN** a configured command executes
- **THEN** the log entry includes the full command string and arguments

#### Scenario: Log values include control characters
- **WHEN** a logged value contains newlines, carriage returns, or tabs
- **THEN** the log entry replaces them with escaped sequences
