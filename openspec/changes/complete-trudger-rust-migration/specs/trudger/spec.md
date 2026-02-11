## ADDED Requirements
### Requirement: Rust is the canonical implementation
The system SHALL treat the Rust implementation as the canonical Trudger runtime.

#### Scenario: Rust is the default runtime
- **WHEN** a user runs Trudger via the supported entrypoint(s)
- **THEN** Trudger executes the Rust implementation

### Requirement: Bash entrypoint is a shim
If a repository-root `./trudger` entrypoint is provided, it SHALL be a minimal shim that delegates to the Rust implementation and SHALL NOT contain the task-processing loop logic.

#### Scenario: Bash shim delegates
- **GIVEN** the repo contains `./trudger`
- **WHEN** a user runs `./trudger`
- **THEN** the Rust implementation is executed

## MODIFIED Requirements
### Requirement: Trudger entrypoint
The system SHALL provide a Trudger executable entrypoint. The canonical implementation SHALL be the Rust binary.

#### Scenario: Entrypoint is executable
- **WHEN** a user runs Trudger
- **THEN** it starts without requiring an explicit shell invocation

### Requirement: Configuration parsing
The system SHALL parse `~/.config/trudger.yml` using native Rust YAML parsing and SHALL treat null values as validation errors. The system SHALL NOT require external YAML/JSON parsing utilities (for example `yq` or `jq`) to load the configuration.

#### Scenario: Config parsing without external tools
- **GIVEN** `yq` and `jq` are not installed
- **WHEN** the config file is valid
- **THEN** Trudger loads and validates the configuration successfully
