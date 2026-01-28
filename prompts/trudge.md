---
description: Work a br task in the trudger loop.
argument-hint: br-task-id
---

$ARGUMENTS
Work the specified br task to completion in the current repo.

**Requirements**
- Load the task details with `br show <id> --json`.
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Follow the repo's AGENTS.md workflow and any referenced specs.
- Keep changes minimal and aligned to the task scope.
- Run the relevant tests/quality gates.
- Commit and push your changes.
- Do not close the task or apply `requires-human` in this step; leave that for the review prompt.

**If blocked**
- Leave any intermediate notes you believe will help review, but do not label the task.
