## 1. Implementation
- [x] 1.1 Update `trudger` to require a config file and print a curl bootstrap command when missing.
- [x] 1.2 Remove in-script defaults that are now provided by config.
- [x] 1.3 Update tests for the missing-config bootstrap behavior.
- [x] 1.4 Update README to describe the bootstrap flow and sample configs.
- [x] 1.5 Remove label-specific behavior and default task selection from `trudger`, requiring hooks/next-task config.
- [x] 1.6 Update tests to use `sample_configuration` for label-driven behavior.
- [x] 1.7 Externalize all task commands (`next_task`, `task_show`, `task_update_in_progress`) into config.
- [x] 1.8 Update sample configs to include `commands.*` entries.
- [x] 1.9 Update tests to use configured command wrappers for show/update.
- [x] 1.10 Update README to document the command expectations and exit code semantics.

## 2. Validation
- [x] 2.1 Run `bats tests/trudger_test.bats`.
