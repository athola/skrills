#!/usr/bin/env bash
# Publish every workspace crate to crates.io in dependency order, skipping
# versions crates.io already has. verify_publish_order.sh reads the
# `publish_one` lines in main() as the publish order.
set -euo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly REQUIRED_DEPENDENCIES="jq curl"

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' "Env: CARGO_REGISTRY_TOKEN (required) authenticates cargo publish."
  exit 0
}

require_token() {
  : "${CARGO_REGISTRY_TOKEN:?CARGO_REGISTRY_TOKEN is not set; cannot publish.}"
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

# Read a workspace member's version from `cargo metadata` via jq. Robust to
# workspace-inherited versions, unlike grepping a crate's Cargo.toml.
crate_version() {
  local crate="${1}" version
  version=$(cargo metadata --no-deps --format-version 1 |
    jq -r --arg crate "${crate}" 'first(.packages[] | select(.name == $crate) | .version) // empty')
  case "${version}" in
    "")
      log 3 "crate not found: ${crate}"
      return 1
      ;;
  esac
  printf '%s\n' "${version}"
}

# Ask crates.io for the exact version: 200 means published, 404 means not.
# `cargo search` ranks results, so a ranking change read as "not published"
# and the re-run then aborted on cargo's "already exists" error.
already_published() {
  local crate="${1}"
  local version="${2}"
  local status
  status=$(curl -sS -o /dev/null -w '%{http_code}' \
    -A "skrills-publish (https://github.com/athola/skrills)" \
    "https://crates.io/api/v1/crates/${crate}/${version}") || {
    log 3 "ERROR: crates.io lookup failed for ${crate} ${version} (network issue?)"
    exit 1
  }
  case "${status}" in
    200) return 0 ;;
    404) return 1 ;;
    *)
      log 3 "ERROR: crates.io returned HTTP ${status} for ${crate} ${version}"
      exit 1
      ;;
  esac
}

publish_one() {
  local crate="${1}" version
  version="$(crate_version "${crate}")"
  if already_published "${crate}" "${version}"; then
    log 6 "Skipping ${crate} v${version} (already on crates.io)"
    return
  fi
  log 6 "Publishing ${crate} v${version}"
  # cargo publish (1.66+) waits until the index lists the new version, so
  # dependents published next can resolve it.
  cargo publish -p "${crate}"
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

  require_token
  depcheck

  # Level 0: leaf crates (no internal dependencies)
  publish_one skrills-snapshot
  publish_one skrills-validate
  publish_one skrills-state
  publish_one skrills-discovery
  publish_one skrills-metrics
  publish_one skrills-tome

  # Level 1: depend on crates published above, in this order
  publish_one skrills_sync
  publish_one skrills-setup
  publish_one skrills-subagents
  publish_one skrills-intelligence
  publish_one skrills-analyze
  publish_one skrills-dashboard

  # Level 2: server depends on all above
  publish_one skrills-server

  # Level 3: cli depends on server
  publish_one skrills
}

main "$@"
