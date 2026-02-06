## 1. Template Data
- [ ] 1.1 Define the template data format and add agent templates (`codex`, `claude`, `pi`) plus tracking templates (`br-next-task`, `bd-labels`) and defaults.
- [ ] 1.2 Embed template data into the binary and add a loader that parses the embedded files at runtime.
- [ ] 1.3 Validate at startup (wizard path) that required templates are present and error clearly if the embedded data is missing expected templates.

## 2. Wizard Command
- [ ] 2.1 Add `trudger wizard` CLI path with interactive prompts for agent and tracking selection.
- [ ] 2.2 Fail fast in non-interactive contexts (wizard requires stdin/stdout TTY).
- [ ] 2.3 Implement config assembly (templates + embedded defaults for `review_loop_limit` and `log_path`).
- [ ] 2.4 When a config exists, preselect current agent/tracking choices and perform per-key diff prompts for known keys that differ, letting the user keep or replace each key.
- [ ] 2.5 Comment out unknown/custom top-level keys from the existing config in the generated output and warn the user.
- [ ] 2.6 Validate the final assembled config using existing config parsing and validation; do not write on validation failure.
- [ ] 2.7 Create the parent directory for the config path if missing.
- [ ] 2.8 Create a timestamped backup when overwriting an existing config file.
- [ ] 2.9 Update missing-config bootstrap and validation messaging to direct users to `trudger wizard`.

## 3. Docs and Tests
- [ ] 3.1 Update `README.md` and `--help` output to document the wizard and remove sample-config bootstrap references.
- [ ] 3.2 Update tests that rely on `sample_configuration/` and add wizard coverage (TTY requirement, defaults, per-key merge prompts, unknown key commenting, backups, overwrite).

## 4. Validation
- [ ] 4.1 Run `cargo test` and fix failures.
- [ ] 4.2 Run `bats tests/trudger_test.bats` (or the project’s preferred test command) and fix failures.
- [ ] 4.3 Run `openspec validate add-trudger-config-wizard --strict --no-interactive`.
