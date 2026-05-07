#!/usr/bin/env bash
# Block decorative separator-comment blocks in Rust source.
#
# Pattern: `// ─{20,}` (20+ box-drawing characters in a comment).
# See M1 in the AI hygiene report — these are an AI generation
# signature that adds no semantic content. Fail the build to keep
# them from creeping back in.
#
# Usage: ./scripts/lint-rust-decoration.sh
set -euo pipefail

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep (rg) is required for the decoration lint." >&2
  exit 2
fi

RG_ARGS=(
  --color=never
  --no-heading
  --line-number
  --type=rust
  --glob='!target/**'
  --glob='!.cargo*/**'
  -e '//\s*─{20,}'
)

if rg "${RG_ARGS[@]}" .; then
  echo "" >&2
  echo "ERROR: decorative separator comments (// ─...) detected." >&2
  echo "Replace with a blank line + Rustdoc section comment, or remove." >&2
  exit 1
fi

echo "rust-decoration lint clean (no // ─{20,} separator blocks)"
