#!/usr/bin/env sh
# Install skrills and wire it into Claude Code (uv-style installer).
# Usage:
#   curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh
# Env overrides:
#   SKRILLS_GH_REPO   owner/repo (default: athola/skrills)
#   SKRILLS_VERSION   release tag without leading v (default: latest)
#   SKRILLS_BIN_DIR   install directory (default: $HOME/.skrills/bin)
#   SKRILLS_BIN_NAME  binary name (default: skrills)
#   SKRILLS_TARGET    explicit target triple override
#   SKRILLS_SKIP_PATH_MESSAGE  set to 1 to silence PATH reminder
#   SKRILLS_NO_HOOK   set to 1 to skip hook/MCP registration
#   SKRILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
#   SKRILLS_SKIP_CHECKSUM  set to 1 to install without verifying the release
#                     checksum. Only for a host with no sha256 tool or a
#                     release whose .sha256 sidecar is missing.
set -eu
# dash (sh) on some systems doesn't support pipefail; guard it.
if (set -o | grep -q pipefail 2>/dev/null); then
  set -o pipefail
fi

# --- helpers ---------------------------------------------------------------
fail() { echo "install error: $*" >&2; exit 1; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"; }

OS()
{
  case "$(uname -s)" in
    Linux) echo linux ;;
    Darwin) echo macos ;;
    *) fail "unsupported OS: $(uname -s)" ;;
  esac
}

ARCH()
{
  case "$(uname -m)" in
    x86_64|amd64) echo x86_64 ;;
    aarch64|arm64) echo aarch64 ;;
    *) fail "unsupported arch: $(uname -m)" ;;
  esac
}

TARGET()
{
  if [ -n "${SKRILLS_TARGET:-}" ]; then
    echo "$SKRILLS_TARGET"; return
  fi
  os="$(OS)"; arch="$(ARCH)"
  case "$os" in
    linux)
      echo "${arch}-unknown-linux-gnu" ;;
    macos)
      echo "${arch}-apple-darwin" ;;
    *) fail "unsupported os: $os" ;;
  esac
}

REPO()
{
  echo "${SKRILLS_GH_REPO:-athola/skrills}";
}

BIN_NAME()
{
  echo "${SKRILLS_BIN_NAME:-skrills}";
}

API_URL()
{
  repo="$(REPO)"
  if [ -n "${SKRILLS_VERSION:-}" ]; then
    echo "https://api.github.com/repos/${repo}/releases/tags/v${SKRILLS_VERSION}";
  else
    echo "https://api.github.com/repos/${repo}/releases/latest";
  fi
}

# Pick download URL by matching target in asset name
# jq implementation of asset selection. $1=release JSON, $2=target triple.
_SELECT_ASSET_JQ()
{
  # `printf '%s\n'`, not `echo`: under dash and macOS /bin/sh, `echo` expands
  # backslash escapes, so a release note containing \n turns the JSON body into
  # a literal newline and jq rejects the whole document.
  #
  # `first(...)` rather than `| head -n1`: under pipefail, head closing the pipe
  # early makes jq exit 141 and takes the installer down with it.
  #
  # `.assets // []`: a draft tag or an API error document has no .assets, and
  # `.assets[]` then errors "Cannot iterate over null" and exits 5, so the
  # installer died before reaching its own "no release asset found" message.
  printf '%s\n' "$1" | jq -r --arg target "$2" \
    'first((.assets // [])[] | select(.name | contains($target)) | select(.name | endswith(".tar.gz")) | .browser_download_url) // empty'
}

