## 1. Implementation
- [x] 1.1 Remove duplicate spec copies under completed changes and keep a single canonical spec source
- [x] 1.2 Update `trudger` config parsing and ensure clear errors (superseded: Rust-native YAML parsing; no `yq` dependency)
- [x] 1.3 Introduce a shared command-execution helper for task/hook commands
- [x] 1.4 Centralize base config setup in tests (superseded: bats suite moved under `historical/`; Rust tests are canonical)
- [x] 1.5 Add shared queue/fixture helpers for fixture bins and update fixtures to use them
- [x] 1.6 Align docs (`README.md`, prompts) with hook-driven behavior and current config requirements
- [x] 1.7 Update/extend tests to cover config parsing, unknown-key warnings, null validation errors, and helper behavior
- [x] 1.8 Update `openspec/specs/trudger/spec.md` Purpose section to remove label-specific language and describe the generic task system (with br/bd as examples)

## 2. Quality
- [x] 2.1 Run `openspec validate refactor-trudger-config-and-docs --strict --no-interactive`
