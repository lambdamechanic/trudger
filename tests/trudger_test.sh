#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PATH="${ROOT_DIR}/tests/fixtures/bin:${PATH}"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  if ! printf '%s' "$haystack" | grep -q -- "$needle"; then
    fail "expected to find '$needle'"
  fi
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  if printf '%s' "$haystack" | grep -q -- "$needle"; then
    fail "expected to not find '$needle'"
  fi
}

run_trudger() {
  local output
  set +e
  output=$("${ROOT_DIR}/trudger" 2>&1)
  local status=$?
  set -e
  printf '%s\n' "$output"
  return $status
}

run_test_missing_prompts() {
  local temp_dir
  temp_dir=$(mktemp -d)
  HOME="$temp_dir" \
  BD_MOCK_READY_JSON='[]' \
  run_trudger >"${temp_dir}/out" || true
  local output
  output=$(cat "${temp_dir}/out")
  assert_contains "$output" "Missing prompt file"
}

run_test_no_tasks_exits_zero() {
  local temp_dir
  temp_dir=$(mktemp -d)
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge" "${temp_dir}/.codex/prompts/trudge_review"
  local bd_log="${temp_dir}/bd.log"
  local codex_log="${temp_dir}/codex.log"

  HOME="$temp_dir" \
  BD_MOCK_READY_JSON='[]' \
  BD_MOCK_LOG="$bd_log" \
  CODEX_MOCK_LOG="$codex_log" \
  run_trudger >/dev/null

  if [[ -s "$codex_log" ]]; then
    fail "codex should not be invoked when no tasks are ready"
  fi
}

run_test_closed_task_removes_label() {
  local temp_dir
  temp_dir=$(mktemp -d)
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge" "${temp_dir}/.codex/prompts/trudge_review"
  local bd_log="${temp_dir}/bd.log"
  local codex_log="${temp_dir}/codex.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-1"}]' '[]' > "${ready_queue}"

  HOME="$temp_dir" \
  BD_MOCK_READY_QUEUE="$ready_queue" \
  BD_MOCK_SHOW_JSON='[{"id":"tr-1","status":"closed","labels":["trudgeable"]}]' \
  BD_MOCK_LOG="$bd_log" \
  CODEX_MOCK_LOG="$codex_log" \
  run_trudger >/dev/null

  local bd_calls
  bd_calls=$(cat "$bd_log")
  assert_contains "$bd_calls" "label remove tr-1 trudgeable"

  local codex_calls
  codex_calls=$(cat "$codex_log")
  assert_contains "$codex_calls" "codex exec /prompt:trudge tr-1"
  assert_contains "$codex_calls" "codex exec resume --last /prompt:trudge_review tr-1"
}

run_test_requires_human_updates() {
  local temp_dir
  temp_dir=$(mktemp -d)
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge" "${temp_dir}/.codex/prompts/trudge_review"
  local bd_log="${temp_dir}/bd.log"
  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-2"}]' '[]' > "${ready_queue}"

  HOME="$temp_dir" \
  BD_MOCK_READY_QUEUE="$ready_queue" \
  BD_MOCK_SHOW_JSON='[{"id":"tr-2","status":"open","labels":["trudgeable","requires-human"]}]' \
  BD_MOCK_LOG="$bd_log" \
  TRUDGER_REQUIRES_HUMAN_COMMENT="Needs human input" \
  TRUDGER_REQUIRES_HUMAN_NOTES="Awaiting guidance" \
  run_trudger >/dev/null

  local bd_calls
  bd_calls=$(cat "$bd_log")
  assert_contains "$bd_calls" "comments add tr-2"
  assert_contains "$bd_calls" "update tr-2 --notes"
  assert_contains "$bd_calls" "label remove tr-2 trudgeable"
  assert_contains "$bd_calls" "label add tr-2 requires-human"
}

run_test_errors_when_no_close_or_requires_human() {
  local temp_dir
  temp_dir=$(mktemp -d)
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge" "${temp_dir}/.codex/prompts/trudge_review"

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-3"}]' '[]' > "${ready_queue}"

  if HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    BD_MOCK_SHOW_JSON='[{"id":"tr-3","status":"open","labels":["trudgeable"]}]' \
    run_trudger >/dev/null; then
    fail "expected non-zero exit"
  fi
}

run_test_codex_exec_failure_aborts() {
  local temp_dir
  temp_dir=$(mktemp -d)
  mkdir -p "${temp_dir}/.codex/prompts"
  touch "${temp_dir}/.codex/prompts/trudge" "${temp_dir}/.codex/prompts/trudge_review"

  local ready_queue="${temp_dir}/ready.queue"
  printf '%s\n' '[{"id":"tr-4"}]' '[]' > "${ready_queue}"

  if HOME="$temp_dir" \
    BD_MOCK_READY_QUEUE="$ready_queue" \
    CODEX_MOCK_FAIL_ON=exec \
    run_trudger >/dev/null; then
    fail "expected non-zero exit"
  fi
}

run_test_missing_prompts
run_test_no_tasks_exits_zero
run_test_closed_task_removes_label
run_test_requires_human_updates
run_test_errors_when_no_close_or_requires_human
run_test_codex_exec_failure_aborts

printf 'All tests passed.\n'
