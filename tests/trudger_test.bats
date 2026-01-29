#!/usr/bin/env bats

setup() {
  ROOT_DIR="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"
  PATH="${ROOT_DIR}/tests/fixtures/bin:${PATH}"
  if [[ -z "${BATS_TEST_TMPDIR:-}" ]]; then
    BATS_TEST_TMPDIR="$(mktemp -d)"
  fi
}

run_trudger() {
  run "${ROOT_DIR}/trudger"
}

should_run_codex_tests() {
  [[ "${TRUDGER_TEST_RUN_CODEX:-0}" == "1" ]]
}

create_prompts() {
  local temp_dir="$1"
  mkdir -p "${temp_dir}/.codex/prompts"
  printf '%s\n' "\$ARGUMENTS" > "${temp_dir}/.codex/prompts/trudge.md"
  printf '%s\n' "\$ARGUMENTS" > "${temp_dir}/.codex/prompts/trudge_review.md"
}

write_config() {
  local temp_dir="$1"
  mkdir -p "${temp_dir}/.config"
  cat > "${temp_dir}/.config/trudger.yml"
}

copy_sample_config() {
  local temp_dir="$1"
  local name="$2"
  mkdir -p "${temp_dir}/.config"
  cp "${ROOT_DIR}/sample_configuration/${name}.yml" "${temp_dir}/.config/trudger.yml"
}

@test "missing prompt files cause a clear error" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-prompts"
  mkdir -p "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  HOME="$temp_dir" \
    BR_MOCK_READY_JSON='[]' \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"Missing prompt file"* ]]
}

@test "missing config prints bootstrap instructions" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-config"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"

  HOME="$temp_dir" \
    BR_MOCK_READY_JSON='[]' \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"Missing config file"* ]]
  [[ "$output" == *"trudgeable-with-hooks.yml"* ]]
  [[ "$output" == *"robot-triage.yml"* ]]
}

@test "missing commands.next_task errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-next-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  HOME="$temp_dir" \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.next_task must not be empty"* ]]
}

@test "missing commands.task_show errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-task-show"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  HOME="$temp_dir" \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.task_show must not be empty"* ]]
}

@test "missing commands.task_update_in_progress errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-task-update"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  HOME="$temp_dir" \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.task_update_in_progress must not be empty"* ]]
}

@test "missing on_completed hook errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-on-completed"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: ""
  on_requires_human: "hook --needs-human"
CONFIG

  HOME="$temp_dir" \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"hooks.on_completed must not be empty"* ]]
}

@test "missing on_requires_human hook errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-on-requires-human"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: ""
CONFIG

  HOME="$temp_dir" \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"hooks.on_requires_human must not be empty"* ]]
}

@test "no tasks exits zero without codex" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-tasks"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "open task is treated as ready" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/open-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local codex_log="${temp_dir}/codex.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' 'tr-42' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"closed","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_UPDATE_OUTPUT="UPDATE_IGNORED" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "SHOW_PAYLOAD" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "UPDATE_IGNORED" "$codex_log"
  [ "$status" -ne 0 ]
}

@test "closed task removes trudgeable label" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/closed-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  copy_sample_config "$temp_dir" "trudgeable-with-hooks"

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-1"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BR_MOCK_READY_QUEUE="$ready_queue" \
    BR_MOCK_SHOW_JSON='[{"id":"tr-1","status":"closed","labels":["trudgeable"]}]' \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-1 trudgeable" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec " "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec resume --last " "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "tr-1" "$codex_log"
  [ "$status" -eq 0 ]
}

