# trudger Specification

## Purpose
TBD - created by archiving change add-trudger. Update Purpose after archive.
## Requirements
### Requirement: Trudger entrypoint
The system SHALL provide a root-level executable script named `./trudger` that orchestrates bd task selection and Codex execution.

#### Scenario: Script is executable
- **WHEN** a user runs `./trudger`
- **THEN** the script starts without requiring an explicit shell invocation

### Requirement: Prompt file presence
The script SHALL verify that `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist before starting work and exit with a clear error message if either is missing.

#### Scenario: Prompt file missing
- **GIVEN** one or both prompt files do not exist
- **WHEN** `./trudger` starts
- **THEN** the script exits with a clear error indicating the missing prompt file path

### Requirement: Task selection
The script SHALL select the lowest-priority ready bd task that has the `trudgeable` label and process one task per outer loop iteration.

#### Scenario: No trudgeable tasks
- **WHEN** no ready tasks with label `trudgeable` are found
- **THEN** the script exits with status 0

### Requirement: Codex prompt execution
For each selected task, the script SHALL start a Codex exec session using the contents of `~/.codex/prompts/trudge.md` with `$ARGUMENTS` replaced by the bd id, then resume the same session with `~/.codex/prompts/trudge_review.md` with `$ARGUMENTS` replaced by the bd id.

#### Scenario: Codex solve + review
- **WHEN** a task is selected
- **THEN** the script invokes `codex exec` with the rendered trudge prompt
- **AND** the script invokes `codex exec resume --last` with the rendered review prompt

### Requirement: Task closure on success
When the review prompt indicates the task meets acceptance criteria, the script SHALL close the bd task and remove the `trudgeable` label.

#### Scenario: Task closed after successful review
- **WHEN** Codex review reports acceptance criteria satisfied
- **THEN** the bd task is closed and the `trudgeable` label is removed

### Requirement: Requires-human escalation
When the review prompt indicates human input is required, the script SHALL remove `trudgeable` and add `requires-human`.

#### Scenario: Requires-human handling
- **WHEN** Codex review reports that human input is required
- **THEN** the task receives a comment and notes update
- **AND** the `trudgeable` label is removed
- **AND** the `requires-human` label is added

### Requirement: Codex update verification
After the review step, the script SHALL verify that the task was either closed or labeled `requires-human` (with `trudgeable` removed) and exit with a non-zero status if neither occurred.

#### Scenario: Missing update is an error
- **WHEN** the review step completes without closing the task or applying requires-human escalation
- **THEN** the script exits with a non-zero status

