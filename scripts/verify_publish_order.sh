#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PUBLISH_SCRIPT="${ROOT}/scripts/publish_crates.sh"

if [[ ! -f "${PUBLISH_SCRIPT}" ]]; then
  echo "Missing publish script at ${PUBLISH_SCRIPT}" >&2
  exit 1
fi

mapfile -t ORDER < <(awk '/^[[:space:]]*publish_one[[:space:]]+/ {print $2}' "${PUBLISH_SCRIPT}")
if [[ ${#ORDER[@]} -eq 0 ]]; then
  echo "No publish_one entries found in scripts/publish_crates.sh" >&2
  exit 1
fi

if [[ "${1:-}" == "--print-order" ]]; then
  echo "${ORDER[*]}"
  exit 0
fi

WORKSPACE_MEMBERS=()
in_members=0
while read -r line; do
  if [[ "${line}" == *"members"* && "${line}" == *"["* ]]; then
    in_members=1
  fi
  if (( in_members )); then
    if matches=$(grep -oE '"[^"]+"' <<< "${line}" 2>/dev/null); then
      while read -r match; do
        member="${match%\"}"
        member="${member#\"}"
        WORKSPACE_MEMBERS+=("${member}")
      done <<< "${matches}"
    fi
    if [[ "${line}" == *"]"* ]]; then
      in_members=0
    fi
  fi
done < "${ROOT}/Cargo.toml"

if [[ ${#WORKSPACE_MEMBERS[@]} -eq 0 ]]; then
  echo "No workspace members found in Cargo.toml" >&2
  exit 1
fi

WORKSPACE_NAMES=()
PUBLISHABLE_NAMES=()
DEPS=()
for member in "${WORKSPACE_MEMBERS[@]}"; do
  manifest="${ROOT}/${member}/Cargo.toml"
  if [[ ! -f "${manifest}" ]]; then
    echo "Missing manifest at ${manifest}" >&2
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
      elif [[ "${line}" =~ ^publish[[:space:]]*=[[:space:]]*false ]]; then
        publishable=0
      fi
    fi
  done < "${manifest}"

  if [[ -z "${name}" ]]; then
    echo "Missing package name in ${manifest}" >&2
    exit 1
  fi

  WORKSPACE_NAMES+=("${name}")
  if (( publishable )); then
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

  finalize_dep_table() {
    if (( dep_table_active )) && (( dep_table_allow )); then
      if (( dep_table_has_path )) || (( dep_table_workspace )); then
        dep_name="${dep_table_pkg:-$dep_table_name}"
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
    if (( inline_active )); then
      if (( inline_has_path )) || (( inline_workspace )); then
        dep_name="${inline_pkg:-$inline_dep}"
        add_dep "${dep_name}"
      fi
    fi
    inline_active=0
    inline_dep=""
    inline_pkg=""
    inline_has_path=0
    inline_workspace=0
  }

  dep_section_allow=0
  add_dep() {
    local dep_name="$1"
    local key="${name}|${dep_name}"
    if [[ -z "${added_dep[$key]+x}" ]]; then
      DEPS+=("${name} ${dep_name}")
      added_dep[$key]=1
    fi
  }
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
    if (( dep_table_active )) && (( dep_table_allow )); then
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
    if (( dep_section_allow )); then
      if (( inline_active )); then
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
        dot_seen[$dep]=1
        dot_has_path[$dep]=1
        continue
      elif [[ "${line}" =~ ^\"([^\"]+)\"\.path[[:space:]]*= ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_has_path[$dep]=1
        continue
      elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.workspace[[:space:]]*=[[:space:]]*true ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_workspace[$dep]=1
        continue
      elif [[ "${line}" =~ ^\"([^\"]+)\"\.workspace[[:space:]]*=[[:space:]]*true ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_workspace[$dep]=1
        continue
      elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_pkg[$dep]="${BASH_REMATCH[2]}"
        continue
      elif [[ "${line}" =~ ^\"([^\"]+)\"\.package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_pkg[$dep]="${BASH_REMATCH[2]}"
        continue
      elif [[ "${line}" =~ ^([A-Za-z0-9_-]+)\.package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_pkg[$dep]="${BASH_REMATCH[2]}"
        continue
      elif [[ "${line}" =~ ^\"([^\"]+)\"\.package[[:space:]]*=[[:space:]]*\'([^\']+)\' ]]; then
        dep="${BASH_REMATCH[1]}"
        dot_seen[$dep]=1
        dot_pkg[$dep]="${BASH_REMATCH[2]}"
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
  done < "${manifest}"
  finalize_dep_table
  finalize_inline
  for dep in "${!dot_seen[@]}"; do
    if [[ -n "${dot_has_path[$dep]+x}" ]] || [[ -n "${dot_workspace[$dep]+x}" ]]; then
      dep_name="${dot_pkg[$dep]:-$dep}"
      add_dep "${dep_name}"
    fi
  done
done

declare -A ORDER_INDEX=()
declare -A WORKSPACE_SET=()
for idx in "${!ORDER[@]}"; do
  ORDER_INDEX["${ORDER[$idx]}"]="${idx}"
done
for name in "${WORKSPACE_NAMES[@]}"; do
  WORKSPACE_SET["${name}"]=1
done

errors=()
missing=()
for name in "${PUBLISHABLE_NAMES[@]}"; do
  if [[ -z "${ORDER_INDEX[$name]+x}" ]]; then
    missing+=("${name}")
  fi
done
if (( ${#missing[@]} )); then
  errors+=("Publish order missing workspace crates: ${missing[*]}")
fi

extras=()
for name in "${ORDER[@]}"; do
  if [[ -z "${WORKSPACE_SET[$name]+x}" ]]; then
    extras+=("${name}")
  fi
done
if (( ${#extras[@]} )); then
  errors+=("Publish order includes unknown crates: ${extras[*]}")
fi

for pair in "${DEPS[@]}"; do
  pkg="${pair%% *}"
  dep="${pair##* }"
  if [[ -z "${WORKSPACE_SET[$dep]+x}" ]]; then
    continue
  fi
  if [[ -z "${ORDER_INDEX[$dep]+x}" ]]; then
    errors+=("${pkg} depends on ${dep} but it is not in publish order")
    continue
  fi
  if [[ -n "${ORDER_INDEX[$pkg]+x}" ]] && (( ORDER_INDEX["$dep"] > ORDER_INDEX["$pkg"] )); then
    errors+=("${pkg} appears before dependency ${dep}")
  fi
done

if (( ${#errors[@]} )); then
  for error in "${errors[@]}"; do
    echo "[ERROR] ${error}" >&2
  done
  exit 1
fi

echo "Publish order validated against workspace dependencies."
