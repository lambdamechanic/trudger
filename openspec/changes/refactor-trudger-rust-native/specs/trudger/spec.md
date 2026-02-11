## ADDED Requirements
### Requirement: Rust entrypoint
The system SHALL provide a native Rust executable built by the project (for example `target/release/trudger`) that orchestrates task selection and agent execution alongside the existing shell implementation.

#### Scenario: Rust entrypoint is executable
- **WHEN** a user runs the release binary (for example `target/release/trudger`)
- **THEN** the program starts without requiring an explicit shell invocation

### Requirement: Rust configuration parsing
The Rust implementation SHALL parse `~/.config/trudger.yml` into a typed configuration schema using a native YAML parser, treat null values as validation errors, and exit with a clear parse error if YAML decoding fails. The Rust implementation SHALL NOT invoke external YAML/JSON parsing utilities (for example `yq` or `jq`) to load the configuration.

#### Scenario: Null config value
- **WHEN** a required config value is present but null
- **THEN** the Rust implementation exits non-zero with a clear error naming the field

#### Scenario: Config parse failure
- **GIVEN** the config file exists but contains invalid YAML
- **WHEN** the Rust implementation loads configuration
- **THEN** it exits non-zero with a clear parse error that names the config path

#### Scenario: Config parsing without external tools
- **GIVEN** `yq` and `jq` are not installed
- **WHEN** the config file is valid
- **THEN** the Rust implementation loads the configuration successfully

### Requirement: Rust in-process loop
The Rust implementation SHALL process tasks in a single in-process loop and SHALL NOT re-exec itself between iterations.

#### Scenario: No re-exec between tasks
- **WHEN** the Rust implementation finishes processing a task
- **THEN** it continues the loop without spawning a new process

### Requirement: Rust hook execution parity
The Rust implementation SHALL execute hooks without positional task arguments and SHALL provide task context via `TRUDGER_*` environment variables, matching the shell implementation.

#### Scenario: Hook receives env vars in Rust
- **WHEN** a hook command executes
- **THEN** the Rust implementation provides `TRUDGER_TASK_ID` and other task context via environment variables
- **AND** no positional task id argument is passed

### Requirement: Rust logging parity
When `log_path` is configured, the Rust implementation SHALL log command start/exit and quit reasons using the same single-line format and control-character escaping rules as the shell implementation.

#### Scenario: Rust log format matches shell
- **WHEN** the Rust implementation logs command execution
- **THEN** the log entries match the shell implementation format and escaping rules
