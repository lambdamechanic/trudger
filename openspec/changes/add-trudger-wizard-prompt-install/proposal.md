# Change: Wizard-managed prompt installation

## Why
Today, Trudger's task-processing mode requires prompt files at fixed paths (`~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md`). Users must discover and run `./install.sh` separately, which is easy to miss after running `trudger wizard`.

## What Changes
- Extend `trudger wizard` to optionally install (or update) the required prompt files as part of the interactive bootstrap flow.
- Keep `./install.sh` as a supported alternative for installing the same default prompts.
- Improve wizard output so it clearly reports whether prompts were installed/updated or still need installation.

## Dependencies
- Depends on `add-trudger-config-wizard` (the interactive `trudger wizard` flow and embedded template pattern).
- Depends on `refactor-trudger-command-contract` (prompt delivery via `TRUDGER_PROMPT`/`TRUDGER_REVIEW_PROMPT` and prompt file presence requirements).
- Depends on `refactor-trudger-rust-native` (wizard behavior implemented in the canonical Rust binary).

## Impact
- Affected specs: `trudger` (prompt handling + wizard flow)
- Affected code (planned): Rust wizard CLI, prompt embedding/installation, and related tests.
