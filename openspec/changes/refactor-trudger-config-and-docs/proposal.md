# Change: Refactor Trudger config parsing, docs, and test helpers

## Why
Trudger’s current docs and specs no longer match actual behavior, and the repo contains duplicate spec content plus repeated test/fixture logic that obscures intent and increases maintenance costs.

## What Changes
- Consolidate duplicated spec content and align behavior docs/specs with the current hook-based workflow.
- Replace the ad-hoc YAML parser with a real parser (`yq`) for configuration loading.
- Centralize base test config setup and shared fixture queue helpers.
- Factor a shared command-execution helper for task/hook commands.

## Impact
- Affected specs: `trudger`
- Affected code: `trudger`, `README.md`, `prompts/*.md`, `tests/trudger_test.bats`, `tests/fixtures/bin/*`, `openspec/changes/*`
- New dependency: `yq` (runtime requirement for config parsing)

## Dependencies
- Depends on completing and archiving `add-trudger-config`, `migrate-bd-to-br`, `add-trudger-config-flag`, and `update-trudger-config-bootstrap` before consolidating spec copies under `openspec/changes/`.
