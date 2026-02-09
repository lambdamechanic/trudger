# trudger Spec Delta

## MODIFIED Requirements

### Requirement: Prompt file presence
The system SHALL verify that `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist before starting task-processing work and exit with a clear error message if either is missing.

The system SHALL NOT require prompt files to run `trudger doctor`.

The system SHALL provide a wizard-assisted installation path for the required prompt files when running `trudger wizard`.

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
- **THEN** the system prompts to install the default prompts to `~/.codex/prompts/`
- **AND** when the user accepts, it writes both prompt files and reports the install paths

#### Scenario: Wizard can skip prompt installation
- **GIVEN** one or both prompt files do not exist
- **WHEN** a user runs `trudger wizard`
- **AND** the user declines prompt installation
- **THEN** the system still writes the config file
- **AND** it prints clear follow-up instructions indicating prompts are still required for task-processing mode

#### Scenario: Wizard offers to update differing prompts
- **GIVEN** both prompt files exist
- **AND** one or both prompt files differ from the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **THEN** the system prompts whether to overwrite the existing prompts
- **AND** when the user accepts, it overwrites the differing prompt files and reports the updated paths

## ADDED Requirements

### Requirement: Wizard prompt installation is safe
When `trudger wizard` installs prompts, the system SHALL create `~/.codex/prompts/` if it does not exist and SHALL NOT overwrite existing prompt files without explicit user confirmation.

#### Scenario: Wizard creates prompt directory
- **GIVEN** `~/.codex/prompts/` does not exist
- **WHEN** a user runs `trudger wizard` and accepts prompt installation
- **THEN** the system creates `~/.codex/prompts/` and writes the prompt files

#### Scenario: Wizard refuses to overwrite without confirmation
- **GIVEN** a prompt file exists and differs from the built-in defaults
- **WHEN** a user runs `trudger wizard`
- **AND** the user declines overwrite
- **THEN** the system leaves the existing prompt file unchanged

