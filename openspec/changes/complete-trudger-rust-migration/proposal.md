# Change: Complete Trudger Rust Migration

## Why
Trudger's Rust implementation is now the canonical one, but the repository still ships and tests a full Bash implementation. That duplication creates drift, complicates releases, and keeps older tool dependencies (for example `yq`) in the critical path.

## What Changes
- Make the Rust binary the official Trudger implementation and release artifact.
- Convert the repo-root `./trudger` script into a thin shim that delegates to the Rust binary (no task loop logic in Bash).
- Update install/docs/tests/tooling so the Rust binary is what users run and what CI validates.

## Impact
- Affected specs: `trudger`
- Affected code: `install.sh`, `./trudger` shim, docs, tests, git hooks

## Dependencies
- `refactor-trudger-rust-native`
- `refactor-trudger-command-contract`
- `add-trudger-doctor-scratch-db`
- `refactor-trudger-cli-args`
