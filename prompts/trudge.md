---
description: Work a task in the trudger loop.
argument-hint: task-id
---

Task ID: $ARGUMENTS
Task details:
$TASK_SHOW
Work the specified task to completion in the current repo. The task details from `commands.task_show` are provided above.

**Requirements**
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Follow the repo's AGENTS.md workflow and any referenced specs.
- Trudger reads `~/.config/trudger.yml` (parsed via `yq`); required keys include `codex_command`, `review_loop_limit`, `log_path`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `hooks.on_completed`, and `hooks.on_requires_human`; `commands.next_task` is required unless manual task IDs are supplied.
- Keep changes minimal and aligned to the task scope.
- Run the relevant tests/quality gates.
- Commit and push your changes.
- Do not close the task or apply `requires-human` in this step; leave that for the review prompt.

**If blocked**
- Leave any intermediate notes you believe will help review, but do not label the task.
