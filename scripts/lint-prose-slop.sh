#!/usr/bin/env bash
# Block AI-slop vocabulary in user-facing prose.
#
# Scans non-archive Markdown documentation for banned words tracked
# in CLAUDE.md and the AI hygiene report (.ai-hygiene-report.md).
# Fails with non-zero exit when any banned word is found, so it can
# gate pre-commit and CI.
#
# Usage: ./scripts/lint-prose-slop.sh
set -euo pipefail

BANNED='leverage|seamless|cutting-edge|delve into'

# Files that may legitimately contain these words (the linter itself,
# the hygiene report that catalogs them, and historical archives).
EXCLUDE_PATHS=(
  "docs/archive/*"
  "docs/CHANGELOG.md"
  ".ai-hygiene-report.md"
  ".unbloat-report.md"
  ".refinement-evidence.md"
  "scripts/lint-prose-slop.sh"
  ".claude/*"
  "node_modules/*"
  "target/*"
  ".cargo*"
)

EXCLUDES=()
for p in "${EXCLUDE_PATHS[@]}"; do
  EXCLUDES+=(--exclude-dir="${p%%/*}" --exclude="$p")
done

if ! command -v rg >/dev/null 2>&1; then
  echo "ripgrep (rg) is required for the prose-slop lint." >&2
  exit 2
fi

# Build globs for ripgrep.
RG_ARGS=(
  --color=never
  --no-heading
  --line-number
  --type=md
  --glob='!docs/archive/**'
  --glob='!docs/CHANGELOG.md'
  --glob='!.ai-hygiene-report.md'
  --glob='!.unbloat-report.md'
  --glob='!.refinement-evidence.md'
  --glob='!scripts/lint-prose-slop.sh'
  --glob='!.claude/**'
  --glob='!node_modules/**'
  --glob='!target/**'
  -e "\\b(${BANNED})\\b"
)

if rg "${RG_ARGS[@]}" .; then
  echo "" >&2
  echo "ERROR: AI-slop vocabulary detected in non-archive prose." >&2
  echo "Banned words: ${BANNED//|/, }" >&2
  echo "Replace with concrete language. Allowed in docs/archive/, CHANGELOG.md." >&2
  exit 1
fi

echo "prose-slop lint clean (no banned vocabulary in user-facing docs)"
