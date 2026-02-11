## 1. Implementation
- [x] 1.1 Replace config key contract with `commands.task_update_status` and remove `commands.reset_task`.
- [x] 1.2 Add `TRUDGER_TARGET_STATUS` env propagation for configured commands/hooks.
- [x] 1.3 Refactor run loop and doctor to use env-based status updates for all status transitions.
- [x] 1.4 Update wizard/templates/sample configs/docs/prompts to new config key and env contract.
- [x] 1.5 Migrate `~/.config/trudger.yml` in place.

## 2. Validation
- [x] 2.1 Update and run Rust tests.
- [x] 2.2 Run project quality gates and push.
