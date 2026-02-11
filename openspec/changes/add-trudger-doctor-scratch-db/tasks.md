## 1. Implementation
- [x] 1.1 Add `hooks.on_doctor_setup` to the config schema (Rust; keep shell in sync if still supported).
- [x] 1.2 Add `trudger doctor` that performs full config validation, creates a temporary scratch directory, runs the setup hook from the invocation working directory with `TRUDGER_DOCTOR_SCRATCH_DIR` set, sets `TRUDGER_CONFIG_PATH`, ensures `TRUDGER_TASK_*`, `TRUDGER_PROMPT`, and `TRUDGER_REVIEW_PROMPT` are unset, then cleans up the scratch directory and exits 0 on success (cleanup failure is an error).
- [x] 1.3 Update sample configs to reinitialize a scratch database using local context, targeting `TRUDGER_DOCTOR_SCRATCH_DIR`.
- [x] 1.4 Document the doctor setup hook, execution environment, cleanup, and error reporting.
- [x] 1.5 Add tests that assert doctor setup hook invocation, env var behavior, full config validation, cleanup, cleanup failure behavior, error messaging, and sample config conformance.
