## 1. Script scaffolding
- [x] 1.1 Add `./trudger` with executable permissions and a clear usage header.
- [x] 1.2 Validate required prompt files exist before any work begins.

## 2. br task selection
- [x] 2.1 Query br for ready tasks labeled `trudgeable` and select the lowest-priority item.
- [x] 2.2 Exit 0 when no trudgeable tasks remain.

## 3. Codex orchestration
- [x] 3.1 Start a new Codex exec session using the rendered trudge prompt (replace `$ARGUMENTS` with the id).
- [x] 3.2 Resume the same session using the rendered review prompt.

## 4. br state updates
- [x] 4.1 On success, close the task and remove the `trudgeable` label.
- [x] 4.2 On requires-human, remove `trudgeable` and add `requires-human`.
- [x] 4.3 Treat lack of close or requires-human as an error and exit non-zero.

## 5. Validation
- [x] 5.1 Run a dry pass with a mock trudgeable task and confirm it exits when none remain.
- [x] 5.2 Confirm missing prompt files produce a clear startup error.
