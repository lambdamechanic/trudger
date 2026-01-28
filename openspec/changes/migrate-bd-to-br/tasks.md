## 1. Spec updates
- [x] 1.1 Update `openspec/specs/trudger/spec.md` to reference br in purpose and requirements.
- [x] 1.2 Update open change specs/proposals that reference bd to br for consistency.

## 2. Docs and prompts
- [x] 2.1 Update `README.md`, `AGENTS.md`, `prompts/*.md`, and `sample_configuration/*.yml` to use br commands.
- [x] 2.2 Replace legacy sync commands with `br sync --flush-only` and add manual git steps; add the non-invasive br note after beads headers.

## 3. Script and tests
- [x] 3.1 Update `trudger` usage/config bootstrap copy and any default commands to br.
- [x] 3.2 Update tests and fixtures to expect br commands.

## 4. Validation
- [x] 4.1 Run `skills/bd-to-br-migration/scripts/find-bd-refs.sh .` and ensure no bd refs remain (excluding migration skill docs).
- [x] 4.2 Run the test suite.
