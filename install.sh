#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
  shift
fi

if [[ "$#" -ne 0 ]]; then
  printf 'Usage: %s [--force]\n' "$0" >&2
  exit 1
fi

bin_dir="${HOME}/.local/bin"
prompt_dir="${HOME}/.codex/prompts"

mkdir -p "$bin_dir" "$prompt_dir"

install -m 0755 "${root_dir}/trudger" "${bin_dir}/trudger"
install_prompt() {
  local src="$1"
  local dst="$2"

  if [[ -e "$dst" ]]; then
    if cmp -s "$src" "$dst"; then
      printf 'Prompt %s already up to date\n' "$dst"
      return 0
    fi

    printf 'Prompt %s differs from repo version:\n' "$dst"
    diff -u "$dst" "$src" || true
    if [[ "$force" -eq 1 ]]; then
      install -m 0644 "$src" "$dst"
      printf 'Updated prompt %s\n' "$dst"
    else
      printf 'Skipped prompt %s (use --force to overwrite)\n' "$dst"
    fi
    return 0
  fi

  install -m 0644 "$src" "$dst"
  printf 'Installed prompt %s\n' "$dst"
}

install_prompt "${root_dir}/prompts/trudge.md" "${prompt_dir}/trudge.md"
install_prompt "${root_dir}/prompts/trudge_review.md" "${prompt_dir}/trudge_review.md"

printf 'Installed trudger to %s and prompts to %s\n' "$bin_dir" "$prompt_dir"
