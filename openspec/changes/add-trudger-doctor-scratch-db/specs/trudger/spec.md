## ADDED Requirements
### Requirement: Doctor entrypoint
The system SHALL provide a `trudger doctor` mode that initializes a scratch task database before running any doctor checks.

#### Scenario: Doctor mode uses temporary scratch directory
- **WHEN** a user runs `trudger doctor`
- **THEN** the system creates a temporary scratch directory
- **AND** it executes `hooks.on_doctor_setup` from the invocation working directory with `TRUDGER_DOCTOR_SCRATCH_DIR` set to the scratch directory

#### Scenario: Doctor mode completes after setup
- **GIVEN** `hooks.on_doctor_setup` exits with code 0
- **WHEN** a user runs `trudger doctor`
- **THEN** the system exits with code 0 after setup completes

### Requirement: Doctor setup hook
The system SHALL support a `hooks.on_doctor_setup` command used to initialize a scratch task database for doctor mode, and it SHALL execute the hook before any doctor checks.

#### Scenario: Doctor setup runs before checks
- **WHEN** Trudger runs in doctor mode
- **THEN** it executes `hooks.on_doctor_setup` before any doctor checks

### Requirement: Doctor check working directory
When doctor mode runs doctor checks, the system SHALL execute those checks with the scratch directory as the working directory.

#### Scenario: Checks run from scratch directory
- **GIVEN** doctor checks are executed
- **WHEN** Trudger runs in doctor mode after setup
- **THEN** doctor checks are executed from the scratch directory as the working directory

### Requirement: Doctor setup environment
When doctor mode runs, the system SHALL set `TRUDGER_DOCTOR_SCRATCH_DIR` to the scratch directory path, SHALL set `TRUDGER_CONFIG_PATH`, and SHALL ensure `TRUDGER_TASK_ID`, `TRUDGER_TASK_SHOW`, `TRUDGER_TASK_STATUS`, `TRUDGER_PROMPT`, and `TRUDGER_REVIEW_PROMPT` are unset before invoking `hooks.on_doctor_setup`.

#### Scenario: Doctor setup env vars
- **WHEN** Trudger invokes `hooks.on_doctor_setup`
- **THEN** `TRUDGER_DOCTOR_SCRATCH_DIR` is set
- **AND** `TRUDGER_CONFIG_PATH` is set
- **AND** `TRUDGER_TASK_ID`, `TRUDGER_TASK_SHOW`, `TRUDGER_TASK_STATUS`, `TRUDGER_PROMPT`, and `TRUDGER_REVIEW_PROMPT` are unset

### Requirement: Doctor hook validation
When running doctor mode, the system SHALL require `hooks.on_doctor_setup` to be present and non-empty.

#### Scenario: Doctor hook missing
- **GIVEN** `hooks.on_doctor_setup` is missing or empty
- **WHEN** Trudger runs in doctor mode
- **THEN** it exits non-zero with a clear error naming `hooks.on_doctor_setup`

### Requirement: Doctor setup failure handling
If `hooks.on_doctor_setup` exits non-zero, the system SHALL exit non-zero and print a clear error that includes the hook name and exit code.

#### Scenario: Doctor setup failure aborts
- **GIVEN** `hooks.on_doctor_setup` exits non-zero
- **WHEN** Trudger runs in doctor mode
- **THEN** it exits non-zero and prints an error naming `hooks.on_doctor_setup` and its exit code to stderr

### Requirement: Doctor scratch cleanup
The system SHALL clean up the temporary scratch directory created for doctor mode on both success and failure.

#### Scenario: Doctor cleanup on success
- **GIVEN** `hooks.on_doctor_setup` exits with code 0
- **WHEN** Trudger runs in doctor mode
- **THEN** it removes the temporary scratch directory before exiting

#### Scenario: Doctor cleanup on failure
- **GIVEN** `hooks.on_doctor_setup` exits non-zero
- **WHEN** Trudger runs in doctor mode
- **THEN** it removes the temporary scratch directory before exiting

## MODIFIED Requirements
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
