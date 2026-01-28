---
description: Review a br task after trudger work and update br state.
argument-hint: br-task-id
---

$ARGUMENTS
Review the specified br task after the solve step and update br state accordingly.

**Requirements**
- Load the task details with `br show <id> --json`.
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Verify acceptance criteria against the implemented changes and tests.

**If the task is complete**
- Close the task with `br close <id>`.
- Remove the `trudgeable` label.

**If human input is required**
- Add a `requires-human` label.
- Remove the `trudgeable` label.
- Add a br comment with a clear, specific question or decision needed.
- Update task notes with a concise summary of the blocker, what was attempted, and what is needed next.

**Response**
- Summarize what you verified and which br updates you made.
