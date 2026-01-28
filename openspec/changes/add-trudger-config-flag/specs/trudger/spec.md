## ADDED Requirements
### Requirement: Config path override flag
The script SHALL accept `-c` or `--config` to override the default config file path and load configuration from the provided file.

#### Scenario: Config flag provided
- **GIVEN** a config file exists at the provided path
- **WHEN** the user runs `./trudger --config /path/to/file.yml`
- **THEN** the script loads configuration from that path instead of `~/.config/trudger.yml`

#### Scenario: Config flag points to missing file
- **GIVEN** the provided config path does not exist
- **WHEN** the user runs `./trudger -c /missing.yml`
- **THEN** the script exits non-zero with a clear error about the missing config file

### Requirement: Sample configuration files
The repository SHALL include sample configuration files for legacy/default behavior and `bv --robot-triage` task selection.

#### Scenario: Sample configs available
- **WHEN** a developer opens the repository
- **THEN** sample configs are available for copy/adaptation
