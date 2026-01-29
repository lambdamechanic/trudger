## ADDED Requirements
### Requirement: Configuration loading
The script SHALL load configuration from `~/.config/trudger.yml` and exit with a clear error if the file is missing.

#### Scenario: Missing config file
- **WHEN** `~/.config/trudger.yml` does not exist
- **THEN** the script exits non-zero and prints bootstrap instructions for sample configs

### Requirement: Configuration validation
The script SHALL require `codex_command`, `commands.next_task`, `commands.task_show`, `commands.task_update_in_progress`, `hooks.on_completed`, `hooks.on_requires_human`, `review_loop_limit`, and `log_path` to be present and non-empty.

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
The script SHALL execute hooks either with `$1`/`${1}` substitution or by prepending the task id as the first argument when no substitution is present.

#### Scenario: Hook uses $1 substitution
- **WHEN** a hook command contains `$1` or `${1}`
- **THEN** the hook is executed via a shell with the task id available as `$1`

#### Scenario: Hook without substitution
- **WHEN** a hook command does not contain `$1` or `${1}`
- **THEN** the task id is passed as the first argument

## MODIFIED Requirements
### Requirement: Task selection
The script SHALL select the next task by running the configured `commands.next_task` command and process one task per outer loop iteration.

#### Scenario: No selectable tasks
- **WHEN** `commands.next_task` returns an empty result or exits with code 1
- **THEN** the script exits with status 0

#### Scenario: Task not ready is skipped
- **WHEN** `commands.next_task` returns a task that is not `ready` or `open`
- **THEN** the script skips it and retries up to `TRUDGER_SKIP_NOT_READY_LIMIT` before idling

### Requirement: Task closure on success
When the review prompt indicates the task is closed, the script SHALL invoke `hooks.on_completed`.

#### Scenario: Task closed after successful review
- **WHEN** the task status is `closed` after the review step
- **THEN** `hooks.on_completed` is executed for that task

### Requirement: Requires-human escalation
When the review prompt indicates the task is still open, the script SHALL invoke `hooks.on_requires_human`.

#### Scenario: Task still open after review
- **WHEN** the task status is not `closed` after the review step
- **THEN** `hooks.on_requires_human` is executed for that task

### Requirement: Codex update verification
After the review step, the script SHALL verify that the task has a non-empty status and error if status is missing.

#### Scenario: Missing status after review
- **WHEN** the review step completes and the task status is empty or missing
- **THEN** the script exits with a non-zero status
