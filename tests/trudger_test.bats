#!/usr/bin/env bats

setup() {
  ROOT_DIR="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"
  PATH="${ROOT_DIR}/tests/fixtures/bin:${PATH}"
}

run_trudger() {
  run "${ROOT_DIR}/trudger"
}

should_run_codex_tests() {
  [[ "${TRUDGER_TEST_RUN_CODEX:-0}" == "1" ]]
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

@test "no tasks exits zero without codex" {
  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-tasks"
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge.md" "${temp_dir}/.codex/prompts/trudge_review.md"

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
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge.md" "${temp_dir}/.codex/prompts/trudge_review.md"

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
  run grep -q -- "label remove tr-1 trudgeable" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex exec /prompt:trudge tr-1" "$codex_log"
  [ "$status" -eq 0 ]
  run grep -q -- "codex exec resume --last /prompt:trudge_review tr-1" "$codex_log"
  [ "$status" -eq 0 ]
}

@test "requires-human updates labels" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/requires-human"
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge.md" "${temp_dir}/.codex/prompts/trudge_review.md"

  local bd_log="${temp_dir}/bd.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-2"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
    BD_MOCK_LOG="$bd_log" \
    run_trudger

  [ "$status" -eq 0 ]
  run grep -q -- "label remove tr-2 trudgeable" "$bd_log"
  [ "$status" -eq 0 ]
  run grep -q -- "label add tr-2 requires-human" "$bd_log"
  [ "$status" -eq 0 ]
}

@test "errors when task not closed or requires-human" {
  if ! should_run_codex_tests; then
    skip "set TRUDGER_TEST_RUN_CODEX=1 to enable"
  fi

  local temp_dir
  temp_dir="${BATS_TEST_TMPDIR}/no-close"
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge.md" "${temp_dir}/.codex/prompts/trudge_review.md"

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
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge.md" "${temp_dir}/.codex/prompts/trudge_review.md"

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-4"}]' '[]' > "$ready_queue"

  HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    CODEX_MOCK_FAIL_ON=exec \
    run_trudger

  [ "$status" -ne 0 ]
}
