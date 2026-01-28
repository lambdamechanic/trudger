## ADDED Requirements
### Requirement: Trudger configuration bootstrap
The system SHALL require a configuration file at `~/.config/trudger.yml` before starting work. If the file is missing, the system SHALL emit curl commands for each sample configuration (including the header docs describing each option) that install to that path and exit with a non-zero status.

#### Scenario: Config missing
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** `./trudger` starts
- **THEN** it prints curl commands for each sample config that write to `~/.config/trudger.yml`
- **AND** it includes the header docs describing each sample config
- **AND** it exits non-zero

#### Scenario: Config present
- **GIVEN** `~/.config/trudger.yml` exists
- **WHEN** `./trudger` starts
- **THEN** it loads settings from the file and continues execution

## MODIFIED Requirements
### Requirement: Task selection
The script SHALL select the next task by running the configured next-task command and using the first whitespace-delimited token of its output as the task id. The script SHALL NOT fall back to `bd ready` when no next-task command is configured.

#### Scenario: Custom next-task command
- **GIVEN** a configured next-task command
- **WHEN** the command outputs a task id
- **THEN** the script selects that id as the next task

#### Scenario: Missing next-task command is an error
- **GIVEN** no next-task command is configured
- **WHEN** `./trudger` starts
- **THEN** the script exits non-zero with a clear error

### Requirement: Task closure on success
When the review step results in the task being closed, the script SHALL invoke the configured completion hook by executing the hook command with the task id as the first argument and the remaining configured tokens as subsequent arguments. The script SHALL NOT perform label updates itself.

#### Scenario: Completion hook configured
- **GIVEN** a configured completion hook command
- **WHEN** the task is closed after review
- **THEN** the hook is executed as `<command> <task_id> <extra args>`

#### Scenario: Completion hook missing
- **GIVEN** no completion hook is configured
- **WHEN** the task is closed after review
- **THEN** the script exits non-zero with a clear error

### Requirement: Requires-human escalation
When the review step indicates human input is required, the script SHALL invoke the configured requires-human hook by executing the hook command with the task id as the first argument and the remaining configured tokens as subsequent arguments. The script SHALL NOT perform label updates itself. Human-input requirement is detected when the task remains open after review.

#### Scenario: Requires-human hook configured
- **GIVEN** a configured requires-human hook command
- **WHEN** the requires-human condition is detected after review
- **THEN** the hook is executed as `<command> <task_id> <extra args>`

#### Scenario: Requires-human hook missing
- **GIVEN** no requires-human hook is configured
- **WHEN** the requires-human condition is detected after review
- **THEN** the script exits non-zero with a clear error
