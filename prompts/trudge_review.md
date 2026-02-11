---
description: Review a task after trudger work and update task state.
argument-hint: task-id
---

Task context is available via environment variables:
- TRUDGER_TASK_ID
- TRUDGER_TASK_SHOW
- TRUDGER_TASK_STATUS
- TRUDGER_CONFIG_PATH
Review the specified task after the solve step and update the task state accordingly. The task details from `commands.task_show` are available in `TRUDGER_TASK_SHOW`.

**Requirements**
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Verify acceptance criteria against the implemented changes and tests.
- Trudger reads `~/.config/trudger.yml` (parsed natively in Rust); required keys include `agent_command`, `agent_review_command`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_status`, `hooks.on_completed`, and `hooks.on_requires_human`; `commands.next_task` is required unless manual task IDs are supplied; `log_path` is optional (missing/empty disables logging).

**If the task is complete**
- Close the task with the configured task-close command (for example `br close <id>`).
- Remove the `trudgeable` label (or equivalent) via your task system.

**If human input is required**
- Add a `human_required` label (or equivalent).
- Remove the `trudgeable` label (or equivalent).
- Add a task comment with a clear, specific question or decision needed.
- Update task notes with a concise summary of the blocker, what was attempted, and what is needed next.

**Response**
- Summarize what you verified and which br updates you made.
