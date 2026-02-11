## 1. Implementation
- [x] 1.1 Add `-c/--config` flag parsing in the Rust CLI; ensure the repo-root `./trudger` shim forwards args unchanged.
- [x] 1.2 Define behavior when the override path is missing (error) and update config loading logic
- [x] 1.3 Add sample config files for legacy/default behavior and `bv --robot-triage`
- [x] 1.4 Update README with flag usage and sample config locations
- [x] 1.5 Update tests to use the config flag where needed
- [x] 1.6 Update tests to rely on `sample_configuration/*.yml` instead of re-declaring configs inline
- [x] 1.7 Add Rust tests for `--config` override success and missing-path error behavior

## 2. Validation
- [x] 2.1 Run `cargo test`
