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

### Requirement: Configured task commands
The system SHALL load task command configuration from `~/.config/trudger.yml` and use it for task selection, show, and update operations. The system SHALL NOT invoke `br` directly; any `br` usage must be expressed in configured commands.

#### Scenario: Commands configured
- **GIVEN** task commands are configured
- **WHEN** Trudger runs
- **THEN** it uses the configured commands for selection, show, and update

#### Scenario: Command missing
- **GIVEN** a required task command is missing
- **WHEN** Trudger runs
- **THEN** it exits non-zero with a clear error

## MODIFIED Requirements
### Requirement: Task selection
The script SHALL select the next task by running the configured `commands.next_task` command and using the first whitespace-delimited token of its output as the task id. The task id SHALL be provided to other configured commands and hooks via `TRUDGER_TASK_ID`.

#### Scenario: Custom next-task command
- **GIVEN** a configured `commands.next_task` command
- **WHEN** the command outputs a task id
- **THEN** the script selects that id as the next task

#### Scenario: No tasks available
- **GIVEN** `commands.next_task` exits with status 1
- **WHEN** `./trudger` runs
- **THEN** the script exits with status 0

#### Scenario: Next-task command failure
- **GIVEN** `commands.next_task` exits with a non-zero status other than 1
- **WHEN** `./trudger` runs
- **THEN** the script exits non-zero with a clear error

### Requirement: Task show command
The script SHALL obtain task state by executing `commands.task_show` with `TRUDGER_TASK_ID` set in the environment and no positional task arguments. The show command output is treated as free-form text and provided to the agent; Trudger MUST NOT parse or validate it.

#### Scenario: Show command output
- **GIVEN** `commands.task_show` is configured
- **WHEN** Trudger needs task state
- **THEN** it executes the command with `TRUDGER_TASK_ID` set and no positional task id
- **AND** it provides the output to the agent without parsing

### Requirement: Task update command
Before running the solve prompt, the script SHALL execute `commands.task_update_status` with `TRUDGER_TASK_ID` set in the environment and no positional task arguments. The update command output is ignored.

#### Scenario: Update command execution
- **GIVEN** `commands.task_update_status` is configured
- **WHEN** Trudger begins work on a task
- **THEN** it executes the command with `TRUDGER_TASK_ID` set and no positional task id

### Requirement: Task closure on success
When the review step results in the task being closed, the script SHALL invoke the configured completion hook with `TRUDGER_TASK_ID` set in the environment and no positional task arguments. The script SHALL NOT perform label updates itself.

#### Scenario: Completion hook configured
- **GIVEN** a configured completion hook command
- **WHEN** the task is closed after review
- **THEN** the hook is executed with `TRUDGER_TASK_ID` set and no positional task id

### Requirement: Requires-human escalation
When the review step indicates human input is required, the script SHALL invoke the configured requires-human hook with `TRUDGER_TASK_ID` set in the environment and no positional task arguments. The script SHALL NOT perform label updates itself. Human-input requirement is detected when the task remains open after review.

#### Scenario: Requires-human hook configured
- **GIVEN** a configured requires-human hook command
- **WHEN** the requires-human condition is detected after review
- **THEN** the hook is executed with `TRUDGER_TASK_ID` set and no positional task id
