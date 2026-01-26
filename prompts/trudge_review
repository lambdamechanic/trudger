---
description: Review a bd task after trudger work and update bd state.
argument-hint: bd-task-id
---

$ARGUMENTS
Review the specified bd task after the solve step and update bd state accordingly.

**Requirements**
- Load the task details with `bd show <id> --json`.
- Verify acceptance criteria against the implemented changes and tests.

**If the task is complete**
- Close the task with `bd close <id>`.
- Remove the `trudgeable` label.

**If human input is required**
- Add a `requires-human` label.
- Remove the `trudgeable` label.
- Add a bd comment with a clear, specific question or decision needed.
- Update task notes with a concise summary of the blocker, what was attempted, and what is needed next.

**Response**
- Summarize what you verified and which bd updates you made.
