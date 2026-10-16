#!/usr/bin/env bash
# Block AI-slop vocabulary in user-facing prose.
#
# Scans non-archive Markdown documentation under the current directory for
# banned words tracked in CLAUDE.md and the AI hygiene report
# (.ai-hygiene-report.md). Fails with non-zero exit when any banned word is
# found, so it can gate pre-commit and CI.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="rg"
# Stems catch inflections (leveraging, seamlessly); matched case-insensitively.
readonly BANNED='leverag[a-z]*|seamless[a-z]*|cutting-edge|delve into'
readonly RG_ARGS=(
  --color=never
  --no-heading
  --line-number
  --ignore-case
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

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' "Exit: 0 clean, 1 banned word found, 2 ripgrep missing or failed."
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
    log 5 "ripgrep (${utility}) is required for the prose-slop lint."
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
  # ripgrep exits 0 on a match, 1 on no match and 2 on error. Testing it
  # directly in an `if` made an error read as "no match", so a bad glob or an
  # unreadable file reported the gate clean and disabled it silently.
  local rg_rc=0
  rg "${RG_ARGS[@]}" . || rg_rc=${?}
  case "${rg_rc}" in
    0)
      log 3 ""
      log 3 "ERROR: AI-slop vocabulary detected in non-archive prose."
      log 3 "Banned words: ${BANNED//|/, }"
      log 3 "Replace with concrete language. Allowed in docs/archive/, CHANGELOG.md."
      exit 1
      ;;
    1) ;;
    *)
      log 3 "ERROR: ripgrep failed (exit ${rg_rc}); prose lint did not run."
      exit 2
      ;;
  esac

  # stdout, not log: callers and tests read this line as the clean verdict.
  printf '%s\n' "prose-slop lint clean (no banned vocabulary in user-facing docs)"
}

main "$@"
