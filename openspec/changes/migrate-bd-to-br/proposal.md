# Change: Migrate issue tracking from bd to br

## Why
Trudger currently documents and shells out to the legacy beads CLI. The team wants to standardize on br (beads_rust), which is non-invasive and does not run git commands. We need to migrate docs, scripts, and tests to avoid stale instructions and ensure the workflow matches br behavior.

## What Changes
- Update documentation, prompts, and sample configs to use br commands.
- Replace legacy sync commands with `br sync --flush-only` and add manual git steps.
- Update `trudger` script usage/config bootstrap messaging and any defaults to br.
- Update specs and tests to reflect br terminology and behavior.

## Impact
- Affected specs: `trudger`
- Affected code/docs: `trudger`, `README.md`, `AGENTS.md`, `prompts/*.md`, `sample_configuration/*.yml`, `tests/**`, `tests/fixtures/bin/*`, `openspec/changes/*`
