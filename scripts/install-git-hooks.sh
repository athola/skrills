#!/usr/bin/env bash
# Point git at the repo's githooks/ directory, so every commit runs
# `make precommit` first.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="git"

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
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
    log 5 "${utility} is required but was not found on PATH."
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
  (
    cd "${MYDIR%/}/.."
    git config core.hooksPath githooks
  )
  log 6 "Configured git core.hooksPath to githooks/"
  log 6 "pre-commit hook will now run 'make precommit' on every commit."
}

main "$@"
