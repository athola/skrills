#!/usr/bin/env bash
# Fail when a Rust source file is not declared as a module in its crate.
#
# A file under `crates/*/src` that no `mod` declaration names is never
# compiled: rustc, clippy and coverage all skip it, so it rots silently
# while still reading like live code. Undeclared files accumulated in
# crates/sync and crates/intelligence before this check existed.
#
# The check is a declaration check, not a full module-graph walk: every
# non-root file must be named by its PARENT module, which is where rustc
# looks. A `mod <stem>;` in some unrelated module of the same crate does
# not compile the file, and stems repeat across the adapter modules in
# crates/sync, so a crate-wide search would pass an orphan there.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' "Exit: 0 every file declared, 1 an undeclared file, 2 nothing scanned."
  exit 0
}

# Append to the caller's `orphans` each .rs file under crate source dir $1
# that its parent module does not declare, one per line.
crate_orphans() {
  local crate_src="${1:?crate_orphans needs a crate source dir}"
  local path_values file base dir stem parent_dir parent declared path_value
  local -a parents

  # `#[path = "..."]` compiles a file under a name of its own, and the value
  # may carry directory components ("gen/impl_a.rs"). Collect every value in
  # the crate once, then match each file by path suffix.
  path_values="$(grep -rhoE '#\[path[[:space:]]*=[[:space:]]*"[^"]+"' "${crate_src}" |
    sed -E 's/.*"([^"]+)"/\1/' | sort -u)" || path_values=""

  while IFS= read -r file; do
    base="${file##*/}"
    case "${base}" in
      lib.rs | main.rs) continue ;;
    esac
    # Cargo auto-discovers src/bin/*.rs as binary targets, which need no
    # `mod` item anywhere.
    case "${file}" in
      */src/bin/*) continue ;;
    esac

    # `foo/mod.rs` is declared as `mod foo;` by the module that owns `foo/`'s
    # parent directory; `foo.rs` by the module that owns its own directory.
    case "${base}" in
      mod.rs)
        dir="${file%/mod.rs}"
        stem="${dir##*/}"
        parent_dir="${dir%/*}"
        ;;
      *)
        stem="${base%.rs}"
        parent_dir="${file%/*}"
        ;;
    esac

    # The parent module is `<parent_dir>/mod.rs` or the sibling
    # `<parent_dir>.rs`, except at the crate source root where it is the
    # crate root file.
    case "${parent_dir}" in
      "${crate_src}") parents=("${crate_src}/lib.rs" "${crate_src}/main.rs") ;;
      *) parents=("${parent_dir}/mod.rs" "${parent_dir}.rs") ;;
    esac

    declared=0
    for parent in "${parents[@]}"; do
      [ -f "${parent}" ] || continue
      if grep -qE "^[[:space:]]*(#\[[^]]*\][[:space:]]*)*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+(r#)?${stem}[[:space:]]*;" \
        "${parent}"; then
        declared=1
        break
      fi
    done
    case "${declared}" in
      1) continue ;;
    esac

    while IFS= read -r path_value; do
      case "${path_value}" in
        "") continue ;;
      esac
      case "${file}" in
        *"/${path_value}")
          declared=1
          break
          ;;
      esac
    done <<<"${path_values}"
    case "${declared}" in
      1) continue ;;
    esac

    orphans="${orphans}${file}"$'\n'
  done < <(find "${crate_src}" -name '*.rs' -type f)
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

  # Anchor on the script's own location. Run from another directory the
  # `crates/*/src` glob matches nothing, and the gate used to report clean.
  (
    cd "${MYDIR%/}/.."
    local crate_src orphans="" scanned=0
    for crate_src in crates/*/src; do
      [ -d "${crate_src}" ] || continue
      scanned=$((scanned + 1))
      crate_orphans "${crate_src}"
    done

    case "${scanned}" in
      0)
        log 3 "ERROR: no crates/*/src directory found under $(pwd)."
        log 3 "The module-reachability lint scanned nothing, so it proved nothing."
        exit 2
        ;;
    esac

    case "${orphans}" in
      "") ;;
      *)
        log 3 "ERROR: Rust files not reachable from their crate root:"
        printf '%s' "${orphans}" >&2
        log 3 ""
        log 3 "Declare each in its parent module with a 'mod' item, or delete it."
        log 3 "An undeclared file is never compiled and never checked."
        exit 1
        ;;
    esac

    # stdout, not log: callers and tests read this line as the clean verdict.
    printf '%s\n' "module-reachability lint clean (${scanned} crates, every source file declared)"
  )
}

main "$@"
