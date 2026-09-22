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
#
# Usage: ./scripts/lint-module-reachability.sh
# Exit:  0 = every file declared, 1 = an undeclared file, 2 = nothing scanned.
set -euo pipefail

# Anchor on the script's own location. Run from another directory the
# `crates/*/src` glob matches nothing, and the gate used to report clean.
case "${BASH_SOURCE[0]}" in
  */*) script_dir="${BASH_SOURCE[0]%/*}" ;;
  *) script_dir="." ;;
esac
cd "${script_dir}/.."

orphans=""
scanned=0

for crate_src in crates/*/src; do
  [ -d "${crate_src}" ] || continue
  scanned=$((scanned + 1))

  # `#[path = "..."]` compiles a file under a name of its own, and the value
  # may carry directory components ("gen/impl_a.rs"). Collect every value in
  # the crate once, then match each file by path suffix.
  path_values="$(grep -rhoE '#\[path[[:space:]]*=[[:space:]]*"[^"]+"' "${crate_src}" \
    | sed -E 's/.*"([^"]+)"/\1/' | sort -u)" || path_values=""

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
    if [ "${base}" = "mod.rs" ]; then
      dir="${file%/mod.rs}"
      stem="${dir##*/}"
      parent_dir="${dir%/*}"
    else
      stem="${base%.rs}"
      parent_dir="${file%/*}"
    fi

    # The parent module is `<parent_dir>/mod.rs` or the sibling
    # `<parent_dir>.rs`, except at the crate source root where it is the
    # crate root file.
    if [ "${parent_dir}" = "${crate_src}" ]; then
      parents=("${crate_src}/lib.rs" "${crate_src}/main.rs")
    else
      parents=("${parent_dir}/mod.rs" "${parent_dir}.rs")
    fi

    declared=0
    for parent in "${parents[@]}"; do
      [ -f "${parent}" ] || continue
      if grep -qE "^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+${stem}[[:space:]]*;" \
        "${parent}"; then
        declared=1
        break
      fi
    done
    [ "${declared}" -eq 1 ] && continue

    while IFS= read -r path_value; do
      [ -n "${path_value}" ] || continue
      case "${file}" in
        *"/${path_value}") declared=1; break ;;
      esac
    done <<<"${path_values}"
    [ "${declared}" -eq 1 ] && continue

    orphans="${orphans}${file}"$'\n'
  done < <(find "${crate_src}" -name '*.rs' -type f)
done

if [ "${scanned}" -eq 0 ]; then
  echo "ERROR: no crates/*/src directory found under $(pwd)." >&2
  echo "The module-reachability lint scanned nothing, so it proved nothing." >&2
  exit 2
fi

if [ -n "${orphans}" ]; then
  echo "ERROR: Rust files not reachable from their crate root:" >&2
  printf '%s' "${orphans}" >&2
  echo "" >&2
  echo "Declare each in its parent module with a 'mod' item, or delete it." >&2
  echo "An undeclared file is never compiled and never checked." >&2
  exit 1
fi

echo "module-reachability lint clean (${scanned} crates, every source file declared)"
