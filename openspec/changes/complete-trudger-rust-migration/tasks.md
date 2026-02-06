## 1. Entrypoint + Packaging
- [ ] 1.1 Decide the supported runtime entrypoint(s): installed `trudger` binary, repo-root `./trudger`, and any compatibility shims.
- [ ] 1.2 Update `install.sh` to install the Rust binary (not the full Bash implementation) alongside prompt files.
- [ ] 1.3 Replace the repo-root `./trudger` with a minimal shim that delegates to the Rust binary and contains no task loop logic.

## 2. Specs + Docs
- [ ] 2.1 Update the `trudger` spec to reflect Rust-native behavior as canonical (including config parsing without `yq`).
- [ ] 2.2 Update `README.md` and `--help` output to reflect the Rust canonical interface and remove Bash-only requirements.

## 3. Tests + Tooling
- [ ] 3.1 Move core behavior coverage to Rust tests and/or Rust-focused integration tests; keep shell tests only for shim behavior if needed.
- [ ] 3.2 Update git hooks/CI to run Rust quality gates (`cargo test`, any lints/format) as the primary checks.

## 4. Validation
- [ ] 4.1 Run `openspec validate complete-trudger-rust-migration --strict --no-interactive`.
