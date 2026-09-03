#!/usr/bin/env bash
# Fail when a Rust source file is not reachable from its crate root.
#
# A file under `crates/*/src` that no `mod` declaration names is never
# compiled: rustc, clippy and coverage all skip it, so it rots silently
# while still reading like live code. Four such files (3,256 lines)
# accumulated in crates/sync before this check existed.
#
# The check is deliberately a name check, not a full module-graph walk:
# every non-root file must have its stem declared as a module somewhere
# in the same crate. That catches an undeclared file without needing to
# resolve paths.
#
# Usage: ./scripts/lint-module-reachability.sh
set -euo pipefail

orphans=""

for crate_src in crates/*/src; do
  [ -d "${crate_src}" ] || continue
  crate_name="${crate_src%/src}"
  crate_name="${crate_name##*/}"

  while IFS= read -r file; do
    base="${file##*/}"
    case "${base}" in
      lib.rs | main.rs) continue ;;
    esac

    # `foo/mod.rs` is declared as `mod foo;`, `foo.rs` as `mod foo;`.
    if [ "${base}" = "mod.rs" ]; then
      dir="${file%/mod.rs}"
      stem="${dir##*/}"
    else
      stem="${base%.rs}"
    fi

    if ! grep -rqE "^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?mod[[:space:]]+${stem}[[:space:]]*;" \
      "${crate_src}"; then
      orphans="${orphans}${file}"$'\n'
    fi
  done < <(find "${crate_src}" -name '*.rs' -type f)
done

if [ -n "${orphans}" ]; then
  echo "ERROR: Rust files not reachable from their crate root:" >&2
  printf '%s' "${orphans}" >&2
  echo "" >&2
  echo "Declare each with a 'mod' item, or delete it. An undeclared file" >&2
  echo "is never compiled and never checked." >&2
  exit 1
fi

echo "module-reachability lint clean (every crate source file is declared)"
