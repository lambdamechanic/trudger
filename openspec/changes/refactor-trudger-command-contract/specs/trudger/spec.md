## RENAMED Requirements
- FROM: `### Requirement: Codex prompt execution`
- TO: `### Requirement: Agent prompt execution`
- FROM: `### Requirement: Codex update verification`
- TO: `### Requirement: Agent update verification`

## ADDED Requirements
### Requirement: Command execution environment
The system SHALL execute configured commands and hooks without positional task arguments and SHALL provide task context via environment variables. When a task context exists, the system SHALL set `TRUDGER_TASK_ID`. After task show output is available, the system SHALL set `TRUDGER_TASK_SHOW`. After task status is available, the system SHALL set `TRUDGER_TASK_STATUS`. The system SHALL always set `TRUDGER_CONFIG_PATH` to the active config path. Agent commands SHALL receive the relevant prompt content via `TRUDGER_PROMPT` (solve) or `TRUDGER_REVIEW_PROMPT` (review); the non-relevant prompt env var SHALL be unset.

#### Scenario: Command environment provided
- **WHEN** Trudger executes a configured command or hook
- **THEN** it passes task context via `TRUDGER_*` environment variables
- **AND** it does not pass the task id as a positional argument

## MODIFIED Requirements
### Requirement: Configuration validation
The script SHALL require `agent_command`, `agent_review_command`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `hooks.on_completed`, `hooks.on_requires_human`, `review_loop_limit`, and `log_path` to be present and non-empty.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the script exits non-zero with a clear error naming the missing field

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

### Requirement: Agent update verification
After the review step, the script SHALL verify that the task has a non-empty status (from `commands.task_status`) and error if status is missing.

#### Scenario: Missing status after review
- **WHEN** the review step completes and `commands.task_status` returns an empty result
- **THEN** the script exits with a non-zero status
