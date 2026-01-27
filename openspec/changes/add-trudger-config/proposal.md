## Change: Add trudger config file and hooks

## Why
Trudger needs configurable Codex invocation, task selection, and outcome handling so teams can customize workflows without editing the script.

## What Changes
- Add `~/.config/trudger.yml` for configuration and warn when it is missing.
- Configure the Codex invocation command line from config.
- Configure the next-task selection command from config.
- Add optional hooks for task completion and requires-human outcomes, passing the task id as a parameter.
- Make `trudgeable` and `requires-human` labels optional via config defaults (used only when hooks are not configured).
- Remove `TRUDGER_*` environment variable configuration.

## Impact
- Affected specs: `trudger`
- Affected code: `./trudger`
