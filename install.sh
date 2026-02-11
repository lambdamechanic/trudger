#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
  shift
fi

if [[ "$#" -ne 0 ]]; then
  printf 'Usage: %s [--force]\n\nInstalls prompt files under ~/.codex/prompts.\n' "$0" >&2
  exit 1
fi

prompt_dir="${HOME}/.codex/prompts"

mkdir -p "$prompt_dir"
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

cat <<EOF
Installed prompts to ${prompt_dir}

Install the Rust binary with cargo:
  cargo install --path "${root_dir}" --locked

Ensure "\$HOME/.cargo/bin" is on your PATH, then run:
  trudger --help
EOF
