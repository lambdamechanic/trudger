## ADDED Requirements
### Requirement: Trudger configuration bootstrap
The system SHALL require a configuration file at `~/.config/trudger.yml` before starting work. If the file is missing, the system SHALL emit a curl command that installs the trudgeable sample configuration to that path and exit with a non-zero status.

#### Scenario: Config missing
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** `./trudger` starts
- **THEN** it prints a curl command that writes the trudgeable sample config to `~/.config/trudger.yml`
- **AND** it exits non-zero

#### Scenario: Config present
- **GIVEN** `~/.config/trudger.yml` exists
- **WHEN** `./trudger` starts
- **THEN** it loads settings from the file and continues execution
