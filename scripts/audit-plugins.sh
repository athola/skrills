#!/usr/bin/env bash
# Skrills plugin audit shim.
#
# Strategy:
#   1. If NIGHT_MARKET_ROOT (or a discovered ~/claude-night-market checkout)
#      contains the upstream sanctum/abstract scripts, use those — they are
#      the source of truth and may include Phase 2-4 features.
#   2. Otherwise fall back to the in-tree ports under scripts/.
#
# Usage:
#   scripts/audit-plugins.sh [--fix] [--validate] [--modernize] [--all]
#                            [<plugin-name>]
#
# Flags:
#   --fix         Pass --fix through to the registration auditor.
#   --validate    Run the structural validator (validate_plugin.py).
#   --modernize   Run the hook modernization auditor.
#   --all         Run registration audit + validate + modernize.
#   --skip-research  Reserved for parity with sanctum CLI; ignored locally.
#
# Environment:
#   NIGHT_MARKET_ROOT   Override the upstream checkout path.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGINS_ROOT="${REPO_ROOT}/plugins"

# Default flags.
DO_REGISTRATION=1
DO_VALIDATE=0
DO_MODERNIZE=0
FIX_FLAG=""
PLUGIN_NAME=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fix)        FIX_FLAG="--fix" ;;
    --validate)   DO_VALIDATE=1 ;;
    --modernize)  DO_MODERNIZE=1 ;;
    --all)        DO_VALIDATE=1; DO_MODERNIZE=1 ;;
    --skip-research) ;;  # parity flag; no-op here
    -h|--help)
      sed -n '2,22p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "Unknown flag: $1" >&2
      exit 64
      ;;
    *)
      PLUGIN_NAME="$1"
      ;;
  esac
  shift
done

# Resolve upstream checkout if available.
NIGHT_MARKET_ROOT="${NIGHT_MARKET_ROOT:-}"
if [[ -z "${NIGHT_MARKET_ROOT}" ]]; then
  for candidate in "${HOME}/claude-night-market" "${HOME}/work/claude-night-market"; do
    if [[ -d "${candidate}" ]]; then
      NIGHT_MARKET_ROOT="${candidate}"
      break
    fi
  done
fi

UPSTREAM_REGISTRATION="${NIGHT_MARKET_ROOT}/plugins/sanctum/scripts/update_plugin_registrations.py"
UPSTREAM_VALIDATE="${NIGHT_MARKET_ROOT}/plugins/abstract/scripts/validate_plugin.py"
UPSTREAM_MODERNIZE="${NIGHT_MARKET_ROOT}/scripts/check_hook_modernization.py"

LOCAL_REGISTRATION="${REPO_ROOT}/scripts/update_plugin_registrations.py"
LOCAL_VALIDATE="${REPO_ROOT}/scripts/validate_plugin.py"
LOCAL_MODERNIZE="${REPO_ROOT}/scripts/check_hook_modernization.py"

pick_script() {
  local upstream="$1"
  local local_fallback="$2"
  if [[ -n "${NIGHT_MARKET_ROOT}" && -f "${upstream}" ]]; then
    echo "${upstream}"
  else
    echo "${local_fallback}"
  fi
}

REGISTRATION_SCRIPT="$(pick_script "${UPSTREAM_REGISTRATION}" "${LOCAL_REGISTRATION}")"
VALIDATE_SCRIPT="$(pick_script "${UPSTREAM_VALIDATE}" "${LOCAL_VALIDATE}")"
MODERNIZE_SCRIPT="$(pick_script "${UPSTREAM_MODERNIZE}" "${LOCAL_MODERNIZE}")"

if [[ "${REGISTRATION_SCRIPT}" == "${UPSTREAM_REGISTRATION}" ]]; then
  echo "[shim] using upstream registration script: ${REGISTRATION_SCRIPT}"
else
  echo "[shim] using ported registration script: ${REGISTRATION_SCRIPT}"
fi

# 1) Registration audit.
if (( DO_REGISTRATION )); then
  echo
  echo "=== Phase 1: registration audit ==="
  pushd "${REPO_ROOT}" >/dev/null
  if [[ -n "${PLUGIN_NAME}" ]]; then
    python3 "${REGISTRATION_SCRIPT}" "${PLUGIN_NAME}" \
      --plugins-root "${PLUGINS_ROOT}" ${FIX_FLAG:+${FIX_FLAG}} || rc=$?
  else
    python3 "${REGISTRATION_SCRIPT}" \
      --plugins-root "${PLUGINS_ROOT}" ${FIX_FLAG:+${FIX_FLAG}} || rc=$?
  fi
  popd >/dev/null
fi

# 2) Structural validation.
if (( DO_VALIDATE )); then
  echo
  echo "=== Phase 1b: structural validation ==="
  if [[ -n "${PLUGIN_NAME}" ]]; then
    python3 "${VALIDATE_SCRIPT}" "${PLUGINS_ROOT}/${PLUGIN_NAME}" || rc=$?
  else
    for plugin_dir in "${PLUGINS_ROOT}"/*/; do
      [[ -d "${plugin_dir}" ]] || continue
      python3 "${VALIDATE_SCRIPT}" "${plugin_dir%/}" || rc=$?
    done
  fi
fi

# 3) Hook modernization.
if (( DO_MODERNIZE )); then
  echo
  echo "=== Phase 1c: hook modernization audit ==="
  python3 "${MODERNIZE_SCRIPT}" --root "${REPO_ROOT}" || rc=$?
fi

exit "${rc:-0}"
