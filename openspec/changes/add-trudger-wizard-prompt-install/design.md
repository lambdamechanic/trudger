## Context
Trudger's task-processing mode currently loads prompt content from fixed filesystem paths under `~/.codex/prompts/`. The interactive wizard generates `~/.config/trudger.yml` but does not manage prompt installation. This leads to a common "generated config, but `trudger` fails on missing prompts" footgun.

## Goals / Non-Goals
Goals:
- Make `trudger wizard` a "one stop" bootstrap for first-time users, including prompt installation.
- Preserve current runtime behavior: Trudger still reads prompts from the same fixed locations.
- Keep prompt installation safe (don't clobber existing prompts without an explicit user choice).

Non-goals:
- Changing where Trudger loads prompts from (making prompt paths configurable is a separate change).
- Supporting non-interactive wizard flows (wizard remains TTY-only).

## Proposed UX
During `trudger wizard`:
- Detect whether `~/.codex/prompts/trudge.md` and `~/.codex/prompts/trudge_review.md` exist.
- If one or both are missing:
  - Prompt to install missing prompts (default Yes).
  - On yes: create the directory (if needed) and write only the missing prompt files.
  - On no: continue writing config and print clear follow-up instructions indicating prompt files are still required for task-processing mode.
- If one or both exist and differ from the embedded defaults:
  - Prompt to overwrite each differing prompt file (default No; require explicit confirmation).
  - (Optional) show a unified diff (or a short "diff preview") before the overwrite prompt.
  - Create a timestamped backup before overwriting each prompt file.
- If both exist and match the embedded prompt content: no action and no prompts.

The wizard SHOULD print a summary at the end:
- "Wrote config to ..."
- "Installed prompts to ..." or "Prompts unchanged" or "Prompts not installed (skipped)".

## Data Source For Prompts
Since `trudger wizard` runs from the installed Rust binary, it MUST NOT assume the repo checkout exists.
Therefore, prompt sources SHOULD be embedded into the binary at build time (for example via `include_str!`).

`./install.sh` remains a repo-local tool that installs prompt sources from `prompts/` for users working from a checkout.
Keeping both paths reduces friction: installed-binary users use the wizard; repo users can still use the script.

## Safety / Overwrite Rules
- Never overwrite an existing prompt file without interactive confirmation.
- If the prompt file exists and is identical, do nothing.
- If diff rendering fails or is too large, fall back to a simple "file differs" message and still prompt for overwrite.
- Prefer a safe default: overwrite prompts should default to "keep existing".

## Error Handling
- If the user opted into installing/updating prompts and an IO error occurs (directory creation, read, backup, or write), the wizard should exit non-zero and print a clear error naming the failing path.
- Partial success is acceptable (for example one prompt written, one failed) as long as the wizard reports it and does not silently claim success.
