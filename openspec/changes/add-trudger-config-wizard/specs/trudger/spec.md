## ADDED Requirements
### Requirement: Configuration wizard
The system SHALL provide a `trudger wizard` command that interactively builds a Trudger configuration by selecting an agent template and a tracking template. The wizard SHALL write the resulting configuration to the active config path and SHALL overwrite any existing file at that path. When overwriting an existing file, the wizard SHALL create a timestamped backup before writing the new config.

#### Scenario: Wizard writes config
- **WHEN** the user runs `trudger wizard` and completes the selections
- **THEN** Trudger writes the configuration to `~/.config/trudger.yml` (or the `--config` path when provided)
- **AND** any existing config file at that path is overwritten
- **AND** a timestamped backup is created when an existing config file is overwritten

#### Scenario: Wizard creates parent directory
- **GIVEN** the parent directory for the target config path does not exist
- **WHEN** the user completes the wizard
- **THEN** Trudger creates the parent directory before writing the config

#### Scenario: Wizard requires interactive terminal
- **GIVEN** stdin or stdout is not a TTY
- **WHEN** the user runs `trudger wizard`
- **THEN** the wizard exits non-zero with a clear error indicating it requires an interactive terminal

### Requirement: Wizard template sources
The wizard SHALL load agent and tracking templates from data files stored in the repository and embedded into the binary at build time; the wizard SHALL NOT hard-code command templates in source code.

#### Scenario: Templates are embedded
- **GIVEN** the Trudger binary is installed
- **WHEN** the wizard runs
- **THEN** it uses the embedded template data to populate choices without reading external template files

### Requirement: Wizard template availability
The wizard SHALL provide agent template choices named `codex`, `claude`, and `pi`, and tracking template choices named `br-next-task` and `bd-labels`.

#### Scenario: Required templates are available
- **WHEN** the wizard displays agent and tracking template options
- **THEN** it includes `codex`, `claude`, and `pi` agent templates
- **AND** it includes `br-next-task` and `bd-labels` tracking templates

### Requirement: Wizard default values
The wizard SHALL set `review_loop_limit` and `log_path` using embedded defaults and SHALL NOT prompt for these fields.

#### Scenario: Defaults are applied
- **WHEN** the wizard writes a configuration file
- **THEN** the output includes `review_loop_limit` and `log_path` populated from embedded defaults

### Requirement: Wizard per-key replacement prompts
When a config file exists at the target path and the selected templates would change one or more known Trudger config keys (`agent_command`, `agent_review_command`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `commands.reset_task`, `hooks.on_completed`, `hooks.on_requires_human`, `review_loop_limit`, `log_path`), the wizard SHALL show a per-key diff (current value and proposed value) for each differing key and prompt the user to keep the current value or replace it with the proposed value.

#### Scenario: Per-key diff and replacement
- **GIVEN** an existing config file differs from the selected templates for one or more known keys
- **WHEN** the user runs `trudger wizard` and selects templates
- **THEN** the wizard shows the current and proposed values for each differing key
- **AND** the wizard prompts whether to keep or replace each differing key

### Requirement: Unknown/custom keys are commented out
When an existing config file contains unknown/custom top-level keys, the wizard SHALL include those keys in the generated config as commented YAML preceded by a warning comment and SHALL warn the user that the keys were commented out.

#### Scenario: Unknown keys preserved as comments
- **GIVEN** an existing config file contains unknown/custom top-level keys
- **WHEN** the wizard writes a new configuration file
- **THEN** the output config includes those keys as commented YAML preceded by a warning comment
- **AND** the wizard emits a warning to the user that unknown/custom keys were commented out

### Requirement: Wizard output validation
The wizard SHALL validate the generated configuration using existing Trudger config parsing and validation rules and SHALL NOT write the config (or create a backup) when validation fails.

#### Scenario: Validation failure prevents write
- **GIVEN** the wizard generates a configuration that fails Trudger config validation
- **WHEN** the wizard attempts to write the config
- **THEN** it exits non-zero and does not overwrite the existing config file

## MODIFIED Requirements
### Requirement: Configuration loading
The system SHALL load configuration from `~/.config/trudger.yml` and exit with a clear error if the file is missing.

#### Scenario: Missing config file
- **WHEN** `~/.config/trudger.yml` does not exist
- **THEN** the system exits non-zero and prints instructions to run `trudger wizard`
