## 1. Implementation
- [ ] 1.1 Remove duplicate spec copies under completed changes and keep a single canonical spec source
- [ ] 1.2 Update `trudger` to parse config via `yq` and add a clear error when `yq` is missing
- [ ] 1.3 Introduce a shared command-execution helper for task/hook commands
- [ ] 1.4 Centralize base config setup in `tests/trudger_test.bats`
- [ ] 1.5 Add shared queue/fixture helpers for fixture bins and update fixtures to use them
- [ ] 1.6 Align docs (`README.md`, prompts) with hook-driven behavior and current config requirements
- [ ] 1.7 Update/extend tests to cover `yq` parsing, unknown-key warnings, null validation errors, and helper behavior
- [ ] 1.8 Update `openspec/specs/trudger/spec.md` Purpose section to remove label-specific language and describe the generic task system (with br/bd as examples)

## 2. Quality
- [ ] 2.1 Run `openspec validate refactor-trudger-config-and-docs --strict --no-interactive`
