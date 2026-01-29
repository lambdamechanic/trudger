## 1. Implementation
- [ ] 1.1 Add `-c/--config` flag parsing and override config path in `./trudger`
- [ ] 1.2 Define behavior when the override path is missing (error) and update config loading logic
- [x] 1.3 Add sample config files for legacy/default behavior and `bv --robot-triage`
- [ ] 1.4 Update README with flag usage and sample config locations
- [ ] 1.5 Update tests to use the config flag where needed
- [ ] 1.6 Update tests to rely on `sample_configuration/*.yml` instead of re-declaring configs inline
- [ ] 1.7 Add bats tests for `--config` override success and missing-path error behavior

## 2. Validation
- [ ] 2.1 Run `bats tests/trudger_test.bats`
