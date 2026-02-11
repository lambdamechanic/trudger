## Context
Trudger currently bootstraps configuration by printing curl commands for sample YAML files. Users must manually edit YAML to change agent or tracking systems. We want an interactive wizard that is data-driven (templates stored in repo), embedded into the binary, and rerunnable safely.

## Goals / Non-Goals
- Goals:
  - Provide `trudger wizard` to generate `~/.config/trudger.yml` from selectable agent + tracking templates.
  - Store templates in repo data files and embed them into the binary at build time.
  - Re-run the wizard with defaults based on the current config, and resolve differences key-by-key.
  - Overwrite the existing config on completion while creating a timestamped backup.
  - Preserve unknown/custom keys (top-level and under `commands`/`hooks`) as commented YAML so they are not silently dropped.
- Non-Goals:
  - Building a full-screen TUI or editor for arbitrary YAML.
  - Auto-migrating arbitrary custom config structures beyond commenting out unknown keys.
  - Changing the underlying config schema in this proposal.

## Decisions
- Template data lives under a new repo directory (proposed: `config_templates/`) and is split by category:
  - `config_templates/agents/*.yml` with `id`, `label`, `description`, `agent_command`, `agent_review_command`.
  - `config_templates/tracking/*.yml` with `id`, `label`, `description`, `commands`, `hooks`.
  - `config_templates/defaults.yml` with `review_loop_limit`, `log_path`.
- Templates are embedded into the Rust binary using compile-time includes and parsed at runtime.
- The wizard is interactive-only:
  - If stdin or stdout are not TTY, `trudger wizard` exits non-zero with a clear error.
- Wizard flow:
  1. Resolve the target config path (default `~/.config/trudger.yml`, or `--config PATH` when provided).
  2. Load embedded templates and defaults.
  3. If a config exists at the target path, attempt to parse it and capture unknown/custom keys (top-level and under `commands`/`hooks`).
     - If the existing config cannot be parsed as YAML, warn the user and proceed as if no existing config is present (no preselection/merge, no unknown-key preservation) but still back up the invalid file before overwriting it.
  4. Prompt for agent template (`codex`, `claude`, `pi`), then tracking template (`br-next-task`, `bd-labels`).
  5. Build a candidate config from the selected templates plus embedded defaults for `review_loop_limit` and `log_path` when those keys are missing (no prompts for these).
  6. If a valid existing config is present, compare each known template-driven config key against the candidate config:
     - For each key whose value differs, show the current value and the proposed value and ask whether to replace that key.
     - The final config is the merge of the existing config and the candidate config based on these per-key replacement decisions.
     - Default choice for each prompt is to keep the current value.
     - `review_loop_limit` and `log_path` are not prompted; existing values are preserved when present.
  7. Append commented YAML for unknown/custom keys to the output config, preceded by a warning comment, and also print a warning to stderr.
  8. Validate the final assembled config using existing config parsing and validation; if validation fails, exit non-zero without writing.
  9. Ensure the parent directory for the target config path exists (create it if missing).
  10. If the target config exists, create a timestamped backup in the same directory, then overwrite-write the new config (atomic write preferred).
- Missing-config bootstrap output is replaced with instructions to run `trudger wizard` (no curl sample configs).

## Proposed Template Commands
These are initial templates meant to preserve current behavior. The exact command strings live in embedded data files and can be adjusted without changing code; the spec will not pin exact strings.

Agent templates:
- **codex**
  - `agent_command`: `codex --yolo exec --model gpt-5.2-codex --reasoning medium --prompt "$TRUDGER_PROMPT"`
  - `agent_review_command`: `codex --yolo exec --model gpt-5.2-codex --reasoning medium --prompt "$TRUDGER_REVIEW_PROMPT" "$@"`
- **claude**
  - `agent_command`: `claude --model sonnet --prompt "$TRUDGER_PROMPT"`
  - `agent_review_command`: `claude --model sonnet --prompt "$TRUDGER_REVIEW_PROMPT" "$@"`
- **pi**
  - `agent_command`: `pi --prompt "$TRUDGER_PROMPT"`
  - `agent_review_command`: `pi --prompt "$TRUDGER_REVIEW_PROMPT" "$@"`

Tracking templates (labels fixed per request):
- **br-next-task** (current behavior)
  - `commands.next_task`: `task_id=$(br ready --json --label trudgeable --sort priority --limit 1 | jq -r "if type == \"array\" and length > 0 then .[0].id // \"\" else \"\" end"); if [[ -z \"$task_id\" ]]; then exit 1; fi; printf \"%s\" \"$task_id\"`
  - `commands.task_show`: `br show "$TRUDGER_TASK_ID"`
  - `commands.task_status`: `br show "$TRUDGER_TASK_ID" --json | jq -r "if type == \"array\" then .[0].status // \"\" else .status // \"\" end"`
  - `commands.task_update_in_progress`: `br update "$TRUDGER_TASK_ID" "$@"`
  - `commands.reset_task`: `br update "$TRUDGER_TASK_ID" --status open`
  - `hooks.on_completed`: `br label remove "$TRUDGER_TASK_ID" "trudgeable"`
  - `hooks.on_requires_human`: `br label remove "$TRUDGER_TASK_ID" "trudgeable"; br label add "$TRUDGER_TASK_ID" "human_required"`
  - `hooks.on_doctor_setup`: `rm -rf "$TRUDGER_DOCTOR_SCRATCH_DIR/.beads"; cp -R ".beads" "$TRUDGER_DOCTOR_SCRATCH_DIR/"`
- **bd-labels** (legacy label-based)
  - Same commands as above but using `bd` instead of `br`.

Defaults:
- `review_loop_limit: 5`
- `log_path: "./.trudger.log"`

## Risks / Trade-offs
- Template command lines for `claude` and `pi` may require adjustment based on the actual CLI; storing them in data files keeps this editable.
- Per-key merge prompts add complexity, but avoid surprising overwrites when users hand-edit configs and make reruns safer.

## Migration Plan
- Update missing-config output and docs to point to `trudger wizard`.
- Replace or retire `sample_configuration/` in favor of `config_templates/`.
- Add wizard tests for: TTY requirement, backup creation, per-key merge prompts, unknown-key commenting, and config validation failures.

## Open Questions
- None.
