# trudger Spec Delta

## MODIFIED Requirements

### Requirement: Configuration loading
The system SHALL load configuration from `~/.config/trudger.yml` by default and SHALL allow overriding the config path via `-c/--config PATH`. The system SHALL exit with a clear error if the selected config file is missing.

In task-processing mode, the system SHALL allow selecting an active profile via `-p/--profile PROFILE`. When `-p/--profile` is omitted, the system SHALL use `default_profile` from config.

The system SHALL reject `-p/--profile` in non-task-processing modes (`trudger doctor` and `trudger wizard`) with a clear mode-specific error.

#### Scenario: Missing default config prints bootstrap instructions
- **GIVEN** `~/.config/trudger.yml` does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints instructions to run `trudger wizard`
- **AND** it mentions `trudger wizard --config PATH` for generating a config at a non-default path

#### Scenario: Missing explicit config prints missing-path error
- **GIVEN** a user provides `-c/--config PATH`
- **AND** PATH does not exist
- **WHEN** a user runs `trudger`
- **THEN** the system exits non-zero and prints a clear missing config error that includes PATH

#### Scenario: Profile flag selects non-default profile
- **GIVEN** config sets `default_profile: codex`
- **AND** config contains profile `z.ai`
- **WHEN** a user runs `trudger -p z.ai`
- **THEN** Trudger uses profile `z.ai` for solve/review invocation resolution

#### Scenario: Omitted profile flag uses default profile
- **GIVEN** config sets `default_profile: codex`
- **WHEN** a user runs `trudger`
- **THEN** Trudger uses profile `codex` for solve/review invocation resolution

#### Scenario: Profile flag applies to manual task runs
- **GIVEN** config sets `default_profile: codex`
- **AND** config contains profile `z.ai`
- **WHEN** a user runs `trudger -t tr-1 -p z.ai`
- **THEN** Trudger uses profile `z.ai` for solve/review invocation resolution

#### Scenario: Unknown profile flag errors
- **GIVEN** config does not contain profile `moonshot`
- **WHEN** a user runs `trudger -p moonshot`
- **THEN** Trudger exits non-zero with a clear error naming `moonshot`

#### Scenario: Doctor mode rejects profile flag
- **WHEN** a user runs `trudger doctor -p codex`
- **THEN** Trudger exits non-zero with a clear error that `-p/--profile` is not supported in doctor mode

#### Scenario: Wizard mode rejects profile flag
- **WHEN** a user runs `trudger wizard -p codex`
- **THEN** Trudger exits non-zero with a clear error that `-p/--profile` is not supported in wizard mode

### Requirement: Configuration validation
The system SHALL require `default_profile`, `profiles`, `invocations`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, and `hooks.on_requires_human` to be present and non-empty.

`profiles` SHALL be a non-empty mapping. Each profile SHALL provide non-empty `trudge` and `trudge_review` invocation ids.

`invocations` SHALL be a non-empty mapping. Each invocation SHALL provide a non-empty `command` string.

`default_profile` SHALL reference an existing profile key.

Every invocation id referenced by `profiles.*.trudge` and `profiles.*.trudge_review` SHALL exist in `invocations`.

`commands.next_task` SHALL be required only when no manual task ids are provided.

`log_path` SHALL be optional; when it is missing or empty, logging is disabled.

`hooks.on_notification` SHALL be optional; when present, it MUST be non-empty.

`hooks.on_notification_scope` SHALL be optional; when present, it MUST be one of `all_logs`, `task_boundaries`, or `run_boundaries`. If `hooks.on_notification` is configured and `hooks.on_notification_scope` is omitted, the system SHALL default to `task_boundaries`.

If `hooks.on_notification_scope` is configured while `hooks.on_notification` is not configured, the system SHALL continue startup and SHALL emit a warning that `hooks.on_notification_scope` is ignored.

The system SHALL reject top-level `agent_command` and `agent_review_command` keys with a clear migration error that instructs users to migrate to `default_profile`, `profiles`, and `invocations`.

Unknown keys under `profiles.<profile-id>` and `invocations.<invocation-id>` SHALL emit warnings and SHALL NOT block startup.

#### Scenario: Required config value missing
- **WHEN** any required config value is missing or empty
- **THEN** the system exits non-zero with a clear error naming the missing field

#### Scenario: Default profile must reference existing profile
- **GIVEN** config sets `default_profile: codex`
- **AND** `profiles.codex` is missing
- **WHEN** Trudger validates configuration
- **THEN** it exits non-zero with a clear error naming `default_profile`

#### Scenario: Profile references missing invocation
- **GIVEN** `profiles.codex.trudge_review` references `codex-review`
- **AND** `invocations.codex-review` is missing
- **WHEN** Trudger validates configuration
- **THEN** it exits non-zero with a clear error naming the missing invocation id

