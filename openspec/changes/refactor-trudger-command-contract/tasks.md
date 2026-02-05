## 1. Implementation
- [ ] 1.1 Update config schema to require `agent_command` and `agent_review_command` and remove `codex_command`.
- [ ] 1.2 Update command/hook invocation to pass task context via environment variables (no positional args).
- [ ] 1.3 Remove prompt substitutions; ensure agent commands receive only the relevant prompt env var (`TRUDGER_PROMPT` for solve, `TRUDGER_REVIEW_PROMPT` for review) and task context via env vars.
- [ ] 1.4 Update shell implementation to use the new env var contract with improved error reporting.
- [ ] 1.5 Update Rust implementation to use the new env var contract.
- [ ] 1.6 Update sample configs and documentation for the new keys and env var contract.
- [ ] 1.7 Update tests to cover the new env var contract and the separate review command.
