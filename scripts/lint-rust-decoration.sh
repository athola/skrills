#!/usr/bin/env bash
# Block decorative separator-comment blocks in Rust source.
#
# Pattern: `// ─{20,}` (20+ box-drawing characters in a comment).
# See M1 in the AI hygiene report; these are an AI generation
# signature that adds no semantic content. Fail the build to keep
# them from creeping back in.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="rg"
readonly RG_ARGS=(
  --color=never
  --no-heading
  --line-number
  --type=rust
  --glob='!target/**'
  --glob='!.cargo*/**'
  -e '//\s*─{20,}'
)

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' "Exit: 0 clean, 1 separator block found, 2 ripgrep missing or failed."
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
    log 5 "ripgrep (${utility}) is required for the decoration lint."
  done
  exit 2
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
  # See lint-prose-slop.sh: an `if rg ...` treats ripgrep's exit 2 (error) as
  # "no match", so the gate reported clean instead of failing.
  local rg_rc=0
  rg "${RG_ARGS[@]}" . || rg_rc=${?}
  case "${rg_rc}" in
    0)
      log 3 ""
      log 3 "ERROR: decorative separator comments (// ─...) detected."
      log 3 "Replace with a blank line + Rustdoc section comment, or remove."
      exit 1
      ;;
    1) ;;
    *)
      log 3 "ERROR: ripgrep failed (exit ${rg_rc}); decoration lint did not run."
      exit 2
      ;;
  esac

  # stdout, not log: callers and tests read this line as the clean verdict.
  printf '%s\n' "rust-decoration lint clean (no // ─{20,} separator blocks)"
}

main "$@"
