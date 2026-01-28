## ADDED Requirements
### Requirement: Trudger configuration bootstrap
The system SHALL require a configuration file at `~/.config/trudger.yml` before starting work. If the file is missing, the system SHALL emit curl commands for each sample configuration (including the header docs describing each option) that install to that path and exit with a non-zero status.

#### Scenario: Config missing
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** `./trudger` starts
- **THEN** it prints curl commands for each sample config that write to `~/.config/trudger.yml`
- **AND** it includes the header docs describing each sample config
- **AND** it exits non-zero

#### Scenario: Config present
- **GIVEN** `~/.config/trudger.yml` exists
- **WHEN** `./trudger` starts
- **THEN** it loads settings from the file and continues execution
