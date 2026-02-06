## Context
The Rust implementation has become the source of truth for Trudger behavior, but we still carry a full Bash implementation at the repo root. Users and CI end up exercising different paths depending on installation method, which makes releases risky.

## Goals / Non-Goals
- Goals:
  - Make Rust the canonical and default runtime path for `trudger`.
  - Reduce duplication by demoting the Bash `./trudger` file to a minimal shim.
  - Remove Bash-only tool requirements (for example `yq`) from the Trudger runtime.
  - Move core behavior coverage to Rust tests/integration tests.
- Non-Goals:
  - Rewriting configured commands/hooks to avoid `bash -lc` execution.
  - Changing the Trudger config schema as part of the migration (handled by other changes).

## Decisions
- The Rust binary is the canonical implementation.
- The repo-root `./trudger` file (if retained) is a compatibility shim only:
  - It SHALL NOT implement the task loop.
  - It SHOULD delegate to the Rust binary and surface clear errors if the binary is missing.
- Release/validation focus shifts to Rust:
  - Core correctness is validated via Rust tests.
  - Shell tests remain only as needed to validate shim behavior.

## Open Questions
- How should the shim locate the Rust binary?
  - Options: build/install to a known location via `install.sh`, or require users to run `cargo build --release`.
- What is the preferred distribution mechanism?
  - Options: build from source (cargo), or publish prebuilt binaries and update `install.sh` to fetch them.
