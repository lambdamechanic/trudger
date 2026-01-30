#!/usr/bin/env bats

setup() {
  ROOT_DIR="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"
  PATH="${ROOT_DIR}/tests/fixtures/bin:${PATH}"
  if [[ -z "${BATS_TEST_TMPDIR:-}" ]]; then
    BATS_TEST_TMPDIR="$(mktemp -d)"
  fi
}

run_trudger() {
  local config_path=""
  local prev=""
  for arg in "$@"; do
    if [[ "$prev" == "--config" || "$prev" == "-c" ]]; then
      config_path="$arg"
      break
    fi
    case "$arg" in
      --config=*)
        config_path="${arg#*=}"
        break
        ;;
    esac
    prev="$arg"
  done
  if [[ -n "$config_path" && -n "${HOME:-}" && "${TRUDGER_TEST_SKIP_CONFIG_COPY:-}" != "1" ]]; then
    if [[ -x /bin/mkdir ]]; then
      /bin/mkdir -p "${HOME}/.config"
    else
      mkdir -p "${HOME}/.config"
    fi
    if [[ -x /bin/cp ]]; then
      /bin/cp "$config_path" "${HOME}/.config/trudger.yml"
    else
      cp "$config_path" "${HOME}/.config/trudger.yml"
    fi
  fi
  run "${ROOT_DIR}/trudger" "$@"
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

yaml_quote() {
  local value="$1"
  value=${value//\'/\'\'}
  printf "'%s'" "$value"
}

write_base_config() {
  local temp_dir="$1"
  local config_path="${temp_dir}/trudger.yml"
  local codex_command="${BASE_CODEX_COMMAND-"codex --yolo exec"}"
  local next_task_command="${BASE_NEXT_TASK_COMMAND-"next-task"}"
  local task_show_command="${BASE_TASK_SHOW_COMMAND-"task-show"}"
  local task_status_command="${BASE_TASK_STATUS_COMMAND-"task-status"}"
  local task_update_command="${BASE_TASK_UPDATE_COMMAND-"task-update"}"
  local review_loop_limit="${BASE_REVIEW_LOOP_LIMIT-"5"}"
  local log_path="${BASE_LOG_PATH-"./.trudger.log"}"
  local hook_on_completed="${BASE_HOOK_ON_COMPLETED-"hook --done"}"
  local hook_on_requires_human="${BASE_HOOK_ON_REQUIRES_HUMAN-"hook --needs-human"}"
  local extra_config="${BASE_EXTRA_CONFIG-}"

  {
    printf 'codex_command: %s\n' "$(yaml_quote "$codex_command")"
    printf 'commands:\n'
    if [[ -n "$next_task_command" ]]; then
      printf '  next_task: %s\n' "$(yaml_quote "$next_task_command")"
    fi
    printf '  task_show: %s\n' "$(yaml_quote "$task_show_command")"
    printf '  task_status: %s\n' "$(yaml_quote "$task_status_command")"
    printf '  task_update_in_progress: %s\n' "$(yaml_quote "$task_update_command")"
    if [[ -n "$review_loop_limit" ]]; then
      printf 'review_loop_limit: %s\n' "$review_loop_limit"
    else
      printf "review_loop_limit: ''\n"
    fi
    printf 'log_path: %s\n' "$(yaml_quote "$log_path")"
    printf 'hooks:\n'
    printf '  on_completed: %s\n' "$(yaml_quote "$hook_on_completed")"
    printf '  on_requires_human: %s\n' "$(yaml_quote "$hook_on_requires_human")"
    if [[ -n "$extra_config" ]]; then
      printf '%s\n' "$extra_config"
    fi
  } > "${config_path}"
  printf '%s' "${config_path}"
}

write_config() {
  local temp_dir="$1"
  local config_path="${temp_dir}/trudger.yml"
  cat > "${config_path}"
  printf '%s' "${config_path}"
}

copy_sample_config() {
  local temp_dir="$1"
  local name="$2"
  local config_path="${ROOT_DIR}/sample_configuration/${name}.yml"
  printf '%s' "${config_path}"
}

make_minimal_path() {
  local temp_dir="$1"
  local bin_dir="${temp_dir}/bin"
  mkdir -p "$bin_dir"
  local cmd
  for cmd in bash awk sed date hostname jq cat; do
    ln -sf "$(command -v "$cmd")" "${bin_dir}/${cmd}"
  done
  printf '%s' "$bin_dir"
}

@test "missing prompt files cause a clear error" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-prompts"
  mkdir -p "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    BR_MOCK_READY_JSON='[]' \
    run_trudger -c "$config_path"

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

@test "config flag uses provided file" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/config-override"
  mkdir -p "${temp_dir}/.config"
  create_prompts "$temp_dir"

  cat > "${temp_dir}/.config/trudger.yml" <<'EOF'
codex_command: "codex --yolo exec --default"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
EOF

  local override_config_path
  override_config_path="$(write_config "$temp_dir" <<'EOF'
codex_command: "codex --yolo exec --override"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update"
review_loop_limit: 5
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
EOF
)"

  local codex_log="${temp_dir}/codex.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-10' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    TRUDGER_TEST_SKIP_CONFIG_COPY=1 \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger --config "$override_config_path"

  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --override" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --default" "$codex_log"
  [ "$status" -ne 0 ]
}

