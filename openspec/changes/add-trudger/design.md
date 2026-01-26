## Context
We want a minimal shell script that automates a bd + Codex loop for tasks labeled `trudgeable`. The script should start/resume a single Codex session, issue a solve prompt, then issue a review prompt to decide closure or escalation. It must validate that Codex updated bd state after the review step and error if not.

## Goals / Non-Goals
- Goals:
  - Provide a single executable `./trudger` entrypoint.
  - Process the lowest-priority ready `trudgeable` bd task each cycle.
  - Use Codex prompts stored in `~/.codex/prompts/trudge` and `~/.codex/prompts/trudge_review`.
  - Update bd state based on Codex review (close, or mark requires-human).
  - Fail fast when Codex does not close or escalate a task after review.
- Non-Goals:
  - Parallel processing of multiple tasks.
  - Building a general task runner framework.
  - Managing or authoring Codex prompt content in-repo.

## Decisions
- Script location and interface:
  - Place the script at `./trudger` (repo root) and commit it as executable.
- Task selection:
  - Use bd readiness and priority to select the lowest-priority ready task with label `trudgeable`.
- Codex session handling:
  - Use `codex exec` for the initial call and `codex exec resume --last` for subsequent calls.
  - Pass the bd id via `/prompt:trudge <id>` and `/prompt:trudge_review <id>`.
- Error handling:
  - Treat missing prompt files as a startup error with a clear message.
  - Treat missing bd updates after review (no close, no requires-human) as a task failure.

## Risks / Trade-offs
- Relying on external prompt files can drift from repo expectations.
- Using a single session avoids concurrency but may carry context between tasks.

## Migration Plan
- Add the proposal and spec deltas.
- Implement the script after approval and validate behavior with a sample trudgeable task.

## Open Questions
- None.
