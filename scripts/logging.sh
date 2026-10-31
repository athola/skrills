#!/bin/sh
# Logging for the repo's shell scripts. A library: source it, never run it.
#
# log [level] <message...>
#   level is a syslog severity, 0 (emergency) to 7 (debug): 3 error,
#   4 warning, 5 notice, 6 info, 7 debug. A first argument that is not a
#   level makes the whole argument list the message, logged at 6. Lines above
#   LOG_LEVEL (default 6) are dropped. Output goes to stderr, so stdout stays
#   free for the results other tools parse.
#
# banner <message...>
#   Frames the message between two separator lines on stdout, for the
#   summary block a run ends with.
#
# POSIX sh: bash scripts and the sh installer tooling both source it.

__logging_loaded=1
__banner_sep="========================================"

log() {
  case "${1:-}" in
    [0-7])
      __log_level="${1}"
      shift
      ;;
    *) __log_level=6 ;;
  esac
  case "${LOG_LEVEL:-6}" in
    [0-7]) __log_threshold="${LOG_LEVEL:-6}" ;;
    *) __log_threshold=6 ;;
  esac
  # An `if`, not `[ ... ] && printf`: a dropped line must still return 0, or
  # the caller dies under `set -e`.
  if [ "${__log_level}" -le "${__log_threshold}" ]; then
    printf '%s\n' "${*}" >&2
  fi
  unset __log_level __log_threshold
  return 0
}

banner() {
  printf '%s\n%s\n%s\n' "${__banner_sep}" "${*}" "${__banner_sep}"
}