@test "missing -c value errors with usage" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-config-flag"
  mkdir -p "$temp_dir"

  HOME="$temp_dir" \
    run_trudger -c

  [ "$status" -ne 0 ]
  [[ "$output" == *"Missing value for -c"* ]]
  [[ "$output" == *"Usage:"* ]]
}

@test "missing commands.next_task errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-next-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_NEXT_TASK_COMMAND="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.next_task must not be empty"* ]]
}

@test "missing commands.task_show errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-task-show"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_TASK_SHOW_COMMAND="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.task_show must not be empty"* ]]
}

@test "missing commands.task_status errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-task-status"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_TASK_STATUS_COMMAND="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.task_status must not be empty"* ]]
}

@test "missing commands.task_update_in_progress errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-task-update"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_TASK_UPDATE_COMMAND="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"commands.task_update_in_progress must not be empty"* ]]
}

@test "missing on_completed hook errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-on-completed"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_HOOK_ON_COMPLETED="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"hooks.on_completed must not be empty"* ]]
}

@test "missing on_requires_human hook errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-on-requires-human"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_HOOK_ON_REQUIRES_HUMAN="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"hooks.on_requires_human must not be empty"* ]]
}

@test "empty codex_command errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/empty-codex-command"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_CODEX_COMMAND="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"codex_command must not be empty"* ]]
}

@test "empty log_path errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/empty-log-path"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_LOG_PATH="" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"log_path must not be empty"* ]]
}

@test "null review_loop_limit errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/null-review-limit"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"

  config_path="$(write_config "$temp_dir" <<'EOF'
codex_command: "codex --yolo exec"
commands:
  next_task: "next-task"
  task_show: "task-show"
  task_status: "task-status"
  task_update_in_progress: "task-update"
review_loop_limit: null
log_path: "./.trudger.log"
hooks:
  on_completed: "hook --done"
  on_requires_human: "hook --needs-human"
EOF
)"

  HOME="$temp_dir" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"review_loop_limit must be a positive integer"* ]]
}

@test "unknown config keys warn" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/unknown-config-keys"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_EXTRA_CONFIG="unknown_key: 'mystery'" config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  [[ "$output" == *"Warning: Unknown config key: unknown_key"* ]]
}

@test "missing yq prints clear error" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-yq"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local minimal_path
  minimal_path="$(make_minimal_path "$temp_dir")"

  HOME="$temp_dir" \
    PATH="$minimal_path" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"Missing dependency: yq"* ]]
}

@test "no tasks exits zero without codex" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-tasks"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "open task is treated as ready" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/open-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local codex_log="${temp_dir}/codex.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-42' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"open","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    '[{"id":"tr-42","status":"closed","labels":[],"payload":"SHOW_PAYLOAD"}]' \
    > "$show_queue"
  printf '%s\n' 'open' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    TASK_UPDATE_OUTPUT="UPDATE_IGNORED" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

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
  config_path="$(copy_sample_config "$temp_dir" "trudgeable-with-hooks")"

  local br_log="${temp_dir}/br.log"
  local codex_log="${temp_dir}/codex.log"
  local ready_queue="${temp_dir}/ready.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' '[{"id":"tr-1"}]' '[]' > "$ready_queue"
  printf '%s\n' \
    '[{"id":"tr-1","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-1","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-1","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-1","status":"closed","labels":["trudgeable"]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    BR_MOCK_READY_QUEUE="$ready_queue" \
    BR_MOCK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-1 trudgeable" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec " "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "resume --last" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "tr-1" "$codex_log"
  [ "$status" -eq 0 ]
}

