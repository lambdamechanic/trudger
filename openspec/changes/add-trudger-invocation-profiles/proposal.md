# Change: Add profile-based invocation selection and reusable invocations

## Why
Trudger currently hard-codes two top-level command strings (`agent_command` and `agent_review_command`) in the config, which causes duplication and makes ad-hoc switching across agent setups cumbersome. We need a profile model so users can run `trudger -p codex`, `trudger -p z.ai`, etc., while still having a required default profile when no flag is provided.

## Approval
- Status: Approved
- Approved on: 2026-02-27

## What Changes
- Add a task-processing-only `-p/--profile PROFILE` CLI flag to select the active profile for this invocation.
- Reject `-p/--profile` in `trudger doctor` and `trudger wizard` with clear mode-specific errors.
- Introduce reusable invocation definitions in config, then reference them from profile phase keys (`trudge`, `trudge_review`) so one invocation can be reused without duplication.
- Require `default_profile` in config and use it when `-p/--profile` is not provided.
- **BREAKING** Replace required top-level `agent_command`/`agent_review_command` with profile + invocation mappings.
- Add generic phase prompt env signaling for agent invocations (for shared invocation commands), and remove legacy `TRUDGER_PROMPT` / `TRUDGER_REVIEW_PROMPT` immediately.
- Stop appending `resume --last` to review invocations; both phases run with a clean invocation context unless the configured command itself adds arguments.
- Keep unknown keys under `profiles.*` and `invocations.*` non-fatal (warn + continue) to preserve current warning-based extensibility.
- Update sample configs, template configs, wizard-generated config output, and migration docs/examples for local user config updates (including `~/.config/trudger.yml` and `~/.config/trudge.yml` when that legacy file exists).
- Keep wizard agent choices (`codex`, `claude`, `pi`) and emit a multi-profile mapping that includes the selected agent profile plus predefined `z.ai`.
- Package a `pi_trudge` helper as a Rust binary target shipped with Trudger and use it for the predefined `z.ai` invocation in sample/template/wizard-generated configs via `pi_trudge --prompt-env TRUDGER_AGENT_PROMPT`, instead of machine-local ambient paths.
- Make packaged `pi_trudge` stateless by default per invocation (clean context; no implicit session resume behavior).

## Dependencies
- Depends on `refactor-trudger-cli-args` for current run/wizard/doctor mode argument parsing boundaries.
- Depends on `refactor-trudger-command-contract` for current prompt/env contract foundations.
- Depends on `add-trudger-config-wizard` because generated config schema must move with runtime schema.
- Depends on `add-trudger-wizard-prompt-install` for coordinated wizard + README changes touching prompt/env docs and wizard output.
- Depends on `refactor-trudger-rust-native` because runtime, config parsing, and wizard behavior are all in the Rust binary.
- Coordinates with `add-trudger-config-wizard` wizard-template requirements that currently assume single-agent template outputs (`codex`/`claude`/`pi`) so wizard behavior remains spec-consistent after this multi-profile output change.

## Impact
- Affected specs: `trudger`
- Affected code: `src/cli.rs`, `src/config.rs`, `src/run_loop.rs`, `src/shell.rs`, `src/unit_tests.rs`, `src/wizard_templates.rs`, `src/bin/pi_trudge.rs`, `README.md`, `sample_configuration/*.yml`, `config_templates/**/*.yml`, packaged `pi_trudge` helper implementation
