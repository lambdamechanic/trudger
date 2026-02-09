# trudger Spec Delta

## MODIFIED Requirements

### Requirement: Prompt file presence
The system SHALL verify that `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist before starting task-processing work and exit with a clear error message if either is missing.

The system SHALL NOT require prompt files to run `trudger doctor`.

When running `trudger wizard`, the system SHALL detect the required prompt files and offer a wizard-assisted installation/update path for the default prompts.

#### Scenario: Prompt file missing
- **GIVEN** one or both prompt files do not exist
- **WHEN** task-processing work starts
- **THEN** the system exits with a clear error indicating the missing prompt file path

#### Scenario: Doctor mode does not require prompts
- **GIVEN** one or both prompt files do not exist
- **WHEN** a user runs `trudger doctor`
- **THEN** the system does not error due to missing prompt files

#### Scenario: Wizard offers to install missing prompts
- **GIVEN** one or both prompt files do not exist
- **WHEN** a user runs `trudger wizard`
- **THEN** the system prompts to install the missing prompts to `~/.codex/prompts/` (default Yes)
- **AND** it trims whitespace and treats blank input, `y`, or `yes` (case-insensitive) as acceptance and `n` or `no` (case-insensitive) as decline
- **AND** it reprompts on any other input
- **AND** when the user accepts, it creates `~/.codex/prompts/` if needed and writes any missing prompt files
- **AND** it does not overwrite any existing prompt file as part of this step without a separate overwrite confirmation

#### Scenario: Wizard can skip prompt installation
- **GIVEN** one or both prompt files do not exist
- **WHEN** a user runs `trudger wizard`
- **AND** the user declines prompt installation
- **THEN** the system still writes the config file
- **AND** it prints follow-up instructions that include both required prompt file paths
- **AND** it suggests at least one installation method (for example rerun `trudger wizard` to install prompts, or run `./install.sh` from a repo checkout)

#### Scenario: Wizard offers to update differing prompts
- **GIVEN** one or both prompt files exist
- **AND** at least one existing prompt file differs from the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **THEN** the system prompts whether to overwrite each differing prompt file (default No)
- **AND** it overwrites a prompt file only when the user explicitly confirms the overwrite
- **AND** when it overwrites a prompt file, it reports the updated prompt path

#### Scenario: Wizard does nothing when prompts match defaults
- **GIVEN** both prompt files exist
- **AND** both prompt files match the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **THEN** the system does not prompt to install or update prompt files

## ADDED Requirements

### Requirement: Wizard prompt defaults are embedded
The system SHALL embed the default prompt contents used by `trudger wizard` into the Rust binary at build time, and the wizard SHALL use those embedded defaults when installing or updating prompts. The wizard SHALL NOT require a repository checkout to install prompt files.

#### Scenario: Wizard installs prompts without repo checkout
- **GIVEN** the repository prompt sources are not available on disk
- **WHEN** a user runs `trudger wizard` and accepts prompt installation
- **THEN** the wizard installs the prompt files using embedded defaults

### Requirement: Wizard prompt installation is safe
When `trudger wizard` installs prompts, the system SHALL create `~/.codex/prompts/` if it does not exist.

The system SHALL NOT overwrite an existing prompt file that differs from the built-in defaults without explicit user confirmation. Blank input SHALL default to "keep existing" for overwrite prompts.

When comparing existing prompt content to the built-in defaults, the system SHALL compare normalized text. Normalization SHALL treat `\n`, `\r\n`, and `\r` line endings as equivalent and SHALL ignore a single trailing newline at end-of-file.

When overwriting a prompt file, the system SHALL create a timestamped backup of the existing prompt file before writing the new content.

The system SHALL accept `y` or `yes` (case-insensitive) as explicit overwrite confirmation and SHALL accept blank input, `n`, or `no` (case-insensitive) as "keep existing". The system SHALL reprompt on any other input.

Prompt backups SHALL be created as sibling files in `~/.codex/prompts/` using the naming scheme `{filename}.bak-{timestamp}` where `{timestamp}` is a UTC timestamp formatted as `YYYYMMDDTHHMMSSZ` (for example `20260209T124425Z`). If a backup path already exists, the system SHOULD select a non-colliding sibling path (for example by appending `-2`).

#### Scenario: Wizard creates prompt directory
- **GIVEN** `~/.codex/prompts/` does not exist
- **WHEN** a user runs `trudger wizard` and accepts prompt installation
- **THEN** the system creates `~/.codex/prompts/` and writes the prompt files

#### Scenario: Wizard refuses to overwrite without confirmation
- **GIVEN** a prompt file exists and differs from the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **AND** the user declines overwrite
- **THEN** the system leaves the existing prompt file unchanged

#### Scenario: Wizard backs up prompt before overwrite
- **GIVEN** a prompt file exists and differs from the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **AND** the user explicitly confirms overwrite
- **THEN** the system creates a timestamped backup of the existing prompt file
- **AND** it overwrites the prompt file with the built-in default content

### Requirement: Wizard prompt state detection is strict
When running `trudger wizard`, the system SHALL read any existing required prompt file to determine whether it matches the built-in defaults. If reading or decoding an existing prompt file fails (including non-UTF-8 content), the wizard SHALL exit non-zero, SHALL print a clear error that includes the prompt path and indicates the failing operation, and SHALL NOT write the config file.

#### Scenario: Prompt read/parse failure aborts wizard
- **GIVEN** a required prompt file exists but cannot be read or decoded
- **WHEN** a user runs `trudger wizard`
- **THEN** the wizard exits non-zero and prints an error that includes the failing prompt path
- **AND** the config file is not written

### Requirement: Wizard prompt install/update failures are actionable
If prompt installation or update fails after the user accepts an install/overwrite action, the wizard SHALL exit non-zero, SHALL print a clear error naming the path that failed and indicating the failing operation, and SHALL NOT write the config file.

#### Scenario: Prompt write failure exits non-zero with path
- **GIVEN** the user accepts installing or overwriting prompt files
- **AND** writing a prompt file fails due to an IO or permission error
- **WHEN** the wizard attempts to write the prompt file
- **THEN** the wizard exits non-zero and prints an error that includes the failing prompt path and indicates the failing operation
- **AND** the config file is not written
