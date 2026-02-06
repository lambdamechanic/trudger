---
description: Work a task in the trudger loop.
argument-hint: task-id
---

Task context is available via environment variables:
- TRUDGER_TASK_ID
- TRUDGER_TASK_SHOW
- TRUDGER_TASK_STATUS
- TRUDGER_CONFIG_PATH
Work the specified task to completion in the current repo. The task details from `commands.task_show` are available in `TRUDGER_TASK_SHOW`.

**Requirements**
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Follow the repo's AGENTS.md workflow and any referenced specs.
- Trudger reads `~/.config/trudger.yml` (parsed natively in Rust); required keys include `agent_command`, `agent_review_command`, `review_loop_limit`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `commands.reset_task`, `hooks.on_completed`, and `hooks.on_requires_human`; `commands.next_task` is required unless manual task IDs are supplied; `log_path` is optional (missing/empty disables logging).
- Keep changes minimal and aligned to the task scope.
- Run the relevant tests/quality gates.
- Commit and push your changes.
- Do not close the task or apply `requires-human` in this step; leave that for the review prompt.

**If blocked**
- Leave any intermediate notes you believe will help review, but do not label the task.
