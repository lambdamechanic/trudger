# Change: Add config flag and sample configs for trudger

## Why
Tests and alternate task-selection workflows need a deterministic way to inject configuration without relying on `~/.config/trudger.yml`.

## What Changes
- Add a `-c/--config` flag to override the default config path.
- Add sample config files for legacy behavior and `bv --robot-triage` workflows.
- Document the flag and sample configs.

## Impact
- Affected specs: `trudger`
- Affected code: `./trudger`, `tests/`, `README.md`, new sample config files.
