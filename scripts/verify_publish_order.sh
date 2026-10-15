#!/usr/bin/env bash
# Check that publish_crates.sh publishes every publishable workspace crate,
# names no unknown crate, and publishes each crate after the workspace crates
# it depends on (dev-dependencies excluded).
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

usage() {
  log "Usage: ${0##*/} [-h] [-x] [--print-order]"
  printf '  %-15s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)" \
    "--print-order" "Print the publish order from publish_crates.sh and exit"
  printf '  %s\n' "Needs bash 4+ (associative arrays)."
  exit 0
}

# Validate crate name before using as associative array key
# Valid crate names: non-empty, contain only alphanumeric, underscore, or hyphen
validate_crate_name() {
  local name="${1}"
  local context="${2:-crate}"
  case "${name}" in
    "")
      log 3 "[ERROR] Empty ${context} name detected"
      return 1
      ;;
    *[!a-zA-Z0-9_-]*)
      log 3 "[ERROR] Invalid ${context} name '${name}': contains invalid characters"
      return 1
      ;;
  esac
  return 0
}

# The three helpers below run inside verify_order's manifest loop and read
# and reset its parser state (dep_table_*, inline_*, name, added_dep, DEPS)
# through bash's dynamic scoping.

finalize_dep_table() {
  if ((dep_table_active)) && ((dep_table_allow)); then
    if ((dep_table_has_path)) || ((dep_table_workspace)); then
      dep_name="${dep_table_pkg:-${dep_table_name}}"
      add_dep "${dep_name}"
    fi
  fi
  dep_table_active=0
  dep_table_name=""
  dep_table_pkg=""
  dep_table_has_path=0
  dep_table_workspace=0
  dep_table_allow=0
}

finalize_inline() {
  if ((inline_active)); then
    if ((inline_has_path)) || ((inline_workspace)); then
      dep_name="${inline_pkg:-${inline_dep}}"
      add_dep "${dep_name}"
    fi
  fi
  inline_active=0
  inline_dep=""
  inline_pkg=""
  inline_has_path=0
  inline_workspace=0
}

add_dep() {
  local dep_name="${1}"
  local key="${name}|${dep_name}"
  if [[ -z "${added_dep[${key}]+x}" ]]; then
    DEPS+=("${name} ${dep_name}")
    added_dep[${key}]=1
  fi
}