#### Scenario: Legacy agent command keys are rejected
- **GIVEN** config contains top-level `agent_command` or `agent_review_command`
- **WHEN** Trudger validates configuration
- **THEN** it exits non-zero with a migration error naming the legacy key and the new profile/invocation keys

#### Scenario: Unknown nested profile/invocation keys warn and continue
- **GIVEN** config includes unknown keys under `profiles.codex` or `invocations.codex`
- **WHEN** Trudger validates configuration
- **THEN** it emits warning output naming each unknown key path
- **AND** it continues startup

#### Scenario: Invalid notification scope errors
- **GIVEN** `hooks.on_notification_scope` is set to an unsupported value
- **WHEN** Trudger validates configuration
- **THEN** it exits non-zero with a clear error naming `hooks.on_notification_scope`

#### Scenario: Notification scope without hook warns and is ignored
- **GIVEN** `hooks.on_notification_scope` is configured
- **AND** `hooks.on_notification` is missing
- **WHEN** Trudger validates configuration
- **THEN** Trudger continues startup
- **AND** it emits a warning that `hooks.on_notification_scope` is ignored

### Requirement: Command execution environment
The system SHALL execute configured commands and hooks without positional task arguments and SHALL provide task context via environment variables. When a task context exists, the system SHALL set `TRUDGER_TASK_ID`. After task show output is available, the system SHALL set `TRUDGER_TASK_SHOW`. After task status is available, the system SHALL set `TRUDGER_TASK_STATUS`. The system SHALL always set `TRUDGER_CONFIG_PATH` to the active config path.

For agent solve/review invocations, the system SHALL set `TRUDGER_AGENT_PROMPT` to the active phase prompt and SHALL set `TRUDGER_AGENT_PHASE` to `trudge` or `trudge_review`. The system SHALL set `TRUDGER_PROFILE` to the active profile id and `TRUDGER_INVOCATION_ID` to the resolved invocation id.

For agent solve/review invocations, the system SHALL NOT set legacy prompt env vars `TRUDGER_PROMPT` or `TRUDGER_REVIEW_PROMPT`.

Before spawning configured commands/hooks, the system SHALL truncate individual `TRUDGER_*` environment variable values that exceed 64 KiB (bytes) at a UTF-8 character boundary to reduce the risk of command spawn failures (E2BIG). When truncation occurs, the system SHALL print a warning and (when `log_path` is configured) log an `env_truncate` transition.

#### Scenario: Solve invocation receives generic phase env
- **WHEN** Trudger executes a solve invocation
- **THEN** it sets `TRUDGER_AGENT_PROMPT` to trudge prompt content
- **AND** it sets `TRUDGER_AGENT_PHASE=trudge`
- **AND** it sets `TRUDGER_PROFILE` and `TRUDGER_INVOCATION_ID`

#### Scenario: Review invocation receives generic phase env
- **WHEN** Trudger executes a review invocation
- **THEN** it sets `TRUDGER_AGENT_PROMPT` to trudge-review prompt content
- **AND** it sets `TRUDGER_AGENT_PHASE=trudge_review`
- **AND** it sets `TRUDGER_PROFILE` and `TRUDGER_INVOCATION_ID`

#### Scenario: Legacy prompt env vars are removed
- **WHEN** Trudger executes a solve or review invocation
- **THEN** `TRUDGER_PROMPT` and `TRUDGER_REVIEW_PROMPT` are unset

#### Scenario: Oversized env values are truncated and warned
- **WHEN** `TRUDGER_AGENT_PROMPT` exceeds the truncation threshold
- **THEN** Trudger truncates it before spawning the command
- **AND** it prints a warning indicating truncation occurred

### Requirement: Agent prompt execution
For each selected task, the system SHALL resolve the active profile and execute solve + review commands through invocation references.

Solve SHALL use the command at `invocations[profiles[active_profile].trudge].command`.

Review SHALL use the command at `invocations[profiles[active_profile].trudge_review].command`.

The system SHALL load prompt content from `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` without performing `$ARGUMENTS` or `$TASK_SHOW` substitutions, and SHALL provide prompt content through `TRUDGER_AGENT_PROMPT` together with task context (`TRUDGER_*`).

Trudger SHALL NOT append additional positional arguments to solve or review invocation commands.

#### Scenario: Shared invocation reused across phases
- **GIVEN** `profiles.codex.trudge` and `profiles.codex.trudge_review` both reference `invocations.codex`
- **WHEN** a task is selected
- **THEN** Trudger uses `invocations.codex.command` for both solve and review
- **AND** Trudger differentiates phase behavior via `TRUDGER_AGENT_PHASE`

#### Scenario: Profile combo uses different invocations per phase
- **GIVEN** `profiles.hybrid.trudge` references `invocations.codex`
- **AND** `profiles.hybrid.trudge_review` references `invocations.zai`
- **WHEN** a user runs `trudger -p hybrid`
- **THEN** solve uses `invocations.codex.command`
- **AND** review uses `invocations.zai.command`

