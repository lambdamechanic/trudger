# Change: Require config bootstrap with curl helper

## Why
The current default-in-code configuration hides behavior and makes it harder to share consistent setups. We want a single source of truth (a config file) and a clear bootstrap path when it is missing.

## What Changes
- Require `~/.config/trudger.yml` to exist before starting work.
- When missing, emit curl commands for each sample config (with header docs) and exit non-zero.
- Remove hard-coded defaults in the script, relying on config values instead.
- Remove label-specific behavior and default task selection from the script; configuration must specify selection and label updates via hooks.
- Externalize all task operations (next, show, update) into config commands; Trudger must not invoke `br` directly.
- Update tests and documentation to reflect the new bootstrap flow.

## Impact
- Affected specs: `trudger`
- Affected code: `./trudger`, `README.md`, `tests/`
