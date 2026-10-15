#!/usr/bin/env bash
# Skrills plugin audit shim.
#
# Strategy:
#   1. If NIGHT_MARKET_ROOT (or a discovered ~/claude-night-market checkout)
#      contains the upstream sanctum/abstract scripts, use those; they are
#      the source of truth and may include Phase 2-4 features.
#   2. Otherwise fall back to the in-tree ports under scripts/.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

REPO_ROOT="$(cd "${MYDIR%/}/.." && pwd)"
readonly REPO_ROOT
readonly PLUGINS_ROOT="${REPO_ROOT}/plugins"
readonly REQUIRED_DEPENDENCIES="python3"

readonly LOCAL_REGISTRATION="${REPO_ROOT}/scripts/update_plugin_registrations.py"
readonly LOCAL_VALIDATE="${REPO_ROOT}/scripts/validate_plugin.py"
readonly LOCAL_MODERNIZE="${REPO_ROOT}/scripts/check_hook_modernization.py"

usage() {
  log "Usage: ${0##*/} [--fix] [--validate] [--modernize] [--all] [-x] [<plugin-name>]"
  printf '  %-16s %s\n' \
    "--fix" "Pass --fix through to the registration auditor." \
    "--validate" "Run the structural validator (validate_plugin.py)." \
    "--modernize" "Run the hook modernization auditor." \
    "--all" "Run registration audit + validate + modernize." \
    "--skip-research" "Reserved for parity with sanctum CLI; ignored locally." \
    "-x" "Trace execution (set -x)." \
    "-h, --help" "Show this help and exit."
  printf '\n  %s\n' "Environment:"
  printf '  %-16s %s\n' "NIGHT_MARKET_ROOT" "Override the upstream checkout path."
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

# Print NIGHT_MARKET_ROOT, or the first upstream checkout found under HOME.
resolve_night_market_root() {
  local candidate
  case "${NIGHT_MARKET_ROOT:-}" in
    "") ;;
    *)
      printf '%s\n' "${NIGHT_MARKET_ROOT}"
      return
      ;;
  esac
  for candidate in "${HOME}/claude-night-market" "${HOME}/work/claude-night-market"; do
    if [ -d "${candidate}" ]; then
      printf '%s\n' "${candidate}"
      return
    fi
  done
}

pick_script() {
  local upstream="${1}"
  local local_fallback="${2}"
  case "${night_market_root}" in
    "") ;;
    *)
      if [ -f "${upstream}" ]; then
        printf '%s\n' "${upstream}"
        return
      fi
      ;;
  esac
  printf '%s\n' "${local_fallback}"
}

# Name every pick: an upstream script is external code, and with --fix the
# registration auditor rewrites files in this repo. stdout, not log: the
# announcement is part of the audit report callers read.
announce_pick() {
  local label="${1}" picked="${2}" upstream="${3}"
  case "${picked}" in
    "${upstream}") printf '%s\n' "[shim] using upstream ${label} script: ${picked}" ;;
    *) printf '%s\n' "[shim] using ported ${label} script: ${picked}" ;;
  esac
}

main() {
  local do_registration=1 do_validate=0 do_modernize=0 fix_flag="" plugin_name=""
  local rc=0 night_market_root plugin_dir
  local registration_script validate_script modernize_script
  local upstream_registration upstream_validate upstream_modernize

  case "${1:-}" in
    *[uU][sS][aA][gG][eE] | *[hH][eE][lL][pP]) usage ;;
  esac
  while [ "${#}" -gt 0 ]; do
    case "${1}" in
      --fix) fix_flag="--fix" ;;
      --validate) do_validate=1 ;;
      --modernize) do_modernize=1 ;;
      --all)
        do_validate=1
        do_modernize=1
        ;;
      --skip-research) ;; # parity flag; no-op here
      -h | --help) usage ;;
      -x) set -x ;;
      -*)
        log 3 "Unknown flag: ${1}"
        exit 64
        ;;
      *)
        plugin_name="${1}"
        ;;
    esac
    shift
  done

  depcheck

  night_market_root="$(resolve_night_market_root)"
  upstream_registration="${night_market_root}/plugins/sanctum/scripts/update_plugin_registrations.py"
  upstream_validate="${night_market_root}/plugins/abstract/scripts/validate_plugin.py"
  upstream_modernize="${night_market_root}/scripts/check_hook_modernization.py"

  registration_script="$(pick_script "${upstream_registration}" "${LOCAL_REGISTRATION}")"
  validate_script="$(pick_script "${upstream_validate}" "${LOCAL_VALIDATE}")"
  modernize_script="$(pick_script "${upstream_modernize}" "${LOCAL_MODERNIZE}")"

  announce_pick registration "${registration_script}" "${upstream_registration}"
  case "${do_validate}" in
    1) announce_pick validate "${validate_script}" "${upstream_validate}" ;;
  esac
  case "${do_modernize}" in
    1) announce_pick modernize "${modernize_script}" "${upstream_modernize}" ;;
  esac

  # 1) Registration audit.
  case "${do_registration}" in
    1)
      log 6 ""
      log 6 "=== Phase 1: registration audit ==="
      (
        cd "${REPO_ROOT}"
        case "${plugin_name}" in
          "")
            python3 "${registration_script}" \
              --plugins-root "${PLUGINS_ROOT}" ${fix_flag:+${fix_flag}}
            ;;
          *)
            python3 "${registration_script}" "${plugin_name}" \
              --plugins-root "${PLUGINS_ROOT}" ${fix_flag:+${fix_flag}}
            ;;
        esac
      ) || rc=${?}
      ;;
  esac

  # 2) Structural validation.
  case "${do_validate}" in
    1)
      log 6 ""
      log 6 "=== Phase 1b: structural validation ==="
      case "${plugin_name}" in
        "")
          for plugin_dir in "${PLUGINS_ROOT}"/*/; do
            [ -d "${plugin_dir}" ] || continue
            python3 "${validate_script}" "${plugin_dir%/}" || rc=${?}
          done
          ;;
        *) python3 "${validate_script}" "${PLUGINS_ROOT}/${plugin_name}" || rc=${?} ;;
      esac
      ;;
  esac

  # 3) Hook modernization.
  case "${do_modernize}" in
    1)
      log 6 ""
      log 6 "=== Phase 1c: hook modernization audit ==="
      python3 "${modernize_script}" --root "${REPO_ROOT}" || rc=${?}
      ;;
  esac

  exit "${rc}"
}

main "$@"
