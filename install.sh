#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

bin_dir="${HOME}/.local/bin"
prompt_dir="${HOME}/.codex/prompts"

mkdir -p "$bin_dir" "$prompt_dir"

install -m 0755 "${root_dir}/trudger" "${bin_dir}/trudger"
install -m 0644 "${root_dir}/prompts/trudge.md" "${prompt_dir}/trudge.md"
install -m 0644 "${root_dir}/prompts/trudge_review.md" "${prompt_dir}/trudge_review.md"

printf 'Installed trudger to %s and prompts to %s\n' "$bin_dir" "$prompt_dir"
