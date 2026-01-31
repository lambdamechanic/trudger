# trudger Specification

## Purpose
Trudger is a generic task-processing loop for beads-backed task systems (for example `br` or `bd`). It selects ready tasks, runs Codex solve + review prompts, and verifies tasks are closed or escalated for human input.

## Requirements
### Requirement: Trudger entrypoint
The system SHALL provide a root-level executable script named `./trudger` that orchestrates task selection and Codex execution.

#### Scenario: Script is executable
- **WHEN** a user runs `./trudger`
- **THEN** the script starts without requiring an explicit shell invocation

### Requirement: Prompt file presence
The script SHALL verify that `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist before starting work and exit with a clear error message if either is missing.

#### Scenario: Prompt file missing
- **GIVEN** one or both prompt files do not exist
- **WHEN** `./trudger` starts
- **THEN** the script exits with a clear error indicating the missing prompt file path

### Requirement: Configuration loading
The script SHALL load configuration from `~/.config/trudger.yml` and exit with a clear error if the file is missing.

#### Scenario: Missing config file
- **WHEN** `~/.config/trudger.yml` does not exist
- **THEN** the script exits non-zero and prints bootstrap instructions for sample configs

### Requirement: Configuration validation
The script SHALL require `codex_command`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `hooks.on_completed`, `hooks.on_requires_human`, `review_loop_limit`, and `log_path` to be present and non-empty.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the script exits non-zero with a clear error naming the missing field

### Requirement: Configuration parsing
The script SHALL parse `~/.config/trudger.yml` using `yq`, treat null values as validation errors, and exit with a clear parse error if YAML decoding fails.

#### Scenario: Null config value
- **WHEN** a required config value is present but null
- **THEN** the script exits non-zero with a clear error naming the field

#### Scenario: Config parse failure
- **GIVEN** the config file exists but contains invalid YAML
- **WHEN** the script loads configuration
- **THEN** the script exits non-zero with a clear parse error that names the config path

### Requirement: Unknown config keys
The script SHALL emit a warning for unknown top-level config keys and continue.

#### Scenario: Unknown config key
- **WHEN** the config contains an unknown top-level key
- **THEN** the script prints a warning naming the key and continues startup

### Requirement: Task selection
The script SHALL select the next task by running the configured `commands.next_task` command, then evaluate readiness by running `commands.task_status`, and process one task per outer loop iteration.

#### Scenario: No selectable tasks
- **WHEN** `commands.next_task` returns an empty result or exits with code 1
- **THEN** the script exits with status 0

#### Scenario: Task not ready is skipped
- **WHEN** `commands.next_task` returns a task whose `commands.task_status` result is not `ready` or `open`
- **THEN** the script skips it and retries up to `TRUDGER_SKIP_NOT_READY_LIMIT` before idling

### Requirement: Codex prompt execution
For each selected task, the script SHALL start a Codex exec session using the contents of `~/.codex/prompts/trudge.md` with `$ARGUMENTS` replaced by the task id and `$TASK_SHOW` replaced by the output of `commands.task_show`, then resume the same session with `~/.codex/prompts/trudge_review.md` using the same replacements.

#### Scenario: Codex solve + review
- **WHEN** a task is selected
- **THEN** the script invokes `codex exec` with the rendered trudge prompt
- **AND** the script invokes `codex exec resume --last` with the rendered review prompt

### Requirement: Task show output handling
The script SHALL treat `commands.task_show` output as free-form prompt content and SHALL NOT parse it for control flow decisions.

#### Scenario: Show output is prompt-only
- **GIVEN** `commands.task_show` is configured
- **WHEN** Trudger renders prompts for a task
- **THEN** it passes the show output to Codex without parsing task status

### Requirement: Prompt substitution safety
The script SHALL substitute `$ARGUMENTS` and `$TASK_SHOW` in prompt templates as literal values, preserving special characters like `&` and backslashes without mutation.

#### Scenario: Task show output contains special characters
- **GIVEN** `commands.task_show` returns content containing `&` or backslashes
- **WHEN** Trudger renders a prompt
- **THEN** the rendered prompt includes the content exactly as returned

### Requirement: Hook execution semantics
The script SHALL execute hooks either with `$1`/`${1}` substitution or by prepending the task id as the first argument when no substitution is present.

#### Scenario: Hook uses $1 substitution
- **WHEN** a hook command contains `$1` or `${1}`
- **THEN** the hook is executed via a shell with the task id available as `$1`

#### Scenario: Hook without substitution
- **WHEN** a hook command does not contain `$1` or `${1}`
- **THEN** the task id is passed as the first argument

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

### Requirement: Codex update verification
After the review step, the script SHALL verify that the task has a non-empty status (from `commands.task_status`) and error if status is missing.

#### Scenario: Missing status after review
- **WHEN** the review step completes and `commands.task_status` returns an empty result
- **THEN** the script exits with a non-zero status

### Requirement: Execution logging
When `log_path` is configured, the script SHALL log command start/exit and quit reasons as single-line entries, escaping control characters (newlines, carriage returns, tabs) in logged values. The script SHALL log the full configured command strings and arguments without redaction.

#### Scenario: Command logging includes full command
- **WHEN** a configured command executes
- **THEN** the log entry includes the full command string and arguments

#### Scenario: Log values include control characters
- **WHEN** a logged value contains newlines, carriage returns, or tabs
- **THEN** the log entry replaces them with escaped sequences

### Requirement: Error exit logging
Unhandled errors SHALL be recorded via the same quit path used for explicit exits, and the script SHALL NOT emit a "quit reason" log entry without exiting.

#### Scenario: Unhandled error triggers quit
- **WHEN** an unhandled error occurs
- **THEN** the script logs the quit reason and exits non-zero

### Requirement: Reexec path resolution
After processing a task, the script SHALL re-exec itself using a resolved executable path when available, falling back to the original argv[0], and log the reexec path.

#### Scenario: Reexec uses resolved path
- **WHEN** the script restarts after handling a task
- **THEN** it uses the resolved executable path if available and logs it
