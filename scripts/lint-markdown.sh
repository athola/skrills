#!/usr/bin/env bash
set -euo pipefail

if ! command -v npx >/dev/null 2>&1; then
  echo "npx is required to run markdownlint. Install Node.js/npm first." >&2
  exit 1
fi

# Avoid failures when the user's default npm cache (~/.npm) has bad permissions
# (common when npm was previously run with sudo). Use a repo-local cache instead.
CACHE_DIR="${NPM_CONFIG_CACHE:-$PWD/.npm-cache}"
mkdir -p "$CACHE_DIR"
export NPM_CONFIG_CACHE="$CACHE_DIR"
export npm_config_cache="$CACHE_DIR"

CMD=(npx --yes markdownlint-cli2@0.15.0)
PATTERNS=(
  "**/*.md"
  "!target/**"
  "!book/book/**"
  "!node_modules/**"
  "!.npm-cache/**"
  "!.npm/**"
  "!.cargo-home/**"
  "!.cargo-tmp/**"
  "!.cargo/**"
  "!.home-tmp/**"
  "!.codex/**"
)

echo "running markdownlint-cli2..."
"${CMD[@]}" "${PATTERNS[@]}"
