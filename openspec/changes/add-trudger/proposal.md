# Change: Add trudger automation script

## Why
We need a lightweight automation loop that repeatedly selects the next trudgeable bd task and drives Codex through a standardized solve + review flow, while safely halting on external errors.

## What Changes
- Add a root-level executable script `./trudger` that loops over trudgeable bd tasks and orchestrates Codex sessions.
- Define consistent behavior for Codex prompting, task closure, and requires-human escalation.
- Require prompt file presence and validate Codex-driven bd updates after each review.

## Impact
- Affected specs: `trudger`.
- Affected code/config: `./trudger`, Codex prompt files under `~/.codex/prompts/`.
