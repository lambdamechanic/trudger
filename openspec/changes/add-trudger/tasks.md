## 1. Script scaffolding
- [ ] 1.1 Add `./trudger` with executable permissions and a clear usage header.
- [ ] 1.2 Validate required prompt files exist before any work begins.

## 2. bd task selection
- [ ] 2.1 Query bd for ready tasks labeled `trudgeable` and select the lowest-priority item.
- [ ] 2.2 Exit 0 when no trudgeable tasks remain.

## 3. Codex orchestration
- [ ] 3.1 Start a new Codex exec session with `/prompt:trudge <id>`.
- [ ] 3.2 Resume the same session with `/prompt:trudge_review <id>`.

## 4. bd state updates
- [ ] 4.1 On success, close the task and remove the `trudgeable` label.
- [ ] 4.2 On requires-human, add a comment, update notes, remove `trudgeable`, and add `requires-human`.
- [ ] 4.3 Treat lack of close or requires-human as an error and exit non-zero.

## 5. Validation
- [ ] 5.1 Run a dry pass with a mock trudgeable task and confirm it exits when none remain.
- [ ] 5.2 Confirm missing prompt files produce a clear startup error.
