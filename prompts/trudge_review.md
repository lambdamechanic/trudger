---
description: Review a task after trudger work and update task state.
argument-hint: task-id
---

Task ID: $ARGUMENTS
Task details:
$TASK_SHOW
Review the specified task after the solve step and update the task state accordingly. The task details from `commands.task_show --json` are provided above.

**Requirements**
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Verify acceptance criteria against the implemented changes and tests.
- Trudger reads `~/.config/trudger.yml` (parsed via `yq`); required keys include `codex_command`, `review_loop_limit`, `log_path`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `hooks.on_completed`, and `hooks.on_requires_human`.

**If the task is complete**
- Close the task with the configured task-close command (for example `br close <id>`).
- Remove the `trudgeable` label (or equivalent) via your task system.

**If human input is required**
- Add a `requires-human` label (or equivalent).
- Remove the `trudgeable` label (or equivalent).
- Add a task comment with a clear, specific question or decision needed.
- Update task notes with a concise summary of the blocker, what was attempted, and what is needed next.

**Response**
- Summarize what you verified and which br updates you made.
