# Change: Refactor Trudger To Rust-Native CLI

## Why
Trudger's current Bash script is brittle (parse errors from quoting) and depends on `yq` for configuration parsing. A Rust-native implementation with typed YAML parsing and subprocess execution should be more robust and portable while preserving existing behavior.

## What Changes
- Add a Rust-native Trudger implementation built via Cargo (release binary under `target/release`), maintained side-by-side with the existing shell script.
- Keep `./trudger` behavior compatible while we validate parity for hooks and log format.
- Parse `~/.config/trudger.yml` into a typed schema using a native YAML parser in the Rust implementation (no `yq`/`jq` for config loading).
- Replace the Rust implementation's self re-exec loop with an in-process task loop; execute configured commands and hooks as child processes.
- Preserve prompt handling and task lifecycle semantics from the current spec.

## Impact
- Affected specs: `trudger`
- Affected code: Rust implementation, shell script maintained for fallback during migration

## Dependencies
- Depends on `update-trudger-logging-and-parsing` to stabilize the shell log format used for parity.
