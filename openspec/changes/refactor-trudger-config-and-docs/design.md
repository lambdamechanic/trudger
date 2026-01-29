## Context
Trudger currently parses YAML config with a minimal custom parser, and docs/specs assume label-based behavior that no longer matches the hook-based workflow. Tests and fixtures repeat queue-draining and base config setup logic.

## Goals / Non-Goals
- Goals:
  - Use a real YAML parser to load config reliably.
  - Align specs/docs with actual hook-driven behavior.
  - Reduce duplication in tests and fixture scripts.
- Non-Goals:
  - Change task selection semantics beyond documented behavior.
  - Modify Codex prompt content or hook semantics.

## Decisions
- Decision: Use `yq` to parse `~/.config/trudger.yml` (any implementation acceptable).
  - Rationale: Widely available, YAML-accurate, shell-friendly.
- Decision: Keep hooks as the single source of label/comment behavior.
  - Rationale: Aligns with current implementation and test expectations.

## Risks / Trade-offs
- Additional dependency (`yq`) must be installed for runtime; error messages must be clear if missing.
- Unknown config keys will emit warnings; null values are treated as validation errors.

## Migration Plan
1. Update config parsing to use `yq` while preserving the current config schema.
2. Align specs/docs with hook-driven behavior.
3. Refactor tests and fixtures to reuse shared helpers.

## Open Questions
- Should `yq` be optional with a fallback parser, or required unconditionally?
