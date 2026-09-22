#!/usr/bin/env bash
# Test suite for install.sh helper functions
# Run: ./scripts/test-install.sh
# Exit 0 on success, 1 on any failure

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_SCRIPT="$SCRIPT_DIR/install.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

PASSED=0
FAILED=0

# Test helper functions
pass() {
    echo -e "${GREEN}PASS${NC}: $1"
    PASSED=$((PASSED + 1))
}

test_fail() {
    echo -e "${RED}FAIL${NC}: $1"
    FAILED=$((FAILED + 1))
}

assert_eq() {
    local actual="$1"
    local expected="$2"
    local msg="$3"
    if [[ "$actual" == "$expected" ]]; then
        pass "$msg"
    else
        test_fail "$msg (expected '$expected', got '$actual')"
    fi
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local msg="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        pass "$msg"
    else
        test_fail "$msg (expected to contain '$needle')"
    fi
}

# Source just the helper functions from install.sh
# We extract them to avoid running the main script
extract_functions() {
    # Extract everything between first function and "# --- main"
    sed -n '/^fail()/,/^# --- main/p' "$INSTALL_SCRIPT" | sed '$d'
}

# Create a temp file with the functions
FUNC_FILE=$(mktemp)
trap 'rm -f "$FUNC_FILE"' EXIT

extract_functions > "$FUNC_FILE"
source "$FUNC_FILE"

echo "=== Testing install.sh helper functions ==="
echo ""

# Test 1: OS detection
echo "--- OS Detection Tests ---"
OS_RESULT=$(OS)
case "$(uname -s)" in
    Linux) assert_eq "$OS_RESULT" "linux" "OS detection on Linux" ;;
    Darwin) assert_eq "$OS_RESULT" "macos" "OS detection on macOS" ;;
    *) test_fail "Unknown OS: $(uname -s)" ;;
esac

# Test 2: ARCH detection
echo ""
echo "--- ARCH Detection Tests ---"
ARCH_RESULT=$(ARCH)
case "$(uname -m)" in
    x86_64|amd64) assert_eq "$ARCH_RESULT" "x86_64" "ARCH detection for x86_64" ;;
    aarch64|arm64) assert_eq "$ARCH_RESULT" "aarch64" "ARCH detection for aarch64" ;;
    *) test_fail "Unknown arch: $(uname -m)" ;;
esac

# Test 3: TARGET detection
echo ""
echo "--- TARGET Detection Tests ---"
TARGET_RESULT=$(TARGET)
assert_contains "$TARGET_RESULT" "$(ARCH)" "TARGET contains arch"
case "$(uname -s)" in
    Linux) assert_contains "$TARGET_RESULT" "linux" "TARGET contains linux" ;;
    Darwin) assert_contains "$TARGET_RESULT" "darwin" "TARGET contains darwin" ;;
esac

# Test 4: Default values
echo ""
echo "--- Default Value Tests ---"
assert_eq "$(REPO)" "athola/skrills" "Default REPO"
assert_eq "$(BIN_NAME)" "skrills" "Default BIN_NAME"

# Test 5: Environment variable overrides
echo ""
echo "--- Environment Override Tests ---"
OVERRIDE_REPO=$(SKRILLS_GH_REPO=custom/repo REPO)
assert_eq "$OVERRIDE_REPO" "custom/repo" "SKRILLS_GH_REPO override"
OVERRIDE_BIN=$(SKRILLS_BIN_NAME=custom-bin BIN_NAME)
assert_eq "$OVERRIDE_BIN" "custom-bin" "SKRILLS_BIN_NAME override"

# Test 6: TARGET override
echo ""
echo "--- TARGET Override Test ---"
OVERRIDE_TARGET=$(SKRILLS_TARGET=custom-target TARGET)
assert_eq "$OVERRIDE_TARGET" "custom-target" "SKRILLS_TARGET override"

# Test 7: API_URL construction
echo ""
echo "--- API_URL Tests ---"
API_LATEST=$(API_URL)
assert_contains "$API_LATEST" "releases/latest" "API_URL defaults to latest"
assert_contains "$API_LATEST" "athola/skrills" "API_URL contains repo"

API_VERSIONED=$(SKRILLS_VERSION=1.2.3 API_URL)
assert_contains "$API_VERSIONED" "releases/tags/v1.2.3" "API_URL with version"

# Test 8: asset selection (SELECT_ASSET_FROM_JSON) — exercises the REAL
# install.sh selection logic, not a copy. Each scenario runs through both the
# jq path and the awk fallback (SKRILLS_FORCE_NO_JQ=1) so neither drifts.
echo ""
echo "--- Asset Selection: happy path (tarball present) ---"
MOCK_RELEASE_JSON='{
  "assets": [
    {
      "name": "skrills-aarch64-apple-darwin.tar.gz",
      "browser_download_url": "https://example.com/darwin-arm64.tar.gz"
    },
    {
      "name": "skrills-x86_64-unknown-linux-gnu.tar.gz",
      "browser_download_url": "https://example.com/linux-x64.tar.gz"
    }
  ]
}'

