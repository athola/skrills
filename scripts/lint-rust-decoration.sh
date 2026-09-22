#!/usr/bin/env bash
# Block decorative separator-comment blocks in Rust source.
#
# Pattern: `// ─{20,}` (20+ box-drawing characters in a comment).
# See M1 in the AI hygiene report; these are an AI generation
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

# See lint-prose-slop.sh: an `if rg ...` treats ripgrep's exit 2 (error) as
# "no match", so the gate reported clean instead of failing.
rg_rc=0
rg "${RG_ARGS[@]}" . || rg_rc=$?
case "${rg_rc}" in
  0)
    echo "" >&2
    echo "ERROR: decorative separator comments (// ─...) detected." >&2
    echo "Replace with a blank line + Rustdoc section comment, or remove." >&2
    exit 1
    ;;
  1) ;;
  *)
    echo "ERROR: ripgrep failed (exit ${rg_rc}); decoration lint did not run." >&2
    exit 2
    ;;
esac

echo "rust-decoration lint clean (no // ─{20,} separator blocks)"
