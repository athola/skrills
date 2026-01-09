#!/bin/bash

set -euo pipefail

require_token() {
  if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
    echo "CARGO_REGISTRY_TOKEN is not set; cannot publish." >&2
    exit 1
  fi
}

crate_version() {
  python -c '
import json, sys, subprocess
crate = sys.argv[1]
meta = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"]))
for pkg in meta["packages"]:
    if pkg["name"] == crate:
        print(pkg["version"])
        sys.exit(0)
print("crate not found: " + crate, file=sys.stderr)
sys.exit(1)
' "$1"
}

already_published() {
  crate="$1"
  version="$2"
  cargo search "$crate" --limit 1 | grep -q "$crate = \"$version\""
}

publish_one() {
  crate="$1"
  version="$(crate_version "$crate")"
  if already_published "$crate" "$version"; then
    echo "Skipping $crate v$version (already on crates.io)"
    return
  fi
  echo "Publishing $crate v$version"
  cargo publish -p "$crate"
  # allow index to update before dependents publish
  sleep 20
}

require_token

# Level 0: leaf crates (no internal dependencies)
publish_one skrills-validate
publish_one skrills-state
publish_one skrills-discovery
publish_one skrills-intelligence

# Level 1: depend on leaf crates only
publish_one skrills_sync
publish_one skrills-subagents
publish_one skrills-analyze

# Level 2: server depends on all above
publish_one skrills-server

# Level 3: cli depends on server
publish_one skrills
