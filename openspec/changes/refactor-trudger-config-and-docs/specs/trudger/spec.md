## ADDED Requirements
### Requirement: Configuration loading
The script SHALL load configuration from `~/.config/trudger.yml` and exit with a clear error if the file is missing.

#### Scenario: Missing config file
- **WHEN** `~/.config/trudger.yml` does not exist
- **THEN** the script exits non-zero and prints bootstrap instructions for sample configs

### Requirement: Configuration validation
The script SHALL require `agent_command`, `agent_review_command`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, `hooks.on_requires_human`, `review_loop_limit`, and `log_path` to be present and non-empty.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the script exits non-zero with a clear error naming the missing field

### Requirement: Configuration parsing
The script SHALL parse `~/.config/trudger.yml` using `yq` and treat null values as validation errors.

#### Scenario: Null config value
- **WHEN** a required config value is present but null
- **THEN** the script exits non-zero with a clear error naming the field

### Requirement: Unknown config keys
The script SHALL emit a warning for unknown top-level config keys and continue.

#### Scenario: Unknown config key
- **WHEN** the config contains an unknown top-level key
- **THEN** the script prints a warning naming the key and continues startup

### Requirement: Hook execution semantics
The script SHALL execute hooks without positional task arguments and SHALL provide task context via `TRUDGER_*` environment variables.

#### Scenario: Hook receives env vars
- **WHEN** a hook command executes
- **THEN** it receives `TRUDGER_TASK_ID` and other task context via environment variables
- **AND** no positional task id argument is passed

## RENAMED Requirements
- FROM: `### Requirement: Codex prompt execution`
- TO: `### Requirement: Agent prompt execution`
- FROM: `### Requirement: Codex update verification`
- TO: `### Requirement: Agent update verification`

## MODIFIED Requirements
### Requirement: Task selection
The script SHALL select the next task by running the configured `commands.next_task` command, then evaluate readiness by running `commands.task_status`, and process one task per outer loop iteration.

#### Scenario: No selectable tasks
- **WHEN** `commands.next_task` returns an empty result or exits with code 1
- **THEN** the script exits with status 0

#### Scenario: Task not ready is skipped
- **WHEN** `commands.next_task` returns a task whose `commands.task_status` result is not `ready` or `open`
- **THEN** the script skips it and retries up to `TRUDGER_SKIP_NOT_READY_LIMIT` before idling

### Requirement: Agent prompt execution
For each selected task, the script SHALL execute the configured `agent_command` for the solve step and `agent_review_command` for the review step. The script SHALL load prompt content from `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` without performing `$ARGUMENTS` or `$TASK_SHOW` substitutions, and SHALL provide the prompt content via `TRUDGER_PROMPT` (solve) and `TRUDGER_REVIEW_PROMPT` (review) environment variables alongside task context (`TRUDGER_*`).

#### Scenario: Agent solve + review
- **WHEN** a task is selected
- **THEN** the script invokes `agent_command` with the trudge prompt content
- **AND** the script invokes `agent_review_command` with the review prompt content

#### Scenario: Prompt context via env vars
- **WHEN** the agent commands run
- **THEN** task context is provided via `TRUDGER_*` environment variables
- **AND** the relevant prompt env var is set (`TRUDGER_PROMPT` for solve, `TRUDGER_REVIEW_PROMPT` for review) while the other is unset
- **AND** prompt templates are not substituted by Trudger

### Requirement: Task show output handling
The script SHALL treat `commands.task_show` output as free-form prompt content and SHALL NOT parse it for control flow decisions. The script SHALL provide the output via `TRUDGER_TASK_SHOW` for agent commands and hooks.

#### Scenario: Show output is prompt-only
- **GIVEN** `commands.task_show` is configured
- **WHEN** Trudger renders prompts for a task
- **THEN** it provides the show output via `TRUDGER_TASK_SHOW` without parsing task status

### Requirement: Task closure on success
When the review prompt indicates the task is closed, the script SHALL invoke `hooks.on_completed`.

#### Scenario: Task closed after successful review
- **WHEN** `commands.task_status` returns `closed` after the review step
- **THEN** `hooks.on_completed` is executed for that task

### Requirement: Requires-human escalation
When the review prompt indicates the task is still open, the script SHALL invoke `hooks.on_requires_human`.

#### Scenario: Task still open after review
- **WHEN** `commands.task_status` does not return `closed` after the review step
- **THEN** `hooks.on_requires_human` is executed for that task

### Requirement: Agent update verification
After the review step, the script SHALL verify that the task has a non-empty status (from `commands.task_status`) and error if status is missing.

#### Scenario: Missing status after review
- **WHEN** the review step completes and `commands.task_status` returns an empty result
- **THEN** the script exits with a non-zero status