# Pure-POSIX awk fallback for asset selection. $1=release JSON, $2=target.
# The name must END in .tar.gz: a release carries the target triple in the
# tarball, in the "skrills-<target>.sha256" checksum sidecar and in the
# Windows .zip, so matching on the triple alone picks whichever GitHub lists
# first. Anchor on the closing quote (.tar.gz") so only the tarball matches.
_SELECT_ASSET_AWK()
{
  # `tr '{' '\n'` first: api.github.com pretty-prints today, but a minifying
  # proxy or a GitHub Enterprise host can return one line, and then a single
  # awk record holds every asset. One record per asset keeps each name with
  # its own url.
  #
  # The url is captured and printed at END rather than printed with `exit`:
  # exiting closes the pipe while printf is still writing, and under pipefail
  # that EPIPE fails the whole selection on a release with many assets.
  printf '%s\n' "$1" | tr '{' '\n' | awk -v target="$2" '
    /"name":/ { found = (index($0, target) && $0 ~ /\.tar\.gz"/) }
    found && url == "" && /"browser_download_url":/ {
      line = $0
      sub(/.*"browser_download_url": *"/, "", line)
      sub(/".*/, "", line)
      url = line
    }
    END { if (url != "") print url }
  '
}

# Choose the .tar.gz tarball URL for $2 from the release JSON in $1.
# Network-free and side-effect-free so the install test suite can exercise
# the real selection logic instead of a drifting copy. Prefers jq, falls
# back to awk for pure POSIX shells.
SELECT_ASSET_FROM_JSON()
{
  # SKRILLS_FORCE_NO_JQ=1 forces the awk path so tests can cover it even
  # when jq is installed.
  if [ "${SKRILLS_FORCE_NO_JQ:-0}" != 1 ] && command -v jq >/dev/null 2>&1; then
    _SELECT_ASSET_JQ "$1" "$2"
  else
    _SELECT_ASSET_AWK "$1" "$2"
  fi
}

SELECT_ASSET_URL()
{
  url_json="$(API_URL)"
  need_cmd curl
  release_json=$(curl -fsSL "$url_json") || fail "failed to fetch release metadata from $url_json"
  # Match the .tar.gz tarball, not the "skrills-<target>.sha256" checksum
  # sidecar: both carry the target triple, and GitHub lists the .sha256 first
  # because asset names sort alphabetically, so a match on the triple alone
  # hands tar a 102-byte text file ("not in gzip format").
  SELECT_ASSET_FROM_JSON "$release_json" "$(TARGET)"
}

# Print the sha256 digest of file $1. GNU coreutils ships sha256sum, macOS
# ships shasum. SKRILLS_FORCE_SHASUM=1 forces the shasum path so the test
# suite can cover it on a host that has both.
SHA256_OF()
{
  if [ "${SKRILLS_FORCE_SHASUM:-0}" != 1 ] && command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Verify archive $1 against the release checksum sidecar at URL $2.
# A release ships "skrills-<target>.sha256" beside the tarball, holding one
# sha256sum line: "<64 hex>  skrills-<target>.tar.gz". Only the digest is
# needed, because the archive is saved under a temporary name here.
VERIFY_CHECKSUM()
{
  archive="$1"
  checksum_url="$2"
  if [ "${SKRILLS_SKIP_CHECKSUM:-0}" = 1 ]; then
    echo "Skipping checksum verification (SKRILLS_SKIP_CHECKSUM=1)" >&2
    return
  fi
  need_cmd curl
  sidecar="${archive}.sha256"
  curl -fsSL "$checksum_url" -o "$sidecar" \
    || fail "no checksum sidecar at $checksum_url; set SKRILLS_SKIP_CHECKSUM=1 to install without verification"
  expected="$(cut -d' ' -f1 <"$sidecar")"
  case "${#expected}" in
    64) ;;
    *) fail "$checksum_url is not a sha256 digest line; set SKRILLS_SKIP_CHECKSUM=1 to install without verification" ;;
  esac
  actual="$(SHA256_OF "$archive")" || actual=""
  case "${#actual}" in
    64) ;;
    *) fail "unable to compute a sha256 digest of $archive; install sha256sum or shasum, or set SKRILLS_SKIP_CHECKSUM=1" ;;
  esac
  if [ "$expected" != "$actual" ]; then
    fail "checksum mismatch for $archive: sidecar says $expected, download hashes to $actual"
  fi
  echo "Checksum verified ($expected)"
}

DOWNLOAD_AND_EXTRACT()
{
  download_url="$1"
  bin_dir="$2"
  bin_name="$3"
  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM
  archive="$tmpdir/pkg.tar.gz"
  need_cmd curl
  curl -fL "$download_url" -o "$archive" || fail "download failed: $download_url"
  # Verify before extracting: tar and the binary that follows must never see
  # bytes the release did not sign off on. The sidecar sits beside the tarball
  # under the same name with .tar.gz replaced by .sha256.
  VERIFY_CHECKSUM "$archive" "${download_url%.tar.gz}.sha256"
  mkdir -p "$tmpdir/out"
  tar -xzf "$archive" -C "$tmpdir/out" || fail "unable to unpack archive"
  mkdir -p "$bin_dir"
  if [ -f "$tmpdir/out/$bin_name" ]; then
    mv "$tmpdir/out/$bin_name" "$bin_dir/$bin_name"
  else
    # try to find binary inside
    _found_all=$(find "$tmpdir/out" -type f -name "$bin_name")
    found=$(echo "$_found_all" | head -n1)
    [ -n "$found" ] || fail "binary $bin_name not found in archive"
    mv "$found" "$bin_dir/$bin_name"
  fi
  chmod +x "$bin_dir/$bin_name"
  echo "Installed $bin_name to $bin_dir"
}

install_hook_and_mcp()
{
  if [ "${SKRILLS_NO_HOOK:-0}" = 1 ]; then
    echo "Skipping hook/MCP registration (SKRILLS_NO_HOOK=1)"
    return
  fi
  if [ ! -x "$bin_dir/$bin_name" ]; then
    echo "Warning: binary not found at $bin_dir/$bin_name; skipping setup." >&2
    return
  fi
  # Use the installed binary's setup command
  echo "Running skrills setup..."
  if [ "${SKRILLS_UNIVERSAL:-0}" != "0" ]; then
    "$bin_dir/$bin_name" setup --yes --universal
  else
    "$bin_dir/$bin_name" setup --yes
  fi
}

ensure_path_hint()
{
  [ "${SKRILLS_SKIP_PATH_MESSAGE:-0}" = 1 ] && return
  case ":$PATH:" in
    *:"${1}":*) ;; # already in PATH
    *) echo "Add $1 to your PATH (e.g., export PATH=\"$1:\$PATH\")" ;; esac
}

# --- main ------------------------------------------------------------------
bin_name="$(BIN_NAME)"
bin_dir="${SKRILLS_BIN_DIR:-$HOME/.skrills/bin}"
asset_url=$(SELECT_ASSET_URL)
[ -n "$asset_url" ] || fail "no release asset found matching target $(TARGET); check releases or specify SKRILLS_TARGET/SKRILLS_GH_REPO"
DOWNLOAD_AND_EXTRACT "$asset_url" "$bin_dir" "$bin_name"
ensure_path_hint "$bin_dir"
install_hook_and_mcp