#### Scenario: Review invocation does not receive Trudger-appended args
- **GIVEN** `profiles.codex.trudge_review` references `invocations.codex`
- **WHEN** Trudger executes review for a task
- **THEN** it runs `invocations.codex.command` without additional positional args appended by Trudger

## ADDED Requirements

### Requirement: Configuration artifacts stay aligned with supported schema
The project SHALL keep checked-in configuration artifacts aligned with the supported runtime schema. `sample_configuration/*.yml`, `config_templates/**/*.yml`, and configuration guidance in `README.md` SHALL use the active profile/invocation schema and SHALL NOT use deprecated top-level `agent_command` or `agent_review_command` keys.

Generated config output from `trudger wizard` SHALL use the active profile/invocation schema and SHALL NOT emit deprecated top-level `agent_command` or `agent_review_command` keys.

Generated config output from `trudger wizard` SHALL preserve wizard agent template choices (`codex`, `claude`, `pi`) and SHALL emit a multi-profile mapping that includes the selected agent profile and predefined `z.ai`, each wired through `invocations`.

When `trudger wizard` emits this predefined multi-profile config, `default_profile` SHALL be set to the selected agent template id.

The predefined `z.ai` invocation used in checked-in samples/templates/wizard output SHALL use a packaged `pi_trudge` helper command with the form `pi_trudge --prompt-env TRUDGER_AGENT_PROMPT` and SHALL NOT reference machine-local absolute paths (for example `$HOME/.local/bin/pi_trudge`).

Migration guidance SHALL include updating local user config files that still use the legacy command keys, including `~/.config/trudger.yml` and `~/.config/trudge.yml` when that legacy path exists.

#### Scenario: Sample and template configs use profile/invocation schema
- **WHEN** a user opens checked-in sample/template config files
- **THEN** the files define `default_profile`, `profiles`, and `invocations`
- **AND** the files do not define deprecated top-level `agent_command` or `agent_review_command`

#### Scenario: Wizard output uses profile/invocation schema
- **WHEN** a user runs `trudger wizard`
- **THEN** the generated config defines `default_profile`, `profiles`, and `invocations`
- **AND** it does not define deprecated top-level `agent_command` or `agent_review_command`

#### Scenario: Wizard output includes predefined codex and z.ai profiles
- **GIVEN** no existing config file is present at the target path
- **AND** the user selects agent template `codex`
- **WHEN** the user completes `trudger wizard`
- **THEN** the generated config includes profiles `codex` and `z.ai`
- **AND** both profiles reference invocation ids defined under `invocations`
- **AND** `default_profile` is `codex`

#### Scenario: Wizard preserves alternate agent choices with multi-profile mapping
- **GIVEN** no existing config file is present at the target path
- **AND** the user selects agent template `claude`
- **WHEN** the user completes `trudger wizard`
- **THEN** the generated config includes profiles `claude` and `z.ai`
- **AND** both profiles reference invocation ids defined under `invocations`
- **AND** `default_profile` is `claude`

#### Scenario: z.ai invocation uses packaged pi_trudge helper
- **WHEN** a user opens checked-in sample/template config files or generated wizard output
- **THEN** the predefined `z.ai` invocation command uses packaged `pi_trudge`
- **AND** the predefined `z.ai` invocation command includes `--prompt-env TRUDGER_AGENT_PROMPT`
- **AND** the command does not include machine-local absolute paths such as `$HOME/.local/bin/pi_trudge`

### Requirement: pi_trudge helper is packaged with Trudger
The project SHALL distribute a `pi_trudge` helper command with Trudger as a Rust binary target installed alongside `trudger`, so profile/invocation configs can reference it without requiring users to maintain a machine-local ambient script.

The packaged `pi_trudge` command SHALL default to stateless execution per invocation and SHALL NOT rely on implicit session resume behavior.

#### Scenario: Packaged helper is available after install
- **WHEN** a user installs Trudger
- **THEN** a `pi_trudge` command is available from the installation

#### Scenario: Packaged helper does not depend on Python runtime
- **WHEN** a user installs Trudger through the standard Rust binary install path
- **THEN** `pi_trudge` is available as an installed binary command
- **AND** using `pi_trudge` does not require a machine-local Python script path

#### Scenario: Packaged helper defaults to clean invocation context
- **WHEN** Trudger executes the predefined `z.ai` invocation command
- **THEN** `pi_trudge` starts with clean per-invocation context by default
- **AND** the default command path does not depend on resuming prior sessions

#### Scenario: Migration guidance includes local config updates
- **WHEN** a user follows migration docs for this change
- **THEN** instructions include updating local config files to the profile/invocation schema
- **AND** instructions mention migrating `~/.config/trudger.yml`
- **AND** instructions mention migrating `~/.config/trudge.yml` when that file exists
