#!/usr/bin/env bash

set -euo pipefail

require_token() {
  if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
    echo "CARGO_REGISTRY_TOKEN is not set; cannot publish." >&2
    exit 1
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "$1 is required but was not found on PATH." >&2
    exit 1
  }
}

# Read a workspace member's version from `cargo metadata` via jq. Robust to
# workspace-inherited versions, unlike grepping a crate's Cargo.toml.
crate_version() {
  local crate="$1" version
  version=$(cargo metadata --no-deps --format-version 1 \
    | jq -r --arg crate "$crate" 'first(.packages[] | select(.name == $crate) | .version) // empty')
  if [ -z "$version" ]; then
    echo "crate not found: $crate" >&2
    return 1
  fi
  printf '%s\n' "$version"
}

# Ask crates.io for the exact version: 200 means published, 404 means not.
# `cargo search` ranks results, so a ranking change read as "not published"
# and the re-run then aborted on cargo's "already exists" error.
already_published() {
  local crate="$1"
  local version="$2"
  local status
  status=$(curl -sS -o /dev/null -w '%{http_code}' \
    -A "skrills-publish (https://github.com/athola/skrills)" \
    "https://crates.io/api/v1/crates/${crate}/${version}") || {
    echo "ERROR: crates.io lookup failed for $crate $version (network issue?)" >&2
    exit 1
  }
  case "$status" in
    200) return 0 ;;
    404) return 1 ;;
    *)
      echo "ERROR: crates.io returned HTTP $status for $crate $version" >&2
      exit 1
      ;;
  esac
}

publish_one() {
  crate="$1"
  version="$(crate_version "$crate")"
  if already_published "$crate" "$version"; then
    echo "Skipping $crate v$version (already on crates.io)"
    return
  fi
  echo "Publishing $crate v$version"
  # cargo publish (1.66+) waits until the index lists the new version, so
  # dependents published next can resolve it.
  cargo publish -p "$crate"
}

require_token
require_cmd jq
require_cmd curl

# Level 0: leaf crates (no internal dependencies)
publish_one skrills-snapshot
publish_one skrills-validate
publish_one skrills-state
publish_one skrills-discovery
publish_one skrills-intelligence
publish_one skrills-metrics
publish_one skrills-tome

# Level 1: depend on leaf crates only
publish_one skrills_sync
publish_one skrills-subagents
publish_one skrills-analyze
publish_one skrills-dashboard

# Level 2: server depends on all above
publish_one skrills-server

# Level 3: cli depends on server
publish_one skrills
