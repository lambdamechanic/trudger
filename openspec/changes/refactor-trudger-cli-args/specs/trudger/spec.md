## ADDED Requirements
### Requirement: Manual task ids via -t
The system SHALL accept manual task ids via `-t/--task` options. The `-t/--task` option MAY be provided multiple times, and each value MAY contain comma-separated task ids. The system SHALL process the manual task ids in the order specified and SHALL process them before selecting tasks via `commands.next_task`.

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

### Requirement: Default mode is task-processing
When no subcommand is provided, the system SHALL run task-processing mode (the normal task loop).

#### Scenario: No args runs the task loop
- **WHEN** a user runs `trudger` with no arguments
- **THEN** the system starts task-processing mode

### Requirement: Positional task ids are rejected
The system SHALL NOT accept positional task ids. If unexpected positional arguments are provided in task-processing mode, the system SHALL exit non-zero with a clear migration error instructing the user to use `-t/--task`.

#### Scenario: Positional task id errors
- **WHEN** a user runs `trudger tr-1`
- **THEN** the system exits non-zero with an error instructing the user to use `-t/--task`

### Requirement: Doctor mode rejects manual tasks
When running `trudger doctor`, the system SHALL reject `-t/--task` with a clear error.

#### Scenario: Doctor mode task flag errors
- **WHEN** a user runs `trudger doctor -t tr-1`
- **THEN** the system exits non-zero with an error indicating `-t/--task` is not supported in doctor mode
