# Change: Add Trudger Config Wizard

## Why
Sample configs are static and force users to edit YAML by hand to swap agents or tracking systems. A wizard gives a guided, repeatable way to generate configs, keeps templates in data files, and makes it easy to re-run with current choices as defaults.

## What Changes
- Add a `trudger wizard` command that interactively builds `~/.config/trudger.yml` from selectable agent and tracking templates.
- Replace the missing-config bootstrap output (curl sample configs) with instructions to run the wizard.
- Introduce data-driven template files stored in the repo and embedded into the binary at build time; the wizard reads templates from embedded data, not hard-coded strings.
- Support re-running the wizard: when a config exists, the wizard preselects best matches and, when values differ from the selected templates, shows per-key diffs and asks whether to replace each key individually.
- When overwriting an existing config file, create a timestamped backup.
- Comment out unknown/custom keys (top-level and under `commands`/`hooks`) from an existing config in the generated output and warn the user (so data is not silently dropped).
- If an existing config is invalid YAML, warn and overwrite it with a backup (no per-key merge).
- Use embedded defaults for `review_loop_limit` and `log_path` when missing (no wizard prompts for these fields; preserve existing values when present).
- Fail fast in non-interactive contexts (wizard requires a TTY).
- Retire or repurpose `sample_configuration/` in favor of wizard templates (no more sample-config bootstrap).

## Dependencies
- Depends on `refactor-trudger-command-contract` (current config schema/contract: `agent_command` + `agent_review_command` and env-driven prompts).
- Depends on `add-trudger-config-flag` (consistent `--config PATH` behavior; wizard reads/writes to the active config path).
- Depends on `refactor-trudger-config-and-docs` (overlapping docs/bootstrap messaging and config guidance; avoid conflicting edits).

## Impact
- Affected specs: `specs/trudger/spec.md` (configuration loading + wizard behavior).
- Affected code: CLI argument parsing, config bootstrap messaging, new wizard flow, template loading/embedding.
- Affected docs/tests: `README.md`, wizard usage docs, tests that reference sample configs or missing-config output.