@test "robot-triage sample config selects task via bv" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/robot-triage"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(copy_sample_config "$temp_dir" "robot-triage")"

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
    run_trudger -c "$config_path"

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
  config_path="$(copy_sample_config "$temp_dir" "robot-triage")"

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
    run_trudger -c "$config_path"

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
  BASE_CODEX_COMMAND="codex --yolo exec --custom" config_path="$(write_base_config "$temp_dir")"

  local codex_log="${temp_dir}/codex.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-10' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"ready","labels":[]}]' \
    '[{"id":"tr-10","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

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
  BASE_HOOK_ON_COMPLETED='hook --done "$1"' config_path="$(write_base_config "$temp_dir")"

  local hook_log="${temp_dir}/hook.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-55' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"ready","labels":[]}]' \
    '[{"id":"tr-55","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    HOOK_MOCK_LOG="$hook_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  run grep -q -- "--done tr-55" "$hook_log"
  [ "$status" -eq 0 ]
}

@test "hooks prepend task id when no substitution is present" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/hook-prepend"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  BASE_HOOK_ON_COMPLETED="hook --done" config_path="$(write_base_config "$temp_dir")"

  local hook_log="${temp_dir}/hook.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-66' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-66","status":"ready","labels":[]}]' \
    '[{"id":"tr-66","status":"ready","labels":[]}]' \
    '[{"id":"tr-66","status":"ready","labels":[]}]' \
    '[{"id":"tr-66","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    HOOK_MOCK_LOG="$hook_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  run grep -q -- "tr-66 --done" "$hook_log"
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
  config_path="$(copy_sample_config "$temp_dir" "trudgeable-with-hooks")"

  local br_log="${temp_dir}/br.log"
  local ready_queue="${temp_dir}/ready.queue"
  local show_queue="${temp_dir}/show.queue"
  printf '%s\n' '[{"id":"tr-2"}]' '[]' > "$ready_queue"
  printf '%s\n' \
    '[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    '[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    '[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    '[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    BR_MOCK_READY_QUEUE="$ready_queue" \
    BR_MOCK_SHOW_QUEUE="$show_queue" \
    BR_MOCK_LOG="$br_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-2 trudgeable" "$br_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label add tr-2 human-required" "$br_log"
  [ "$status" -eq 0 ]
}

@test "missing status after review errors" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-status-after-review"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-88' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-88","status":"ready","labels":[]}]' \
    '[{"id":"tr-88","status":"ready","labels":[]}]' \
    '[{"id":"tr-88","status":"ready","labels":[]}]' \
    '[{"id":"tr-88","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' '' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
  [[ "$output" == *"Task tr-88 missing status after review."* ]]
}

@test "next-task command selects id using first token" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local br_log="${temp_dir}/br.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-20 extra' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"closed","labels":["trudgeable"]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    BR_MOCK_LOG="$br_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
}

@test "next-task command returning empty exits zero" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-empty"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "next-task command exit 1 exits zero" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-exit-1"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_EXIT_CODE=1 \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger -c "$config_path"

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "next-task command non-zero exit errors" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-exit-2"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  HOME="$temp_dir" \
    NEXT_TASK_EXIT_CODE=2 \
    run_trudger -c "$config_path"

  [ "$status" -eq 2 ]
  [[ "$output" == *"next_task command failed with exit code 2"* ]]
}

@test "env config is ignored" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/env-ignored"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  config_path="$(write_base_config "$temp_dir")"

  local br_log="${temp_dir}/br.log"
  local show_queue="${temp_dir}/show.queue"
  local next_task_queue="${temp_dir}/next-task.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"ready","labels":[]}]' \
    '[{"id":"tr-99","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'tr-99' '' > "$next_task_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    TRUDGER_NEXT_CMD='next-task' \
    TRUDGER_REVIEW_LOOPS=0 \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    BR_MOCK_LOG="$br_log" \
    run_trudger -c "$config_path"

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
  config_path="$(write_base_config "$temp_dir")"

  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local status_queue="${temp_dir}/status.queue"
  printf '%s\n' 'tr-4' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"ready","labels":[]}]' \
    '[{"id":"tr-4","status":"closed","labels":[]}]' \
    > "$show_queue"
  printf '%s\n' 'ready' 'closed' > "$status_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    TASK_SHOW_QUEUE="$show_queue" \
    TASK_STATUS_QUEUE="$status_queue" \
    CODEX_MOCK_FAIL_ON=exec \
    run_trudger -c "$config_path"

  [ "$status" -ne 0 ]
}
