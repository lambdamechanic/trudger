# Change: Add Trudger Doctor Scratch DB Hook

## Why
Doctor checks need a safe, mutable task database to validate configured commands without touching production data. A dedicated scratch setup hook lets users recreate a task DB using whatever local context they need, targeting an implementation-defined temporary location.

## What Changes
- Add `hooks.on_doctor_setup` to initialize a scratch database for doctor mode.
- Add a minimal `trudger doctor` entrypoint that creates a temporary scratch directory, invokes the setup hook from the invocation working directory with the scratch path available via env var, and then cleans up the scratch directory (doctor checks land in a follow-up change).
- Update sample configs to reinitialize a scratch task database using local context, targeting the scratch directory.
- Document the doctor setup hook and environment contract.

## Impact
- Affected specs: `trudger`
- Affected code: Rust CLI, config parsing/validation, sample configs, docs/tests

## Dependencies
- `refactor-trudger-command-contract` (doctor hook inherits env var execution rules)
- `refactor-trudger-rust-native` (doctor subcommand lands in the canonical Rust CLI)
