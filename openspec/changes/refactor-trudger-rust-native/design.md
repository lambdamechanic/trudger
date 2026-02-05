## Context
Trudger is currently a Bash script that depends on `yq` for config parsing and re-executes itself between iterations. This has proven brittle (quoting and parse errors) and makes reliability dependent on shell parsing and external tools.

## Goals / Non-Goals
Goals:
- Implement a Rust binary (release output under `target/release`, e.g. `trudger`) while preserving existing behavior.
- Parse `~/.config/trudger.yml` with a native YAML parser into a typed schema.
- Remove self re-exec; keep a single-process loop and spawn configured commands/hooks as subprocesses.
- Maintain prompt rendering, hook semantics, logging, and exit behavior from the spec.
Non-Goals:
- Changing the configuration schema or required fields beyond current spec.
- Removing support for shell-based command strings in config.
- Changing task semantics (next_task selection, status checks, review flow).

## Decisions
- Use `serde` + `serde_yaml` to deserialize config into typed structs; treat nulls or missing required fields as validation errors.
- Preserve unknown top-level keys by parsing the root map and warning on keys not in the schema.
- Execute command strings via `std::process::Command` (e.g., `bash -lc <command>`), and preserve existing `$1`/`${1}` substitution behavior for hooks.
- Implement the task loop in-process (no self re-exec) and keep all task state in memory across iterations.

## Risks / Trade-offs
- Shell command strings remain inherently unsafe; using `bash -lc` preserves compatibility but keeps shell semantics.
- Adding the Rust binary requires a build/install step; packaging needs to be defined.

## Migration Plan
- Build a Rust binary that outputs to the standard Cargo release location while keeping the shell `./trudger` for fallback.
- Keep configuration paths and CLI options identical to avoid breaking existing setups.

## Open Questions
- Should we add a JSON schema export or `trudger-rs validate-config` subcommand for debugging?
