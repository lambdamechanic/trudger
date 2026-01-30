---
description: Work a task in the trudger loop.
argument-hint: task-details
---

$ARGUMENTS
Work the specified task to completion in the current repo. The task details from `commands.task_show --json` are provided above.

**Requirements**
- The task details from `commands.task_show --json` are provided above; only re-run task-show if you need to refresh them.
- Agent binaries available: `br`, `codex`, `jq`, `beads_rust`, `MCPShell`.
- Follow the repo's AGENTS.md workflow and any referenced specs.
- Trudger reads `~/.config/trudger.yml` (parsed via `yq`); required keys include `codex_command`, `review_loop_limit`, `log_path`, `commands.next_task`, `commands.task_show`, `commands.task_status`, `commands.task_update_in_progress`, `hooks.on_completed`, and `hooks.on_requires_human`.
- Keep changes minimal and aligned to the task scope.
- Run the relevant tests/quality gates.
- Commit and push your changes.
- Do not close the task or apply `requires-human` in this step; leave that for the review prompt.

**If blocked**
- Leave any intermediate notes you believe will help review, but do not label the task.