# Walk the workspace manifests and compare their dependency edges against the
# publish order. $1 is 1 to print the order and stop.
verify_order() {
  local print_order="${1:-0}"
  local ROOT
  ROOT="$(cd "${MYDIR%/}/.." && pwd)"
  PUBLISH_SCRIPT="${ROOT}/scripts/publish_crates.sh"

  if [ ! -f "${PUBLISH_SCRIPT}" ]; then
    log 3 "Missing publish script at ${PUBLISH_SCRIPT}"
    exit 1
  fi

  ORDER=()
  while IFS= read -r entry; do
    ORDER+=("${entry}")
  done < <(awk '/^[[:space:]]*publish_one[[:space:]]+/ {print $2}' "${PUBLISH_SCRIPT}")
  case "${#ORDER[@]}" in
    0)
      log 3 "No publish_one entries found in scripts/publish_crates.sh"
      exit 1
      ;;
  esac

  case "${print_order}" in
    1)
      # stdout, not log: callers parse the order.
      printf '%s\n' "${ORDER[*]}"
      exit 0
      ;;
  esac

  WORKSPACE_MEMBERS=()
  in_members=0
  while read -r line; do
    if [[ "${line}" == *"members"* && "${line}" == *"["* ]]; then
      in_members=1
    fi
    if ((in_members)); then
      if matches=$(grep -oE '"[^"]+"' <<<"${line}" 2>/dev/null); then
        while read -r match; do
          member="${match%\"}"
          member="${member#\"}"
          WORKSPACE_MEMBERS+=("${member}")
        done <<<"${matches}"
      fi
      if [[ "${line}" == *"]"* ]]; then
        in_members=0
      fi
    fi
  done <"${ROOT}/Cargo.toml"

  case "${#WORKSPACE_MEMBERS[@]}" in
    0)
      log 3 "No workspace members found in Cargo.toml"
      exit 1
      ;;
  esac

  # Expand glob members (`crates/*`) the way cargo does: every matching
  # directory that holds a Cargo.toml.
  EXPANDED_MEMBERS=()
  for member in "${WORKSPACE_MEMBERS[@]}"; do
    if [[ "${member}" == *[*?[]* ]]; then
      for dir in "${ROOT}"/${member}; do
        [[ -f "${dir}/Cargo.toml" ]] && EXPANDED_MEMBERS+=("${dir#"${ROOT}"/}")
      done
    else
      EXPANDED_MEMBERS+=("${member}")
    fi
  done
  WORKSPACE_MEMBERS=("${EXPANDED_MEMBERS[@]}")

  # `publish = false` and `publish = []` both mean "never publish"; a member
  # with `publish.workspace = true` inherits [workspace.package] publish.
  UNPUBLISHED_RE='^publish[[:space:]]*=[[:space:]]*(false|\[[[:space:]]*\])'
  INHERIT_RE='^publish(\.workspace[[:space:]]*=[[:space:]]*true|[[:space:]]*=[[:space:]]*\{[[:space:]]*workspace[[:space:]]*=[[:space:]]*true)'
  workspace_publishable=1
  root_section=""
  while read -r line; do
    line="${line%%#*}"
    if [[ "${line}" =~ ^\[.*\]$ ]]; then
      root_section="${line}"
    elif [[ "${root_section}" == "[workspace.package]" && "${line}" =~ ${UNPUBLISHED_RE} ]]; then
      workspace_publishable=0
    fi
  done <"${ROOT}/Cargo.toml"

  WORKSPACE_NAMES=()
  PUBLISHABLE_NAMES=()
  DEPS=()
  for member in "${WORKSPACE_MEMBERS[@]}"; do
    manifest="${ROOT}/${member}/Cargo.toml"
    if [[ ! -f "${manifest}" ]]; then
      log 3 "Missing manifest at ${manifest}"
      exit 1
    fi

    name=""
    publishable=1
    in_section=""
    while read -r line; do
      line="${line%%#*}"
      [[ -z "${line//[[:space:]]/}" ]] && continue
      if [[ "${line}" =~ ^\[.*\]$ ]]; then
        in_section="${line}"
        continue
      fi
      if [[ "${in_section}" == "[package]" ]]; then
        if [[ "${line}" =~ ^name[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
          name="${BASH_REMATCH[1]}"
        elif [[ "${line}" =~ ${UNPUBLISHED_RE} ]]; then
          publishable=0
        elif [[ "${line}" =~ ${INHERIT_RE} ]]; then
          publishable="${workspace_publishable}"
        fi
      fi
    done <"${manifest}"

    case "${name}" in
      "")
        log 3 "Missing package name in ${manifest}"
        exit 1
        ;;
    esac

    WORKSPACE_NAMES+=("${name}")
    if ((publishable)); then
      PUBLISHABLE_NAMES+=("${name}")
    fi

    in_section=""
    dep_table_active=0
    dep_table_name=""
    dep_table_pkg=""
    dep_table_has_path=0
    dep_table_workspace=0
    dep_table_allow=0
    inline_active=0
    inline_dep=""
    inline_pkg=""
    inline_has_path=0
    inline_workspace=0
    declare -A added_dep=()
    declare -A dot_seen=()
    declare -A dot_has_path=()
    declare -A dot_workspace=()
    declare -A dot_pkg=()

    dep_section_allow=0
    while read -r line; do
      line="${line%%#*}"
      [[ -z "${line//[[:space:]]/}" ]] && continue
      if [[ "${line}" =~ ^\[.*\]$ ]]; then
        finalize_dep_table
        finalize_inline
        in_section="${line}"
        dep_section_allow=0
        if [[ "${in_section}" == "[dependencies]" || "${in_section}" == "[build-dependencies]" ]]; then
          dep_section_allow=1
        elif [[ "${in_section}" == "[dev-dependencies]" ]]; then
          dep_section_allow=0
        elif [[ "${in_section}" =~ ^\[dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=1
          dep_table_active=1
          dep_table_name="${BASH_REMATCH[1]%\"}"
          dep_table_name="${dep_table_name#\"}"
          dep_table_allow=1
        elif [[ "${in_section}" =~ ^\[build-dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=1
          dep_table_active=1
          dep_table_name="${BASH_REMATCH[1]%\"}"
          dep_table_name="${dep_table_name#\"}"
          dep_table_allow=1
        elif [[ "${in_section}" =~ ^\[dev-dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=0
        elif [[ "${in_section}" =~ ^\[target\..*\.dependencies\]$ ]]; then
          dep_section_allow=1
        elif [[ "${in_section}" =~ ^\[target\..*\.build-dependencies\]$ ]]; then
          dep_section_allow=1
        elif [[ "${in_section}" =~ ^\[target\..*\.dev-dependencies\]$ ]]; then
          dep_section_allow=0
        elif [[ "${in_section}" =~ ^\[target\..*\.dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=1
          dep_table_active=1
          dep_table_name="${BASH_REMATCH[1]%\"}"
          dep_table_name="${dep_table_name#\"}"
          dep_table_allow=1
        elif [[ "${in_section}" =~ ^\[target\..*\.build-dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=1
          dep_table_active=1
          dep_table_name="${BASH_REMATCH[1]%\"}"
          dep_table_name="${dep_table_name#\"}"
          dep_table_allow=1
        elif [[ "${in_section}" =~ ^\[target\..*\.dev-dependencies\.([^\]]+)\]$ ]]; then
          dep_section_allow=0
        fi
        continue
      fi
      if ((dep_table_active)) && ((dep_table_allow)); then
        if [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
          dep_table_pkg="${BASH_REMATCH[1]}"
        elif [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
          dep_table_pkg="${BASH_REMATCH[1]}"
        fi
        if [[ "${line}" =~ ^path[[:space:]]*= ]]; then
          dep_table_has_path=1
        fi
        if [[ "${line}" =~ workspace[[:space:]]*=[[:space:]]*true ]]; then
          dep_table_workspace=1
        fi
        continue
      fi
      if ((dep_section_allow)); then
        if ((inline_active)); then
          if [[ "${line}" =~ path[[:space:]]*= ]]; then
            inline_has_path=1
          fi
          if [[ "${line}" =~ workspace[[:space:]]*=[[:space:]]*true ]]; then
            inline_workspace=1
          fi
          if [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
            inline_pkg="${BASH_REMATCH[1]}"
          elif [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
            inline_pkg="${BASH_REMATCH[1]}"
          fi
          if [[ "${line}" == *"}"* ]]; then
            finalize_inline
          fi
          continue
        fi
        dep=""
        if [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.path[[:space:]]*= ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_has_path[${dep}]=1
          continue
        elif [[ "${line}" =~ ^\"([^\"]+)\"\.path[[:space:]]*= ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_has_path[${dep}]=1
          continue
        elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.workspace[[:space:]]*=[[:space:]]*true ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_workspace[${dep}]=1
          continue
        elif [[ "${line}" =~ ^\"([^\"]+)\"\.workspace[[:space:]]*=[[:space:]]*true ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_workspace[${dep}]=1
          continue
        elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_pkg[${dep}]="${BASH_REMATCH[2]}"
          continue
        elif [[ "${line}" =~ ^\"([^\"]+)\"\.package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_pkg[${dep}]="${BASH_REMATCH[2]}"
          continue
        elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_pkg[${dep}]="${BASH_REMATCH[2]}"
          continue
        elif [[ "${line}" =~ ^\"([^\"]+)\"\.package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
          dep="${BASH_REMATCH[1]}"
          dot_seen[${dep}]=1
          dot_pkg[${dep}]="${BASH_REMATCH[2]}"
          continue
        fi
        dep=""
        if [[ "${line}" =~ ^([A-Za-z0-9_-]+)[[:space:]]*= ]]; then
          dep="${BASH_REMATCH[1]}"
        elif [[ "${line}" =~ ^\"([^\"]+)\"[[:space:]]*= ]]; then
          dep="${BASH_REMATCH[1]}"
        fi
        if [[ -n "${dep}" ]]; then
          if [[ "${line}" == *"{"* ]]; then
            inline_active=1
            inline_dep="${dep}"
            inline_has_path=0
            inline_workspace=0
            inline_pkg=""
            if [[ "${line}" =~ path[[:space:]]*= ]]; then
              inline_has_path=1
            fi
            if [[ "${line}" =~ workspace[[:space:]]*=[[:space:]]*true ]]; then
              inline_workspace=1
            fi
            if [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
              inline_pkg="${BASH_REMATCH[1]}"
            elif [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
              inline_pkg="${BASH_REMATCH[1]}"
            fi
            if [[ "${line}" == *"}"* ]]; then
              finalize_inline
            fi
          else
            if [[ "${line}" =~ workspace[[:space:]]*=[[:space:]]*true ]]; then
              DEPS+=("${name} ${dep}")
            fi
          fi
        fi
      fi
    done <"${manifest}"
    finalize_dep_table
    finalize_inline
    for dep in "${!dot_seen[@]}"; do
      if [[ -n "${dot_has_path[${dep}]+x}" ]] || [[ -n "${dot_workspace[${dep}]+x}" ]]; then
        dep_name="${dot_pkg[${dep}]:-${dep}}"
        add_dep "${dep_name}"
      fi
    done
  done

  declare -A ORDER_INDEX=()
  declare -A WORKSPACE_SET=()

  # Validate crate names before using as associative array keys
  for idx in "${!ORDER[@]}"; do
    crate_name="${ORDER[${idx}]}"
    validate_crate_name "${crate_name}" "publish order" || exit 1
    ORDER_INDEX["${crate_name}"]="${idx}"
  done
  for name in "${WORKSPACE_NAMES[@]}"; do
    validate_crate_name "${name}" "workspace" || exit 1
    WORKSPACE_SET["${name}"]=1
  done

  errors=()
  missing=()
  for name in "${PUBLISHABLE_NAMES[@]}"; do
    if [[ -z "${ORDER_INDEX[${name}]+x}" ]]; then
      missing+=("${name}")
    fi
  done
  if ((${#missing[@]})); then
    errors+=("Publish order missing workspace crates: ${missing[*]}")
  fi

  extras=()
  for name in "${ORDER[@]}"; do
    if [[ -z "${WORKSPACE_SET[${name}]+x}" ]]; then
      extras+=("${name}")
    fi
  done
  if ((${#extras[@]})); then
    errors+=("Publish order includes unknown crates: ${extras[*]}")
  fi

  # Warn if no internal dependencies were detected - could indicate a parser issue
  if ((${#DEPS[@]} == 0)); then
    log 4 "[WARN] No internal dependencies detected - verify parser is working correctly"
  fi

  for pair in "${DEPS[@]}"; do
    pkg="${pair%% *}"
    dep="${pair##* }"
    if [[ -z "${WORKSPACE_SET[${dep}]+x}" ]]; then
      continue
    fi
    if [[ -z "${ORDER_INDEX[${dep}]+x}" ]]; then
      errors+=("${pkg} depends on ${dep} but it is not in publish order")
      continue
    fi
    if [[ -n "${ORDER_INDEX[${pkg}]+x}" ]] && ((ORDER_INDEX["${dep}"] > ORDER_INDEX["${pkg}"])); then
      errors+=("${pkg} appears before dependency ${dep}")
    fi
  done

  if ((${#errors[@]})); then
    for error in "${errors[@]}"; do
      log 3 "[ERROR] ${error}"
    done
    exit 1
  fi

  log 6 "Publish order validated against workspace dependencies."
}

main() {
  local print_order=0
  case "${1:-}" in
    *[uU][sS][aA][gG][eE] | *[hH][eE][lL][pP]) usage ;;
  esac
  while [ "${#}" -gt 0 ]; do
    case "${1}" in
      -h) usage ;;
      -x) set -x ;;
      --print-order) print_order=1 ;;
      *)
        log 3 "Unknown argument: ${1}"
        exit 64
        ;;
    esac
    shift
  done

  # The dependency walk uses associative arrays. Stock macOS bash is 3.2,
  # where `declare -A` fails with a bare usage line, so say what is needed.
  if ((BASH_VERSINFO[0] < 4)); then
    log 3 "ERROR: verify_publish_order.sh needs bash 4+ (associative arrays); found ${BASH_VERSION}."
    log 3 "       Install a newer bash (brew install bash) or run 'make verify-publish'."
    exit 1
  fi

  verify_order "${print_order}"
}

main "$@"
