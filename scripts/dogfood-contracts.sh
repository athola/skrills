#!/usr/bin/env bash
# dogfood-contracts.sh — behavior-driven dogfood for output contracts that
# only break in the field (CI, real installs), not in `cargo test`.
#
# Each FEATURE captures business behavior a downstream consumer depends on,
# expressed as Given/When/Then scenarios run against the REAL release binary
# and the REAL shipped tooling (the validate-skills GitHub Action entrypoint).
#
# Run:    ./scripts/dogfood-contracts.sh
# Env:    BIN_PATH=path/to/skrills   (default: target/release/skrills)
# Exit:   0 = all scenarios pass, 1 = a contract regressed, 2 = setup error.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_PATH="${BIN_PATH:-target/release/skrills}"
ENTRYPOINT=".github/actions/validate-skills/entrypoint.sh"
SKILL_DIR="skills"

GREEN='\033[0;32m'; RED='\033[0;31m'; CYAN='\033[0;36m'; DIM='\033[2m'; NC='\033[0m'
PASS=0; FAIL=0

feature() { printf "\n${CYAN}Feature:${NC} %s\n" "$1"; }
scenario() { printf "  ${DIM}Scenario:${NC} %s\n" "$1"; }
ok()   { printf "    ${GREEN}PASS${NC} %s\n" "$1"; PASS=$((PASS + 1)); }
bad()  { printf "    ${RED}FAIL${NC} %s\n" "$1"; FAIL=$((FAIL + 1)); }

# then <description> <command...>  — asserts the command succeeds (exit 0).
then_() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then ok "$desc"; else bad "$desc"; fi
}

