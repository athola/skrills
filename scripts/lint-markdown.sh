#!/usr/bin/env bash
# Run markdownlint over every Markdown file in the repo.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="npx"
readonly CMD=(npx --yes markdownlint-cli2@0.15.0)
readonly PATTERNS=(
  "**/*.md"
  "!target*/**"
  "!book/book/**"
  "!node_modules/**"
  "!.npm-cache/**"
  "!.npm/**"
  "!.cargo-home/**"
  "!.cargo-tmp/**"
  "!.cargo/**"
  "!.home-tmp/**"
  "!.codex/**"
  "!reviews/**"
  "!.claude/**"
)

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' "Env: NPM_CONFIG_CACHE overrides the repo-local npm cache (.npm-cache)."
  exit 0
}

depcheck() {
  local utility missing=""
  for utility in ${REQUIRED_DEPENDENCIES}; do
    command -v "${utility}" >/dev/null 2>&1 || missing="${missing:+"${missing} "}${utility}"
  done
  case "${missing}" in
    "") return 0 ;;
  esac
  for utility in ${missing}; do
    log 5 "${utility} is required to run markdownlint. Install Node.js/npm first."
  done
  exit 1
}

main() {
  case "${1:-}" in
    *[uU][sS][aA][gG][eE] | *[hH][eE][lL][pP]) usage ;;
  esac
  local opt
  while getopts ":hx" opt; do
    case "${opt}" in
      h) usage ;;
      x) set -x ;;
      *)
        log 3 "Unknown flag: -${OPTARG}"
        exit 64
        ;;
    esac
  done

  depcheck
  # Anchor on the repo root: run from a subdirectory, the `**/*.md` globs
  # would lint only that subtree and report the rest clean.
  (
    cd "${MYDIR%/}/.."
    # Avoid failures when the user's default npm cache (~/.npm) has bad
    # permissions (common when npm was previously run with sudo). Use a
    # repo-local cache instead.
    local cache_dir="${NPM_CONFIG_CACHE:-${PWD}/.npm-cache}"
    mkdir -p "${cache_dir}"
    export NPM_CONFIG_CACHE="${cache_dir}"
    export npm_config_cache="${cache_dir}"

    log 6 "running markdownlint-cli2..."
    "${CMD[@]}" "${PATTERNS[@]}"
  )
}

main "$@"
