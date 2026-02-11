## 1. Implementation
- [x] 1.1 Create a Rust CLI binary (release output under `target/release`, e.g. `trudger`) that matches the current `./trudger` interface and flags.
- [x] 1.2 Define a typed configuration schema and validate required fields, nulls, and unknown keys (warnings).
- [x] 1.3 Implement the in-process task loop (next task selection, status checks, agent solve + review).
- [x] 1.4 Execute configured commands and hooks via subprocesses using the `TRUDGER_*` env var contract (no positional task args).
- [x] 1.5 Implement logging and error handling per spec, with log format parity vs the shell implementation.
- [x] 1.6 Implement tmux updates with roughly equivalent semantics (exact parity not required).
- [x] 1.7 Add tests covering config parsing, missing files, command execution, hook substitution semantics, log formatting, and multi-task iterations.
- [x] 1.8 Validation: run `cargo test` and a smoke run using `sample_configuration/*.yml` with the release binary (for example `target/release/trudger`).
