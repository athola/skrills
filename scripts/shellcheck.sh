#!/bin/sh
# Lint the repo's shell scripts: shellcheck at warning level, then shfmt in
# diff mode. Each file is read in the dialect its shebang names. Bash scripts
# are formatted without -p because the POSIX parser rejects the arrays, [[ and
# pipefail they rely on.
set -eu

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="shellcheck shfmt"
readonly DEFAULT_TARGETS="scripts/*.sh githooks/pre-commit"

usage() {
  log "Usage: ${0##*/} [-h] [-x] [script ...]"
  printf '  %-8s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)" \
    "script" "Lint only these files (default: ${DEFAULT_TARGETS})"
  exit 0
}

depcheck() {
  missing=""
  for utility in ${REQUIRED_DEPENDENCIES}; do
    command -v "${utility}" >/dev/null 2>&1 || missing="${missing:+"${missing} "}${utility}"
  done
  case "${missing}" in
    "") return 0 ;;
  esac
  for utility in ${missing}; do
    log 5 "${utility} not installed (brew/apt install ${utility})"
  done
  exit 1
}

# Print "bash" or "posix" for the dialect file $1 declares in its shebang.
dialect() {
  IFS= read -r shebang <"${1:?dialect needs a file}" || shebang=""
  case "${shebang}" in
    *bash*) printf 'bash\n' ;;
    *) printf 'posix\n' ;;
  esac
}

# Lint one file; return 1 when shellcheck or shfmt reports anything.
lint_file() {
  file="${1:?lint_file needs a file}"
  file_rc=0
  shellcheck -x -S warning "${file}" || file_rc=1
  case "$(dialect "${file}")" in
    bash) shfmt -d -i 2 -ci -ln bash "${file}" || file_rc=1 ;;
    *) shfmt -d -p -i 2 -ci "${file}" || file_rc=1 ;;
  esac
  return "${file_rc}"
}

main() {
  case "${1:-}" in
    *[uU][sS][aA][gG][eE] | *[hH][eE][lL][pP]) usage ;;
  esac
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
  shift $((OPTIND - 1))

  depcheck

  # Explicit files are taken relative to the caller's directory; the default
  # set is relative to the repo root, where the `source=` directives resolve.
  targets=""
  for target; do
    case "${target}" in
      /*) ;;
      *) target="$(pwd)/${target}" ;;
    esac
    targets="${targets:+"${targets} "}${target}"
  done

  (
    cd "${MYDIR%/}/.."
    scanned=0
    failed=0
    # shellcheck disable=SC2086 # word splitting expands the target globs
    for file in ${targets:-${DEFAULT_TARGETS}}; do
      scanned=$((scanned + 1))
      lint_file "${file}" || failed=$((failed + 1))
    done
    case "${failed}" in
      0) log 6 "shell lint clean (${scanned} scripts)" ;;
      *)
        log 3 "shell lint failed: ${failed} of ${scanned} scripts"
        exit 1
        ;;
    esac
  )
}

main "$@"
