# Change: Refactor Trudger CLI Args (Subcommands + -t Tasks)

## Why
Trudger currently treats all non-option arguments as manual task ids, which makes it hard to add subcommands like `trudger doctor` without ambiguity. Moving manual task ids behind an explicit `-t/--task` flag makes the CLI unambiguous and easier to extend.

## What Changes
- **BREAKING** Stop accepting positional task ids. Manual task ids are supplied via `-t/--task`.
- Add CLI subcommand parsing so `trudger doctor` is a first-class mode.
- Support multiple `-t` uses and comma-separated values (preserving order).

## Impact
- Affected specs: `trudger`
- Affected code: Rust CLI argument parsing, usage text, docs, tests (shell wrapper may be updated later)

## Dependencies
- `add-trudger-doctor-scratch-db` (doctor mode is part of the CLI surface)