# ---- preconditions ---------------------------------------------------------
[ -x "$BIN_PATH" ] || { echo "ERROR: binary not found at $BIN_PATH (run: make build)"; exit 2; }
[ -f "$ENTRYPOINT" ] || { echo "ERROR: entrypoint not found at $ENTRYPOINT"; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "ERROR: jq required for these contracts"; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIND="$WORK/bin"; mkdir -p "$BIND"
ln -sf "$REPO_ROOT/$BIN_PATH" "$BIND/skrills"

# run_entrypoint <home> <input_path> -> populates $RT for inspection, sets $EXIT
run_entrypoint() {
  local home="$1" input_path="$2"
  RT="$WORK/rt.$RANDOM"; mkdir -p "$RT"
  set +e
  HOME="$home" PATH="$BIND:$PATH" RUST_LOG=debug RUNNER_TEMP="$RT" \
    GITHUB_OUTPUT="$RT/gh.out" INPUT_PATH="$input_path" \
    INPUT_TARGETS="all" INPUT_STRICT="false" \
    bash "$ENTRYPOINT" </dev/null >"$RT/stdout.log" 2>"$RT/stderr.log"
  EXIT=$?
  set -e 2>/dev/null || true
}

# ===========================================================================
feature "skrills validate --format json stays machine-parseable on a noisy stdout"
# The validate-skills action pipes validate's stdout through jq. Three real
# regressions poisoned that stream: a first-run banner, interleaved tracing
# logs, and an ambient RUST_LOG that re-enabled logging. The action must
# recover clean JSON regardless. We drive the ACTUAL action entrypoint.

scenario "Guard check — an unconfigured runner with RUST_LOG really does pollute stdout"
# Given a fresh HOME (skrills not configured) and ambient RUST_LOG=debug
FRESH="$WORK/fresh-home"; mkdir -p "$FRESH"
HOME="$FRESH" RUST_LOG=debug "$BIN_PATH" validate --skill-dir "$SKILL_DIR" \
  --target all --format json </dev/null >"$WORK/raw.out" 2>/dev/null || true
# Then the raw stream carries the banner AND tracing lines AND is NOT valid JSON.
# (If any of these stop holding, the recovery scenario below is vacuous.)
if grep -q "not configured on this system" "$WORK/raw.out"; then ok "raw stdout carries the first-run banner"; else bad "raw stdout missing banner (scenario no longer meaningful)"; fi
if grep -qE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$WORK/raw.out"; then ok "raw stdout carries timestamped tracing lines"; else bad "raw stdout missing tracing lines"; fi
if jq empty "$WORK/raw.out" >/dev/null 2>&1; then bad "raw stdout is already valid JSON (nothing to recover)"; else ok "raw stdout is NOT valid JSON as-is (jq would fail)"; fi

scenario "An unconfigured runner with ambient RUST_LOG still yields parseable JSON"
# Given the same fresh, unconfigured HOME and RUST_LOG=debug
# When the validate-skills action entrypoint runs over the skills directory
run_entrypoint "$FRESH" "$SKILL_DIR/"
# Then it exits cleanly and recovers a valid JSON array with summary outputs.
[ "$EXIT" -eq 0 ] && ok "entrypoint exits 0" || bad "entrypoint exit=$EXIT (stderr: $(tail -1 "$RT/stderr.log"))"
then_ "recovered document is valid JSON" jq empty "$RT/skrills-validate.json"
then_ "recovered JSON is an array" jq -e 'type == "array"' "$RT/skrills-validate.json"
then_ "GITHUB_OUTPUT records total=" grep -q '^total=' "$RT/gh.out"
then_ "GITHUB_OUTPUT records errors=" grep -q '^errors=' "$RT/gh.out"
then_ "GITHUB_OUTPUT records warnings=" grep -q '^warnings=' "$RT/gh.out"

scenario "A configured runner (no banner) still yields parseable JSON"
# Given a HOME where skrills IS configured (no first-run banner path)
CONF="$WORK/conf-home"; mkdir -p "$CONF/.codex/bin"
printf '[mcp_servers.skrills]\ncommand = "skrills"\n' >"$CONF/.codex/config.toml"
ln -sf "$REPO_ROOT/$BIN_PATH" "$CONF/.codex/bin/skrills"
# When the entrypoint runs
run_entrypoint "$CONF" "$SKILL_DIR/"
# Then JSON is still clean (proves recovery is not coupled to the banner).
[ "$EXIT" -eq 0 ] && ok "entrypoint exits 0" || bad "entrypoint exit=$EXIT"
then_ "recovered document is valid JSON" jq empty "$RT/skrills-validate.json"

scenario "A missing skills directory degrades gracefully (no crash)"
# Given INPUT_PATH points at a directory that does not exist
run_entrypoint "$FRESH" "does-not-exist-$RANDOM/"
# Then the action exits 0 and reports zero skills rather than failing the build.
[ "$EXIT" -eq 0 ] && ok "entrypoint exits 0 on missing dir" || bad "entrypoint exit=$EXIT on missing dir"
then_ "reports total=0 for a missing dir" grep -q '^total=0' "$RT/gh.out"

# ===========================================================================
feature "install.sh selects the tarball, never the .sha256 checksum sidecar"
# Releases ship <target>.tar.gz alongside <target>.tar.gz.sha256. Picking the
# sidecar feeds a 106-byte text file to tar ("not in gzip format"). We source
# the REAL selection function and assert across jq and the awk fallback.
# shellcheck source=/dev/null
source <(sed -n '/^fail()/,/^# --- main/p' scripts/install.sh | sed '$d')

SIDECAR_FIRST='{ "assets": [
  { "name": "skrills-x86_64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://example.com/x.tar.gz.sha256" },
  { "name": "skrills-x86_64-unknown-linux-gnu.tar.gz",        "browser_download_url": "https://example.com/x.tar.gz" } ] }'

for impl in jq awk; do
  scenario "asset selection via $impl skips a sidecar listed first"
  if [ "$impl" = awk ]; then export SKRILLS_FORCE_NO_JQ=1; else unset SKRILLS_FORCE_NO_JQ; fi
  got="$(SELECT_ASSET_FROM_JSON "$SIDECAR_FIRST" "x86_64-unknown-linux-gnu")"
  [ "$got" = "https://example.com/x.tar.gz" ] \
    && ok "[$impl] chose the .tar.gz tarball" \
    || bad "[$impl] chose '$got' (expected the .tar.gz tarball)"
done
unset SKRILLS_FORCE_NO_JQ

# ---- summary ---------------------------------------------------------------
printf "\n========================================\n"
printf "Dogfood contracts: ${GREEN}%d passed${NC}, ${RED}%d failed${NC}\n" "$PASS" "$FAIL"
printf "========================================\n"
[ "$FAIL" -eq 0 ] || exit 1
exit 0
