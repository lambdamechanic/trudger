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

@test "missing prompt files cause a clear error" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-prompts"
  mkdir -p "$temp_dir"

  HOME="$temp_dir" \
    BD_MOCK_READY_JSON='[]' \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"Missing prompt file"* ]]
}

@test "missing config warns and uses defaults" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/missing-config"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"

  HOME="$temp_dir" \
    BD_MOCK_READY_JSON='[]' \
    run_trudger

  [ "$status" -eq 0 ]
  [[ "$output" == *"Warning: missing config file"* ]]
}

@test "no tasks exits zero without codex" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-tasks"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    BD_MOCK_READY_JSON='[]' \
    BD_MOCK_LOG="$bd_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "closed task removes trudgeable label" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/closed-task"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local codex_log="${temp_dir}/codex.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-1"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-1","status":"closed","labels":["trudgeable"]}]' \
    BD_MOCK_LOG="$bd_log" \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "update tr-1 --status in_progress" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-1 trudgeable" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec " "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec resume --last " "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "tr-1" "$codex_log"
  [ "$status" -eq 0 ]
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
review_loop_limit: 5
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local codex_log="${temp_dir}/codex.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-10"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-10","status":"closed","labels":["trudgeable"]}]' \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --custom" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex --yolo exec --custom resume --last" "$codex_log"
  [ "$status" -eq 0 ]
}

@test "completion hook runs and skips label removal" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/completion-hook"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
hooks:
  on_completed: "hook --done extra"
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local hook_log="${temp_dir}/hook.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-11"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-11","status":"closed","labels":["trudgeable"]}]' \
    BD_MOCK_LOG="$bd_log" \
    HOOK_MOCK_LOG="$hook_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "tr-11 --done extra" "$hook_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-11" "$bd_log"
  [ "$status" -ne 0 ]
}

@test "requires-human updates labels" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/requires-human"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-2"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "update tr-2 --status in_progress" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-2 trudgeable" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label add tr-2 requires-human" "$bd_log"
  [ "$status" -eq 0 ]
}

@test "requires-human hook runs and skips label changes" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/requires-human-hook"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
hooks:
  on_requires_human: "hook --needs-human"
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local hook_log="${temp_dir}/hook.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-12"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-12","status":"open","labels":["trudgeable","requires-human"]}]' \
    BD_MOCK_LOG="$bd_log" \
    HOOK_MOCK_LOG="$hook_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "tr-12 --needs-human" "$hook_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-12" "$bd_log"
  [ "$status" -ne 0 ]
}

@test "requires-human label optional when hooks absent" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/requires-human-no-label"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
codex_command: "codex --yolo exec"
review_loop_limit: 5
labels:
  trudgeable: ""
  requires_human: ""
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-13"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-13","status":"open","labels":[]}]' \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "label add tr-13" "$bd_log"
  [ "$status" -ne 0 ]
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
review_loop_limit: 5
next_task_command: "next-task"
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"
  local next_task_queue="${temp_dir}/next-task.queue"
  local show_queue="${temp_dir}/show.queue"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[]' > "$ready_queue"
  printf '%s\n' 'tr-20 extra' '' > "$next_task_queue"
  printf '%s\n' \
    '[{"id":"tr-20","status":"ready","labels":["trudgeable"]}]' \
    '[{"id":"tr-20","status":"closed","labels":["trudgeable"]}]' \
    > "$show_queue"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT_QUEUE="$next_task_queue" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_QUEUE="$show_queue" \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "bd ready" "$bd_log"
  [ "$status" -ne 0 ]
}

@test "next-task command returning empty exits zero" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/next-task-empty"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
next_task_command: "next-task"
CONFIG

  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
    NEXT_TASK_OUTPUT='' \
    CODEX_MOCK_LOG="$codex_log" \
    run_trudger

  [ "$status" -eq 0 ]
  [ ! -s "$codex_log" ]
}

@test "default next-task uses configured label filter" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/default-label"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
labels:
  trudgeable: custom-label
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"

  HOME="$temp_dir" \
    BD_MOCK_READY_JSON='[]' \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "ready --json --label custom-label --sort priority --limit 1" "$bd_log"
  [ "$status" -eq 0 ]
}

@test "env config is ignored" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/env-ignored"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local bd_log="${temp_dir}/bd.log"

  HOME="$temp_dir" \
    TRUDGER_NEXT_CMD='next-task' \
    TRUDGER_REVIEW_LOOPS=0 \
    NEXT_TASK_OUTPUT='tr-99' \
    BD_MOCK_READY_JSON='[]' \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "bd ready" "$bd_log"
  [ "$status" -eq 0 ]
}

@test "review loop limit uses config value" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/review-limit"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
review_loop_limit: 2
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-5"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    TRUDGER_REVIEW_LOOPS=1 \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-5","status":"open","labels":["trudgeable"]}]' \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"after 2 review loops"* ]]
}

@test "errors when task not closed or requires-human" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-close"
  mkdir -p "$temp_dir"
  create_prompts "$temp_dir"
  write_config "$temp_dir" <<'CONFIG'
review_loop_limit: 1
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-3"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-3","status":"open","labels":["trudgeable"]}]' \
    run_trudger

  [ "$status" -ne 0 ]
  [[ "$output" == *"not closed and not requires-human"* ]]
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
review_loop_limit: 5
labels:
  trudgeable: trudgeable
  requires_human: requires-human
CONFIG

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-4"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    CODEX_MOCK_FAIL_ON=exec \
    run_trudger

  [ "$status" -ne 0 ]
}
