## MODIFIED Requirements
### Requirement: Trudger entrypoint
The system SHALL provide a root-level executable script named `./trudger` that orchestrates br task selection and Codex execution.

#### Scenario: Script is executable
- **WHEN** a user runs `./trudger`
- **THEN** the script starts without requiring an explicit shell invocation

### Requirement: Task selection
The script SHALL select the lowest-priority ready br task that has the `trudgeable` label and process one task per outer loop iteration.

#### Scenario: No trudgeable tasks
- **WHEN** no ready tasks with label `trudgeable` are found
- **THEN** the script exits with status 0

### Requirement: Codex prompt execution
For each selected task, the script SHALL start a Codex exec session using the contents of `~/.codex/prompts/trudge.md` with `$ARGUMENTS` replaced by the br id, then resume the same session with `~/.codex/prompts/trudge_review.md` with `$ARGUMENTS` replaced by the br id.

#### Scenario: Codex solve + review
- **WHEN** a task is selected
- **THEN** the script invokes `codex exec` with the rendered trudge prompt
- **AND** the script invokes `codex exec resume --last` with the rendered review prompt

### Requirement: Task closure on success
When the review prompt indicates the task meets acceptance criteria, the script SHALL close the br task and remove the `trudgeable` label.

#### Scenario: Task closed after successful review
- **WHEN** Codex review reports acceptance criteria satisfied
- **THEN** the br task is closed and the `trudgeable` label is removed
