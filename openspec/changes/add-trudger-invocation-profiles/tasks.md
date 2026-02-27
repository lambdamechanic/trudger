## 1. CLI + Config Schema
- [ ] 1.1 Add `-p/--profile PROFILE` to task-processing run mode, fail fast when the selected profile does not exist, and reject `-p/--profile` in `doctor`/`wizard`.
- [ ] 1.2 Replace config schema requirements from `agent_command`/`agent_review_command` to `default_profile`, `profiles`, and `invocations`.
- [ ] 1.3 Validate that `default_profile` exists in `profiles` and every `profiles.*.(trudge|trudge_review)` reference exists in `invocations`.
- [ ] 1.4 Reject legacy `agent_command`/`agent_review_command` keys with a migration error.
- [ ] 1.5 Keep unknown keys under `profiles.*` and `invocations.*` non-fatal with clear warnings (matching existing unknown-key behavior).

## 2. Invocation Resolution + Execution
- [ ] 2.1 Resolve the active profile (CLI override, else `default_profile`) before task processing begins.
- [ ] 2.2 Resolve solve/review commands via `profiles.<active>.trudge` and `profiles.<active>.trudge_review` references to `invocations.<id>.command`.
- [ ] 2.3 Remove Trudger-managed review argument appending (`resume --last`) so review invocations run without extra positional args added by Trudger.

## 3. Environment Contract
- [ ] 3.1 Provide a generic prompt env variable for both phases (for shared invocation commands), plus a phase discriminator (`trudge` vs `trudge_review`).
- [ ] 3.2 Expose active profile/invocation identifiers in env for debugging and wrapper scripts.
- [ ] 3.3 Remove legacy `TRUDGER_PROMPT` / `TRUDGER_REVIEW_PROMPT` immediately and ensure only the new prompt contract is emitted.
- [ ] 3.4 Keep env truncation protections and logging behavior for new env keys.

## 4. Docs + Config Artifacts
- [ ] 4.1 Update `README.md` for profile/invocation schema and `trudger -p PROFILE` usage.
- [ ] 4.2 Update `sample_configuration/*.yml` and `config_templates/**/*.yml` to the new schema.
- [ ] 4.3 Update wizard-generated config output (`trudger wizard`) to the new schema while keeping agent template choices (`codex`, `claude`, `pi`) and emitting a multi-profile mapping that includes selected-agent + `z.ai`, with `default_profile` set to the selected agent id, including template parsing/merge logic in `src/wizard_templates.rs` and `src/wizard.rs`.
- [ ] 4.4 Update migration guidance/examples for local user configs, including `~/.config/trudger.yml` and `~/.config/trudge.yml` when that legacy file exists.
- [ ] 4.5 Package a `pi_trudge` helper with Trudger as a Rust binary target and wire predefined `z.ai` invocation commands in samples/templates/wizard output to that packaged helper (not `$HOME/.local/bin/...`).
- [ ] 4.7 Make packaged `pi_trudge` stateless by default per invocation and verify predefined `z.ai` mapping does not depend on resume/session persistence.
- [ ] 4.6 Update wizard template requirements/validation that currently assume `codex`/`claude`/`pi` single-agent outputs so the new predefined multi-profile output stays consistent across spec, templates, and wizard prompts.

## 5. Tests + Validation
- [ ] 5.1 Add/adjust tests for profile selection, default profile fallback, invalid profile errors, non-run-mode `-p` rejection, invocation reference validation, unknown nested key warnings, wizard output schema (selected-agent + `z.ai` with preserved `codex`/`claude`/`pi` choices), immediate legacy env removal, no Trudger-managed review args, packaged Rust `pi_trudge` helper availability, stateless `pi_trudge` defaults, and env contract updates.
- [ ] 5.2 Run `cargo nextest run` (or `cargo test` if nextest unavailable).
- [ ] 5.3 Run `cargo fmt` and `cargo clippy`.
- [ ] 5.4 Run `openspec validate add-trudger-invocation-profiles --strict --no-interactive`.

## 6. Dependency Order
- [ ] 6.1 Complete `1.2` before `1.3` and `1.4`.
- [ ] 6.2 Complete `1.3` before `2.1`, then `2.1` before `2.2`, then `2.2` before `2.3`.
- [ ] 6.3 Complete `2.2` before `3.1` and `3.2`, then complete `3.1` and `3.2` before `3.3` and `3.4`.
- [ ] 6.4 Complete `1.x`, `2.x`, and `3.x` before `4.x` documentation/artifact updates.
- [ ] 6.5 Complete `4.x` before `5.1` so tests assert final artifact/schema behavior.
