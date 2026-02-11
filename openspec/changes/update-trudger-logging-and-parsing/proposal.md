# Change: Update trudger logging, parsing, and reexec behavior

## Why
Recent review feedback (P1/P2) highlighted gaps in logging integrity, error handling consistency, and config parse error clarity. We need to make these behaviors explicit in OpenSpec before implementation changes.

## What Changes
- Specify YAML parse failure handling for invalid configs.
- Specify prompt substitution safety for `$ARGUMENTS` and `$TASK_SHOW` (preserve special characters).
- Specify logging semantics: single-line entries with control-character escaping and full command text.
- Specify error-trap behavior (no quit log without exiting).
- Specify reexec path resolution behavior.

## Impact
- Affected specs: `openspec/specs/trudger/spec.md`
- Affected code: `trudger`, `tests/trudger_test.bats` (and any logging helpers)
