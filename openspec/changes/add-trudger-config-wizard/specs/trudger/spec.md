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
The wizard SHALL set `review_loop_limit` and `log_path` using embedded defaults when generating a new configuration or when those keys are missing from an existing config. The wizard SHALL NOT prompt for these fields. When the target config already defines `review_loop_limit` or `log_path`, the wizard SHALL preserve the existing value.

#### Scenario: Defaults are applied when missing
- **GIVEN** an existing config file is missing `review_loop_limit` or `log_path` (or no config file exists)
- **WHEN** the wizard writes a configuration file
- **THEN** the output includes `review_loop_limit` and `log_path` populated from embedded defaults

#### Scenario: Existing values are preserved
- **GIVEN** an existing config file defines `review_loop_limit` and `log_path`
- **WHEN** the user runs `trudger wizard` and completes the selections
- **THEN** the output preserves the existing values for `review_loop_limit` and `log_path`

### Requirement: Wizard per-key replacement prompts
When a config file exists at the target path and the selected templates would change one or more known Trudger config keys (`agent_command`, `agent_review_command`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, `hooks.on_requires_human`, `hooks.on_doctor_setup`), the wizard SHALL show a per-key diff (current value and proposed value) for each differing key and prompt the user to keep the current value or replace it with the proposed value. The wizard SHALL default to keeping the current value for each prompt.

#### Scenario: Per-key diff and replacement
- **GIVEN** an existing config file differs from the selected templates for one or more known keys
- **WHEN** the user runs `trudger wizard` and selects templates
- **THEN** the wizard shows the current and proposed values for each differing key
- **AND** the wizard prompts whether to keep or replace each differing key
- **AND** the default choice is to keep the current value

### Requirement: Unknown/custom keys are commented out
When an existing config file contains unknown/custom keys at top-level or under `commands`/`hooks`, the wizard SHALL include those keys and their original values in the generated config as commented YAML preceded by a warning comment and SHALL warn the user that the keys were commented out.

#### Scenario: Unknown keys preserved as comments
- **GIVEN** an existing config file contains unknown/custom keys at top-level or under `commands`/`hooks`
- **WHEN** the wizard writes a new configuration file
- **THEN** the output config includes those keys as commented YAML preceded by a warning comment
- **AND** the wizard emits a warning to the user that unknown/custom keys were commented out

### Requirement: Invalid existing config is overwritten with backup
If a config file exists at the target path but cannot be parsed as YAML, the wizard SHALL warn the user that the existing config could not be parsed and will be replaced. After validating the generated configuration successfully, the wizard SHALL create a timestamped backup of the existing file before overwriting it. The wizard SHALL NOT attempt per-key merge or unknown-key preservation from the invalid config.

#### Scenario: Invalid existing config is overwritten
- **GIVEN** a config file exists at the target path but contains invalid YAML
- **WHEN** the user completes the wizard
- **THEN** the wizard warns that the existing config could not be parsed and will be backed up and overwritten
- **AND** after successful validation, it creates a timestamped backup before writing the new config
- **AND** it writes the new config from the selected templates and embedded defaults

### Requirement: Wizard output validation
The wizard SHALL validate the generated configuration using existing Trudger config parsing and validation rules and SHALL NOT write the config (or create a backup) when validation fails.

#### Scenario: Validation failure prevents write
- **GIVEN** the wizard generates a configuration that fails Trudger config validation
- **WHEN** the wizard attempts to write the config
- **THEN** it exits non-zero and does not overwrite the existing config file

## MODIFIED Requirements
### Requirement: Configuration loading
The system SHALL load configuration from `~/.config/trudger.yml` by default and SHALL allow overriding the config path via `-c/--config PATH`. The system SHALL exit with a clear error if the selected config file is missing.

#### Scenario: Missing default config prints wizard instructions
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints instructions to run `trudger wizard`
- **AND** it mentions `trudger wizard --config PATH` for generating a config at a non-default path

#### Scenario: Missing explicit config prints missing-path error
- **GIVEN** a user provides `-c/--config PATH`
- **AND** PATH does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints a clear missing config error that includes PATH
