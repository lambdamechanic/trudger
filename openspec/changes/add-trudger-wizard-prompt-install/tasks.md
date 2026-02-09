## 1. Prompt Data (Required For Wizard Install/Update)
- [ ] 1.1 Embed the default prompt sources in the Rust binary (same content as `prompts/trudge.md` and `prompts/trudge_review.md`).
- [ ] 1.2 Add prompt file install/update helpers (compare vs embedded defaults, write missing, overwrite with confirmation, timestamped backup creation for overwrites).

## 2. Wizard Prompt Installation UX
- [ ] 2.1 Detect prompt state (`missing`, `matches_default`, `differs`) for each required prompt path.
- [ ] 2.2 If one or both prompts are missing, offer to install missing prompts (default Yes).
- [ ] 2.3 If any existing prompt differs from defaults, offer to overwrite each differing prompt (default No; require explicit confirmation).
- [ ] 2.4 Ensure prompt install/update (when accepted) occurs before config write; on failure, exit non-zero and do not write the config file.
- [ ] 2.5 Print a clear end-of-wizard summary for config + prompts (installed/updated/unchanged/skipped) and actionable follow-up instructions when prompts remain missing.

## 3. Tests
- [ ] 3.1 Add tests covering install-missing, overwrite-differing, skip flows, backup behavior (if implemented), and IO/permission error handling.

## 4. Docs
- [ ] 4.1 Update `README.md` and CLI help text to reflect wizard-managed prompt installation while keeping `./install.sh` as an alternative.

## 5. Validation
- [ ] 5.1 Run `cargo test` (or `cargo nextest run` if available) and fix failures.
- [ ] 5.2 Run `cargo fmt` and `cargo clippy` (or the repo quality gates) and fix failures.
- [ ] 5.3 Run `openspec validate add-trudger-wizard-prompt-install --strict --no-interactive`.