for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_RELEASE_JSON" "x86_64-unknown-linux-gnu")
    assert_eq "$R" "https://example.com/linux-x64.tar.gz" "[$impl] selects linux tarball URL"
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_RELEASE_JSON" "aarch64-apple-darwin")
    assert_eq "$R" "https://example.com/darwin-arm64.tar.gz" "[$impl] selects darwin tarball URL"
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_RELEASE_JSON" "nonexistent-target")
    assert_eq "$R" "" "[$impl] returns empty for no match"
done
unset SKRILLS_FORCE_NO_JQ

# Regression: releases ship "skrills-<target>.sha256" beside
# "skrills-<target>.tar.gz", and GitHub lists the .sha256 first because asset
# names sort alphabetically. Both names carry the target triple, so matching
# on the triple alone grabs the 102-byte checksum and tar fails "not in gzip
# format". The ".tar.gz\"" anchor is what tells them apart.
# (commit a6419c2 for jq; the awk fallback fix follows here.)
echo ""
echo "--- Asset Selection: .sha256 sidecar listed FIRST (negative test) ---"
MOCK_SIDECAR_FIRST='{
  "assets": [
    {
      "name": "skrills-x86_64-unknown-linux-gnu.sha256",
      "browser_download_url": "https://example.com/skrills-x86_64-unknown-linux-gnu.sha256"
    },
    {
      "name": "skrills-x86_64-unknown-linux-gnu.tar.gz",
      "browser_download_url": "https://example.com/linux-x64.tar.gz"
    }
  ]
}'

for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_SIDECAR_FIRST" "x86_64-unknown-linux-gnu")
    assert_eq "$R" "https://example.com/linux-x64.tar.gz" "[$impl] skips .sha256 sidecar, picks tarball"
done
unset SKRILLS_FORCE_NO_JQ

# Edge case: only a checksum is published (tarball upload failed). We must
# return nothing rather than hand a .sha256 URL to tar.
echo ""
echo "--- Asset Selection: only a .sha256 sidecar exists (edge case) ---"
MOCK_ONLY_SIDECAR='{
  "assets": [
    {
      "name": "skrills-x86_64-unknown-linux-gnu.sha256",
      "browser_download_url": "https://example.com/skrills-x86_64-unknown-linux-gnu.sha256"
    }
  ]
}'
for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_ONLY_SIDECAR" "x86_64-unknown-linux-gnu")
    assert_eq "$R" "" "[$impl] returns empty when only a sidecar is published"
done
unset SKRILLS_FORCE_NO_JQ

# Regression: a draft tag or an API error document has no `.assets` at all.
# jq's `.assets[]` then errors "Cannot iterate over null" and exits 5, and
# `curl | sh` died on that instead of printing the actionable
# "no release asset found" message the installer has for this case.
echo ""
echo "--- Asset Selection: release document with no assets array ---"
MOCK_NO_ASSETS='{ "tag_name": "v9.9.9", "message": "Not Found" }'
for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    NO_ASSETS_RC=0
    R="$(SELECT_ASSET_FROM_JSON "$MOCK_NO_ASSETS" "x86_64-unknown-linux-gnu" 2>/dev/null)" \
        || NO_ASSETS_RC=$?
    assert_eq "$NO_ASSETS_RC" "0" "[$impl] exits 0 when the document has no assets array"
    assert_eq "$R" "" "[$impl] returns empty when the document has no assets array"
done
unset SKRILLS_FORCE_NO_JQ

# Regression: api.github.com pretty-prints today, but a minifying proxy or a
# GitHub Enterprise host can return the whole document on one line. The awk
# fallback's greedy gsub then kept the LAST url in the document, so a Linux
# host was handed the macOS build. Every other fixture here is multi-line, so
# the suite could not see it.
echo ""
echo "--- Asset Selection: minified single-line JSON ---"
MOCK_MINIFIED='{"assets":[{"name":"skrills-x86_64-unknown-linux-gnu.tar.gz","browser_download_url":"https://example.com/linux-x64.tar.gz"},{"name":"skrills-aarch64-apple-darwin.tar.gz","browser_download_url":"https://example.com/darwin-arm64.tar.gz"}]}'
for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_MINIFIED" "x86_64-unknown-linux-gnu")
    assert_eq "$R" "https://example.com/linux-x64.tar.gz" \
        "[$impl] picks the linux tarball from minified JSON"
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_MINIFIED" "aarch64-apple-darwin")
    assert_eq "$R" "https://example.com/darwin-arm64.tar.gz" \
        "[$impl] picks the darwin tarball from minified JSON"
done
unset SKRILLS_FORCE_NO_JQ

