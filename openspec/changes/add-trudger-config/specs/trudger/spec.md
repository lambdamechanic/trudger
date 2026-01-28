## ADDED Requirements
### Requirement: Trudger configuration file
The system SHALL read configuration from `~/.config/trudger.yml` on startup to determine Codex invocation, task selection, labels, and hooks. If the file is missing, the system SHALL emit a warning and continue with default settings. The system SHALL NOT use `TRUDGER_*` environment variables for configuration.

#### Scenario: Config missing
- **WHEN** `~/.config/trudger.yml` does not exist
- **THEN** the system warns about the missing file
- **AND** continues using default settings

#### Scenario: Env vars ignored
- **GIVEN** `TRUDGER_*` environment variables are set
- **WHEN** configuration is loaded
- **THEN** the configuration file values are used

## MODIFIED Requirements
### Requirement: Task selection
The script SHALL select the next task by running the configured next-task command and using the first whitespace-delimited token of its output as the task id. If no next-task command is configured, the script SHALL default to `br ready --json --sort priority --limit 1`, filtering by the configured trudgeable label when one is set.

#### Scenario: Custom next-task command
- **GIVEN** a configured next-task command
- **WHEN** the command outputs a task id
- **THEN** the script selects that id as the next task

### Requirement: Codex prompt execution
For each selected task, the script SHALL start a Codex exec session using the contents of `~/.codex/prompts/trudge.md` with `$ARGUMENTS` replaced by the br id, then resume the same session with `~/.codex/prompts/trudge_review.md` with `$ARGUMENTS` replaced by the br id, using the configured Codex command line for invocation.

#### Scenario: Configured Codex command
- **GIVEN** a configured Codex command line
- **WHEN** a task is selected
- **THEN** the script uses it for the solve invocation
- **AND** uses the same command line with `resume --last` appended for the review invocation

### Requirement: Task closure on success
When the review step results in the task being closed, the script SHALL invoke the configured completion hook (if provided) by executing the hook command with the task id as the first argument and the remaining configured tokens as subsequent arguments. If no completion hook is configured, the script SHALL remove the configured trudgeable label when it is set.

#### Scenario: Completion hook configured
- **GIVEN** a configured completion hook command
- **WHEN** the task is closed after review
- **THEN** the hook is executed as `<command> <task_id> <extra args>`

#### Scenario: No completion hook
- **GIVEN** no completion hook is configured
- **WHEN** the task is closed after review
- **THEN** the configured trudgeable label is removed if present

### Requirement: Requires-human escalation
When the review step indicates human input is required, the script SHALL invoke the configured requires-human hook (if provided) by executing the hook command with the task id as the first argument and the remaining configured tokens as subsequent arguments. If no requires-human hook is configured, the script SHALL remove the configured trudgeable label (when set) and add the configured requires-human label (when set). Human-input requirement is indicated by the configured requires-human label when set, or by the task remaining open after review when no requires-human label is configured.

#### Scenario: Requires-human hook configured
- **GIVEN** a configured requires-human hook command
- **WHEN** the requires-human condition is detected after review
- **THEN** the hook is executed as `<command> <task_id> <extra args>`

#### Scenario: No requires-human hook
- **GIVEN** no requires-human hook is configured
- **WHEN** the requires-human condition is detected after review
- **THEN** the configured labels are updated when present

### Requirement: Codex update verification
After the review step, the script SHALL verify that either the task is closed or the requires-human condition is detected, and exit with a non-zero status if neither occurred.

#### Scenario: Missing update is an error
- **WHEN** the review step completes without a closed task or requires-human condition
- **THEN** the script exits with a non-zero status
