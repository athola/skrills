#!/usr/bin/env sh
# Install codex-mcp-skills and wire it into Codex (uv-style installer).
# Usage:
#   curl -LsSf https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh | sh
# Env overrides:
#   CODEX_SKILLS_GH_REPO   owner/repo (default: athola/codex-mcp-skills)
#   CODEX_SKILLS_VERSION   release tag without leading v (default: latest)
#   CODEX_SKILLS_BIN_DIR   install directory (default: $HOME/.codex/bin)
#   CODEX_SKILLS_BIN_NAME  binary name (default: codex-mcp-skills)
#   CODEX_SKILLS_TARGET    explicit target triple override
#   CODEX_SKILLS_SKIP_PATH_MESSAGE  set to 1 to silence PATH reminder
#   CODEX_SKILLS_NO_HOOK   set to 1 to skip hook/MCP registration
#   CODEX_SKILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
set -euo pipefail

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
  if [ -n "${CODEX_SKILLS_TARGET:-}" ]; then
    echo "$CODEX_SKILLS_TARGET"; return
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
  echo "${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}";
}

BIN_NAME()
{
  echo "${CODEX_SKILLS_BIN_NAME:-codex-mcp-skills}";
}

API_URL()
{
  repo="$(REPO)"
  if [ -n "${CODEX_SKILLS_VERSION:-}" ]; then
    echo "https://api.github.com/repos/${repo}/releases/tags/v${CODEX_SKILLS_VERSION}";
  else
    echo "https://api.github.com/repos/${repo}/releases/latest";
  fi
}

# Pick download URL by matching target in asset name
SELECT_ASSET_URL()
{
  url_json="$(API_URL)"
  need_cmd curl
  release_json=$(curl -fsSL "$url_json") || fail "failed to fetch release metadata from $url_json"
  target="$(TARGET)"
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$target" <<'PY'
import json,sys,sys
j=json.loads(sys.stdin.read())
needle=sys.argv[1]
for asset in j.get("assets", []):
    name=asset.get("name","")
    if needle in name:
        print(asset.get("browser_download_url",""))
        sys.exit(0)
print("")
PY
  else
    need_cmd jq
    echo "$release_json" | jq -r --arg target "$target" '.assets[] | select(.name | contains($target)) | .browser_download_url' | head -n1
  fi
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
  mkdir -p "$tmpdir/out"
  tar -xzf "$archive" -C "$tmpdir/out" || fail "unable to unpack archive"
  mkdir -p "$bin_dir"
  if [ -f "$tmpdir/out/$bin_name" ]; then
    mv "$tmpdir/out/$bin_name" "$bin_dir/$bin_name"
  else
    # try to find binary inside
    found=$(find "$tmpdir/out" -type f -name "$bin_name" | head -n1)
    [ -n "$found" ] || fail "binary $bin_name not found in archive"
    mv "$found" "$bin_dir/$bin_name"
  fi
  chmod +x "$bin_dir/$bin_name"
  echo "Installed $bin_name to $bin_dir"
}

install_hook_and_mcp()
{
  if [ "${CODEX_SKILLS_NO_HOOK:-0}" = 1 ]; then
    echo "Skipping hook/MCP registration (CODEX_SKILLS_NO_HOOK=1)"
    return
  fi
  if [ ! -x "$bin_dir/$bin_name" ]; then
    echo "Warning: binary not found at $bin_dir/$bin_name; skipping hook." >&2
    return
  fi
  CODEX_SKILLS_BIN="$bin_dir/$bin_name" CODEX_SKILLS_UNIVERSAL="${CODEX_SKILLS_UNIVERSAL:-0}" \
    "$PWD/scripts/install-codex-skills.sh"
}

ensure_path_hint()
{
  [ "${CODEX_SKILLS_SKIP_PATH_MESSAGE:-0}" = 1 ] && return
  case ":$PATH:" in
    *:"${1}":*) ;; # already in PATH
    *) echo "Add $1 to your PATH (e.g., export PATH=\"$1:\$PATH\")" ;; esac
}

# --- main ------------------------------------------------------------------
bin_name="$(BIN_NAME)"
bin_dir="${CODEX_SKILLS_BIN_DIR:-$HOME/.codex/bin}"
asset_url=$(SELECT_ASSET_URL)
[ -n "$asset_url" ] || fail "no release asset found matching target $(TARGET); check releases or specify CODEX_SKILLS_TARGET/CODEX_SKILLS_GH_REPO"
DOWNLOAD_AND_EXTRACT "$asset_url" "$bin_dir" "$bin_name"
ensure_path_hint "$bin_dir"
install_hook_and_mcp