# Regression: `first(...)` instead of `| head -n1`. head closes the pipe as
# soon as it has a line, jq dies of SIGPIPE, and `set -o pipefail` turns that
# into exit 141 for the whole installer. The fixtures above are far too small
# to fill a pipe buffer, so only a large document exercises it.
echo ""
echo "--- Asset Selection: thousands of matching assets under pipefail ---"
build_many_assets() {
    local count="$1" i
    printf '{\n  "assets": [\n'
    for ((i = 1; i <= count; i++)); do
        if ((i > 1)); then printf ',\n'; fi
        printf '    { "name": "skrills-x86_64-unknown-linux-gnu.tar.gz",'
        printf ' "browser_download_url": "https://example.com/%05d.tar.gz" }' "$i"
    done
    printf '\n  ]\n}\n'
}
MOCK_MANY_ASSETS="$(build_many_assets 3000)"
for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    MANY_RC=0
    R="$(SELECT_ASSET_FROM_JSON "$MOCK_MANY_ASSETS" "x86_64-unknown-linux-gnu")" || MANY_RC=$?
    assert_eq "$MANY_RC" "0" "[$impl] exits 0 with 3000 matching assets"
    assert_eq "$R" "https://example.com/00001.tar.gz" "[$impl] returns the first matching asset"
done
unset SKRILLS_FORCE_NO_JQ

# Checksum verification. Releases ship "skrills-<target>.sha256" holding one
# sha256sum line, "<64 hex>  skrills-<target>.tar.gz". Served over file:// so
# the suite stays offline.
echo ""
echo "--- Checksum verification (VERIFY_CHECKSUM) ---"
CK_DIR="$(mktemp -d)"
trap 'rm -f "$FUNC_FILE"; rm -rf "$CK_DIR"' EXIT
printf 'payload\n' >"$CK_DIR/pkg.tar.gz"
CK_GOOD="d4e4877bac978b7952f0d544fc52ebff5411d351d129f1f056fa43f11da9af2b"
CK_BAD="0000000000000000000000000000000000000000000000000000000000000000"
printf '%s  skrills-x86_64-unknown-linux-gnu.tar.gz\n' "$CK_GOOD" >"$CK_DIR/good.sha256"
printf '%s  skrills-x86_64-unknown-linux-gnu.tar.gz\n' "$CK_BAD" >"$CK_DIR/bad.sha256"
printf '<html>404: Not Found</html>\n' >"$CK_DIR/html.sha256"

# VERIFY_CHECKSUM reports through fail(), which exits. Run it in a subshell so
# a negative case cannot take the rest of the suite down with it.
run_verify() {
    VERIFY_RC=0
    VERIFY_OUT="$( (VERIFY_CHECKSUM "$1" "$2") 2>&1 )" || VERIFY_RC=$?
}

run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/good.sha256"
assert_eq "$VERIFY_RC" "0" "a matching sidecar verifies"
assert_contains "$VERIFY_OUT" "$CK_GOOD" "a matching sidecar reports the digest"

run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/bad.sha256"
assert_eq "$VERIFY_RC" "1" "a mismatched sidecar fails"
assert_contains "$VERIFY_OUT" "checksum mismatch" "a mismatch names the failure"

run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/absent.sha256"
assert_eq "$VERIFY_RC" "1" "a missing sidecar fails"
assert_contains "$VERIFY_OUT" "SKRILLS_SKIP_CHECKSUM" "a missing sidecar names the opt-out"

run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/html.sha256"
assert_eq "$VERIFY_RC" "1" "a sidecar that is not a digest line fails"
assert_contains "$VERIFY_OUT" "not a sha256 digest" "a non-digest sidecar names the reason"

export SKRILLS_SKIP_CHECKSUM=1
run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/absent.sha256"
assert_eq "$VERIFY_RC" "0" "SKRILLS_SKIP_CHECKSUM=1 installs without a sidecar"
unset SKRILLS_SKIP_CHECKSUM

export SKRILLS_FORCE_SHASUM=1
run_verify "$CK_DIR/pkg.tar.gz" "file://$CK_DIR/good.sha256"
assert_eq "$VERIFY_RC" "0" "[shasum] shasum -a 256 produces the same digest as sha256sum"

# A digest tool that produces nothing must not read as a match against an
# empty expected value.
STUB_BIN="$CK_DIR/stub-bin"
mkdir -p "$STUB_BIN"
printf '#!/bin/sh\nexit 1\n' >"$STUB_BIN/shasum"
chmod +x "$STUB_BIN/shasum"
VERIFY_RC=0
VERIFY_OUT="$( (PATH="$STUB_BIN:$PATH" VERIFY_CHECKSUM "$CK_DIR/pkg.tar.gz" \
    "file://$CK_DIR/good.sha256") 2>&1 )" || VERIFY_RC=$?
assert_eq "$VERIFY_RC" "1" "a digest tool that produces nothing fails"
assert_contains "$VERIFY_OUT" "unable to compute" "a failed digest names the reason"
unset SKRILLS_FORCE_SHASUM

# Summary
echo ""
echo "========================================"
echo "Results: $PASSED passed, $FAILED failed"
echo "========================================"

if [[ $FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
