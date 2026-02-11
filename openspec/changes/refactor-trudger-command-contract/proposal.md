# Change: Refactor Trudger Command Contract (Env Vars + Agent Review Command)

## Why
Prompt substitution and positional-argument contracts are brittle and are causing failures in the shell implementation. Moving task context into environment variables and separating the review command reduces quoting issues and makes Trudger agent-agnostic.

## What Changes
- **BREAKING** Replace positional-argument task context with explicit environment variables for commands and hooks.
- **BREAKING** Remove `$ARGUMENTS`/`$TASK_SHOW` prompt substitutions from Trudger; task context is provided via env vars instead.
- **BREAKING** Require `agent_review_command` in config (separate from `agent_command`).
- Rename Codex-specific requirements to agent-neutral wording in the spec.

## Impact
- Affected specs: `trudger`
- Affected code: shell `./trudger`, Rust `trudger`, config parsing/validation, docs, sample configs, tests
