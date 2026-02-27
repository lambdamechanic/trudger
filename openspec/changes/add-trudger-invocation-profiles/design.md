## Context
Current config requires two top-level agent command strings, which forces duplication and limits ad-hoc switching. Users want named profile combos that can be selected with a CLI flag, with a required default profile for unattended runs.

## Goals / Non-Goals
- Goals:
- Add task-processing `-p/--profile PROFILE` for per-run profile selection.
- Introduce reusable invocation definitions and phase-level references (`trudge` / `trudge_review`) to avoid repeated command strings.
- Require a `default_profile` for deterministic behavior when no profile override is passed.
- Make prompt env delivery generic enough that a single invocation command can serve both phases.
- Non-Goals:
- Changing task selection lifecycle or review-loop semantics.
- Changing prompt file locations.

## Decisions
- Config schema changes to:
  - `default_profile: <profile-id>`
  - `profiles.<profile-id>.trudge: <invocation-id>`
  - `profiles.<profile-id>.trudge_review: <invocation-id>`
  - `invocations.<invocation-id>.command: <shell command>`
- CLI selection order:
  1. Use `-p/--profile` when provided.
  2. Otherwise use `default_profile`.
  3. Reject `-p/--profile` in `trudger doctor` and `trudger wizard`.
- Validation rules:
  - `default_profile` is required and must exist in `profiles`.
  - `profiles` and `invocations` must be non-empty mappings.
  - Every profile phase reference must point to an existing invocation id.
  - Legacy `agent_command` / `agent_review_command` are rejected with migration guidance.
  - Unknown keys under `profiles.<id>` and `invocations.<id>` remain warning-only (startup continues).
- Execution rules:
  - Solve invocation command resolves from `profiles.<active>.trudge`.
  - Review invocation command resolves from `profiles.<active>.trudge_review`.
  - Trudger does not append positional args to solve/review invocations.
- Env contract updates:
  - Add generic prompt variable `TRUDGER_AGENT_PROMPT` for both solve/review phases.
  - Add `TRUDGER_AGENT_PHASE` with values `trudge` or `trudge_review`.
  - Add profile/invocation context keys (`TRUDGER_PROFILE`, `TRUDGER_INVOCATION_ID`).
  - Remove legacy `TRUDGER_PROMPT` and `TRUDGER_REVIEW_PROMPT` immediately.
  - Keep existing task context keys (`TRUDGER_TASK_*`, etc.) unchanged.
- Artifact updates:
  - Update sample configs, embedded config templates, and wizard-generated config output to the profile/invocation schema.
  - Wizard keeps agent template choices (`codex`, `claude`, `pi`) and emits multi-profile config that includes the selected agent profile plus predefined `z.ai`.
  - `default_profile` is set to the selected agent template id.
  - Predefined `z.ai` invocation uses packaged `pi_trudge --prompt-env TRUDGER_AGENT_PROMPT` (no machine-local ambient helper path).
  - Migration docs always cover `~/.config/trudger.yml` and additionally cover `~/.config/trudge.yml` when that file exists.
- Helper packaging:
  - Ship `pi_trudge` with Trudger as a Rust binary target installed alongside `trudger` so `z.ai` invocation samples/templates/wizard output can reference a stable packaged command without Python runtime dependencies.
  - `pi_trudge` defaults to stateless execution per invocation (clean context, no implicit session resume behavior).

## Risks / Trade-offs
- This is a breaking config change and requires migration of existing configs.
- New env keys increase contract surface area and test coverage requirements.
- Supporting both legacy and new config forms would reduce migration pain but increase complexity; this proposal chooses a strict migration path.

## Migration Plan
1. Convert local configs from top-level `agent_command`/`agent_review_command` to `default_profile` + `profiles` + `invocations`.
2. Update shared examples/templates first to provide copy-paste migration references.
3. Update wrappers that currently branch on `TRUDGER_PROMPT` vs `TRUDGER_REVIEW_PROMPT` to support `TRUDGER_AGENT_PROMPT` + `TRUDGER_AGENT_PHASE`.
4. Validate with `trudger doctor` and then run `trudger -p <profile>` for smoke checks.

## Open Questions
- None.
