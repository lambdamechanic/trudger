# Legacy Bash Trudger (Deprecated)

This folder contains the historical Bash implementation of Trudger and its BATS test suite.

As of **2026-02-06**, the Rust binary is the canonical Trudger implementation. The repo-root `./trudger`
file is a shim that delegates to the Rust binary; `./install.sh` installs prompt files only (binary installation is via `cargo install`).

This legacy Bash implementation is kept for reference only and is no longer exercised by CI or git hooks.
It may drift and should not be relied upon for correctness.

Files:
- `historical/bash/trudger.bash`: legacy Bash task loop.
- `historical/bash/tests/`: legacy BATS tests and fixtures.
