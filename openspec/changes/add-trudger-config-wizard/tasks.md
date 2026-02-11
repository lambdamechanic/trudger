## 1. Template Data
- [x] 1.1 Define the template data format and add agent templates (`codex`, `claude`, `pi`) plus tracking templates (`br-next-task`, `bd-labels`) and defaults.
- [x] 1.2 Embed template data into the binary and add a loader that parses the embedded files at runtime.
- [x] 1.3 Validate at startup (wizard path) that required templates are present and error clearly if the embedded data is missing expected templates.

## 2. Wizard Command
- [x] 2.1 Add `trudger wizard` CLI path with interactive prompts for agent and tracking selection.
- [x] 2.2 Fail fast in non-interactive contexts (wizard requires stdin/stdout TTY).
- [x] 2.3 Implement config assembly (templates + embedded defaults for `review_loop_limit` and `log_path` when missing; do not prompt for these keys).
- [x] 2.4 When a valid config exists, preselect current agent/tracking choices and perform per-key diff prompts for known template-driven keys that differ (including `hooks.on_doctor_setup`), defaulting to "keep current"; preserve existing `review_loop_limit`/`log_path` values without prompting.
- [x] 2.5 Comment out unknown/custom keys (top-level and under `commands`/`hooks`) from the existing config in the generated output and warn the user.
- [x] 2.6 If an existing config cannot be parsed as YAML, warn and proceed as if no existing config is present (no preselection/merge, no unknown-key preservation), but still back up the invalid file before overwriting it.
- [x] 2.7 Validate the final assembled config using existing config parsing and validation; do not write on validation failure.
- [x] 2.8 Create the parent directory for the config path if missing.
- [x] 2.9 Create a timestamped backup when overwriting an existing config file (after successful validation; atomic write preferred).
- [x] 2.10 Update missing-config bootstrap and validation messaging to direct users to `trudger wizard`.

## 3. Docs and Tests
- [x] 3.1 Update `README.md` and `--help` output to document the wizard and remove sample-config bootstrap references.
- [x] 3.2 Update tests that rely on `sample_configuration/` and add wizard coverage (TTY requirement, defaults, per-key merge prompts with default keep-current, unknown key commenting including nested keys, invalid-YAML overwrite-with-backup, backups, overwrite).

## 4. Validation
- [x] 4.1 Run `cargo test` and fix failures.
- [x] 4.2 Run `cargo fmt` and `cargo clippy` (or the project’s preferred quality gate command) and fix failures.
- [x] 4.3 Run `openspec validate add-trudger-config-wizard --strict --no-interactive`.