@test "robot-triage sample config selects task via bv" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/robot-triage"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  copy_sample_config "$temp_dir" "robot-triage"

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"
  local robot_queue="${temp_dir}/robot.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' '{"id":"tr-77"}' '' > "$robot_queue"
  printf '%s\n' \
    '[{"id":"tr-77","status":"ready","labels":[]}]' \
    '[{"id":"tr-77","status":"ready","labels":[]}]' \
    '[{"id":"tr-77","status":"ready","labels":[]}]' \
    '[{"id":"tr-77","status":"closed","labels":[]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    BV_MOCK_ROBOT_NEXT_QUEUE="$robot_queue" \
    BR_MOCK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "update tr-77 --status in_progress" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec " "$codex_log"
  [ "$status" -eq 0 ]
}

@test "robot-triage skips non-ready tasks from bv" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/robot-triage-skip"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  copy_sample_config "$temp_dir" "robot-triage"

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"
  local robot_queue="${temp_dir}/robot.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' '{"id":"tr-1"}' '{"id":"tr-2"}' '' > "$robot_queue"
  printf '%s\n' \
    '[{"id":"tr-1","status":"in_progress","labels":[]}]' \
    '[{"id":"tr-2","status":"ready","labels":[]}]' \
    '[{"id":"tr-2","status":"ready","labels":[]}]' \
    '[{"id":"tr-2","status":"ready","labels":[]}]' \
    '[{"id":"tr-2","status":"closed","labels":[]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    BV_MOCK_ROBOT_NEXT_QUEUE="$robot_queue" \
    BR_MOCK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "update tr-2 --status in_progress" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "update tr-1 --status in_progress" "$br_log"
  [ "$status" -ne 0 ]
}

@test "uses configured codex command for solve and review" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/codex-config"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec --custom"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local codex_log="${temp_dir}/codex.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' 'tr-10' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"closed","labels":[]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --custom" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --custom resume --last" "$codex_log"
  [ "$status" -eq 0 ]
}

@test "hooks honor shell quoting with task id substitution" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/hook-quoting"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "bash -lc 'hook --done \"$1\"'"
  on_requires_human: "hook --needs-human"
CONFIG

  local hook_log="${temp_dir}/hook.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' 'tr-55' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"closed","labels":[]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    HOOK_MOCK_LOG="$hook_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "--done tr-55" "$hook_log"
  [ "$status" -eq 0 ]
}

@test "requires-human updates labels" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/requires-human"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  copy_sample_config "$temp_dir" "trudgeable-with-hooks"

  local br_log="${temp_dir}/br.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-2"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BR_MOCK_READY_QUEUE="$ready_queue" \
    BR_MOCK_SHOW_JSON='[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    BR_MOCK_LOG="$br_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-2 trudgeable" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label add tr-2 human-required" "$br_log"
  [ "$status" -eq 0 ]
}

@test "next-task command selects id using first token" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local br_log="${temp_dir}/br.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' 'tr-20 extra' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"closed","labels":["trudgeable"]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    run_trudger

  [ "$status" -eq 0 ]
}

@test "next-task command returning empty exits zero" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-empty"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "next-task command exit 1 exits zero" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-exit-1"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_EXIT_CODE=1 \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "next-task command non-zero exit errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-exit-2"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  HOME="$temp_dir" \
    NEXT_TASK_EXIT_CODE=2 \
    run_trudger

  [ "$status" -eq 2 ]
  [[ "$output" == *"next_task command failed with exit code 2"* ]]
}

@test "env config is ignored" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/env-ignored"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local br_log="${temp_dir}/br.log"
  local show_queue="${temp_dir}/show.queue"
  local next_task_queue="${temp_dir}/next-task.queue"
  printf '%s\n' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'tr-99' '' > "$next_task_queue"

  HOME="$temp_dir" \
    TRUDGER_NEXT_CMD='next-task' \
    TRUDGER_REVIEW_LOOPS=0 \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    run_trudger

  [ "$status" -eq 0 ]
}

@test "codex exec failure aborts" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/codex-fail"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
CONFIG

  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' 'tr-4' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"closed","labels":[]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    CODEX_MOCK_FAIL_ON=exec \
    run_trudger

  [ "$status" -ne 0 ]
}
