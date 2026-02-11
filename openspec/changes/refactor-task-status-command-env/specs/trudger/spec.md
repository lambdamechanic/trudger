## MODIFIED Requirements
### Requirement: Configuration validation
The system SHALL require `agent_command`, `agent_review_command`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, and `hooks.on_requires_human` to be present and non-empty.

`commands.next_task` SHALL be required only when no manual task ids are provided.

`log_path` SHALL be optional; when it is missing or empty, logging is disabled.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the system exits non-zero with a clear error naming the missing field

### Requirement: Command execution environment
The system SHALL execute configured commands and hooks without positional task arguments and SHALL provide task context via environment variables. When a task context exists, the system SHALL set `TRUDGER_TASK_ID`. After task show output is available, the system SHALL set `TRUDGER_TASK_SHOW`. After task status is available, the system SHALL set `TRUDGER_TASK_STATUS`. The system SHALL always set `TRUDGER_CONFIG_PATH` to the active config path.

For status transitions, the system SHALL set `TRUDGER_TARGET_STATUS` for `commands.task_update_status` and SHALL NOT append `--status ...` positional arguments.

Agent commands SHALL receive the relevant prompt content via `TRUDGER_PROMPT` (solve) or `TRUDGER_REVIEW_PROMPT` (review); the non-relevant prompt env var SHALL be unset.

Before spawning configured commands/hooks, the system SHALL truncate individual `TRUDGER_*` environment variable values that exceed 64 KiB (bytes) at a UTF-8 character boundary to reduce the risk of command spawn failures (E2BIG). When truncation occurs, the system SHALL print a warning and (when `log_path` is configured) log an `env_truncate` transition.

#### Scenario: Status update command receives target status via env
- **WHEN** Trudger executes `commands.task_update_status`
- **THEN** it sets `TRUDGER_TARGET_STATUS` to the desired status value
- **AND** it does not pass `--status` positional arguments

### Requirement: Review loop limit exhaustion
When the review step indicates the task is neither `closed` nor `blocked`, the system SHALL retry solve + review until `review_loop_limit` is exhausted. If the task is still not `closed` after exhausting the limit, the system SHALL mark the task `blocked` via `commands.task_update_status` and invoke `hooks.on_requires_human`.

#### Scenario: Review loop exhaustion marks blocked
- **GIVEN** a task never becomes `closed` after review
- **WHEN** `review_loop_limit` is exhausted
- **THEN** Trudger updates the task status to `blocked` via `commands.task_update_status`
- **AND** it invokes `hooks.on_requires_human`
