## MODIFIED Requirements
### Requirement: Configuration parsing
The script SHALL parse `~/.config/trudger.yml` using `yq`, treat null values as validation errors, and exit with a clear parse error if YAML decoding fails.

#### Scenario: Null config value
- **WHEN** a required config value is present but null
- **THEN** the script exits non-zero with a clear error naming the field

#### Scenario: Config parse failure
- **GIVEN** the config file exists but contains invalid YAML
- **WHEN** the script loads configuration
- **THEN** the script exits non-zero with a clear parse error that names the config path

## ADDED Requirements
### Requirement: Prompt substitution safety
The script SHALL substitute `$ARGUMENTS` and `$TASK_SHOW` in prompt templates as literal values, preserving special characters like `&` and backslashes without mutation.

#### Scenario: Task show output contains special characters
- **GIVEN** `commands.task_show` returns content containing `&` or backslashes
- **WHEN** Trudger renders a prompt
- **THEN** the rendered prompt includes the content exactly as returned

### Requirement: Execution logging
When `log_path` is configured, the script SHALL log command start/exit and quit reasons as single-line entries, escaping control characters (newlines, carriage returns, tabs) in logged values. The script SHALL log the full configured command strings and arguments without redaction.

#### Scenario: Command logging includes full command
- **WHEN** a configured command executes
- **THEN** the log entry includes the full command string and arguments

#### Scenario: Log values include control characters
- **WHEN** a logged value contains newlines, carriage returns, or tabs
- **THEN** the log entry replaces them with escaped sequences

### Requirement: Error exit logging
Unhandled errors SHALL be recorded via the same quit path used for explicit exits, and the script SHALL NOT emit a "quit reason" log entry without exiting.

#### Scenario: Unhandled error triggers quit
- **WHEN** an unhandled error occurs
- **THEN** the script logs the quit reason and exits non-zero

### Requirement: Reexec path resolution
After processing a task, the script SHALL re-exec itself using a resolved executable path when available, falling back to the original argv[0], and log the reexec path.

#### Scenario: Reexec uses resolved path
- **WHEN** the script restarts after handling a task
- **THEN** it uses the resolved executable path if available and logs it
