## Context
Trudger currently passes task context to commands and hooks via positional args and performs in-process prompt substitution. This has proven brittle in the shell implementation and is difficult to make robust across agents.

## Goals / Non-Goals
- Goals:
- Move task context into explicit environment variables for commands and hooks.
- Remove Trudger-side prompt substitution and rely on env vars for context.
- Require a separate review command (`agent_review_command`) for clarity and agent-specific control.
- Non-Goals:
- Changing task selection semantics or review-loop behavior.
- Changing prompt file locations.

## Decisions
- Commands and hooks are executed without positional args; they receive context via env vars.
- Prompt templates are passed to agent commands without substitution; only the relevant prompt env var is set (`TRUDGER_PROMPT` for solve, `TRUDGER_REVIEW_PROMPT` for review).
- Task show output is provided via `TRUDGER_TASK_SHOW` (env-only), assuming typical outputs are small.
- Configuration validation requires both `agent_command` and `agent_review_command`.

## Risks / Trade-offs
- Breaking change for all existing configs and hooks that rely on positional args or `$ARGUMENTS`/`$TASK_SHOW` substitution.
- Environment variable size limits may affect large task show outputs.

## Migration Plan
1. Update configs to include `agent_review_command`.
2. Update hooks and command scripts to read `TRUDGER_*` environment variables.
3. Update prompts/agent wrappers to incorporate task context from env vars.

## Open Questions
- None.
