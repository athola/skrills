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

# Regression: releases ship a .tar.gz.sha256 checksum next to each tarball.
# Both names carry the target triple, and the sidecar name contains the
# substring ".tar.gz". When GitHub lists the sidecar FIRST, a substring
# match grabs the 106-byte checksum and tar fails "not in gzip format".
# (commit a6419c2 for jq; the awk fallback fix follows here.)
echo ""
echo "--- Asset Selection: .sha256 sidecar listed FIRST (negative test) ---"
MOCK_SIDECAR_FIRST='{
  "assets": [
    {
      "name": "skrills-x86_64-unknown-linux-gnu.tar.gz.sha256",
      "browser_download_url": "https://example.com/linux-x64.tar.gz.sha256"
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
      "name": "skrills-x86_64-unknown-linux-gnu.tar.gz.sha256",
      "browser_download_url": "https://example.com/linux-x64.tar.gz.sha256"
    }
  ]
}'
for impl in jq awk; do
    [[ "$impl" == awk ]] && export SKRILLS_FORCE_NO_JQ=1 || unset SKRILLS_FORCE_NO_JQ
    R=$(SELECT_ASSET_FROM_JSON "$MOCK_ONLY_SIDECAR" "x86_64-unknown-linux-gnu")
    assert_eq "$R" "" "[$impl] returns empty when only a sidecar is published"
done
unset SKRILLS_FORCE_NO_JQ

# Summary
echo ""
echo "========================================"
echo "Results: $PASSED passed, $FAILED failed"
echo "========================================"

if [[ $FAILED -gt 0 ]]; then
    exit 1
fi
exit 0
