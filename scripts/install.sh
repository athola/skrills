#!/usr/bin/env sh
# Install skrills and wire it into Codex (uv-style installer).
# Usage:
#   curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh
#   ./scripts/install.sh [--local]
# Env overrides:
#   SKRILLS_GH_REPO   owner/repo (default: athola/skrills)
#   SKRILLS_VERSION   release tag without leading v (default: latest)
#   SKRILLS_BIN_DIR   install directory (default: $HOME/.codex/bin)
#   SKRILLS_BIN_NAME  binary name (default: skrills)
#   SKRILLS_TARGET    explicit target triple override
#   SKRILLS_SKIP_PATH_MESSAGE  set to 1 to silence PATH reminder
#   SKRILLS_NO_HOOK   set to 1 to skip hook/MCP registration
#   SKRILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
#   SKRILLS_MIRROR_SOURCE  source directory for mirroring skills (default: ~/.claude)
# Flags:
#   --local  Build from the current checkout with cargo and install that binary
set -eu
# Some /bin/sh variants (dash/busybox) lack pipefail; try but ignore if unsupported.
(set -o pipefail 2>/dev/null) || true

# --- helpers ---------------------------------------------------------------
fail() { echo "install error: $*" >&2; exit 1; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"; }

clean_legacy_codex_config()
{
  [ -f "$CONFIG_TOML" ] || return
  tmp=$(mktemp)
  awk '
    BEGIN { skip = 0 }
    /^\[mcp_servers\."codex-(mcp-)?skills"\]/ { skip = 1; next }
    /^\[/ { if (skip) skip = 0 }
    { if (!skip) print }
  ' "$CONFIG_TOML" > "$tmp" && mv "$tmp" "$CONFIG_TOML" && \
    echo "Removed legacy codex-skills entry from $CONFIG_TOML" || rm -f "$tmp"
}

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
SELECT_ASSET_URL()
{
  url_json="$(API_URL)"
  need_cmd curl
  release_json=$(curl -fsSL -H "Accept: application/vnd.github+json" "$url_json") \
    || fail "failed to fetch release metadata from $url_json"
  target="$(TARGET)"
  if command -v jq >/dev/null 2>&1; then
    asset_url=$(printf '%s' "$release_json" \
      | jq -er --arg target "$target" '
          .assets[]? 
          | select(
              (.name | contains($target))
              and ((.name | endswith(".tar.gz")) or (.name | endswith(".zip")))
            )
          | .browser_download_url
        ' 2>/dev/null \
      | head -n1 || true)
    if [ -n "$asset_url" ]; then
      echo "$asset_url"
      return
    fi
    echo "Warning: jq failed to extract asset URL, falling back to awk parser." >&2
  fi

  # jq not available: fallback to a simple awk-based extractor (no Python dependency).
  # This is a minimal parser that looks for an asset object containing the target
  # and then grabs its browser_download_url value.
  printf '%s' "$release_json" \
    | tr -d '\n' \
    | awk -v tgt="$target" '{
        n = split($0, parts, /"assets":\[/);
        if (n < 2) exit;
        split(parts[2], assets, /\}\s*,\s*\{/);
        for (i = 1; i <= length(assets); i++) {
          blk = assets[i];
          if (index(blk, tgt)) {
            if (match(blk, /"name":"([^"]+\.(tar\.gz|zip))"/, n)) {
              if (match(blk, /"browser_download_url":"([^"]+)"/, m)) {
                gsub(/\\u0026/, "\\&", m[1]); # decode encoded ampersands if present
                print m[1];
                exit;
              }
            }
          }
        }
      }'
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
  if ! tar -xzf "$archive" -C "$tmpdir/out"; then
    echo "Warning: unable to unpack archive from $download_url; falling back to cargo build." >&2
    return 1
  fi
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

BUILD_FROM_SOURCE()
{
  bin_dir="$1"
  bin_name="$2"
  need_cmd cargo
  repo="$(REPO)"
  tmpdir=$(mktemp -d)
  trap 'rm -rf "$tmpdir"' EXIT INT TERM
  tag_arg=""
  if [ -n "${SKRILLS_VERSION:-}" ]; then
    tag_arg="--tag v${SKRILLS_VERSION}"
  fi
  echo "No release asset available; building from source via cargo install..."
  export CARGO_HOME="$tmpdir/cargo-home"
  export CARGO_TARGET_DIR="$tmpdir/target"
  # Install into a temp root to avoid polluting user cargo/bin, then copy.
  # Be explicit about features so subagents ship on by default even if the
  # workspace default changes in the future.
  cargo install --git "https://github.com/${repo}.git" $tag_arg --bin "skrills" --features "subagents" --root "$tmpdir/cargo-root" --locked --force
  built_bin="$tmpdir/cargo-root/bin/skrills"
  [ -x "$built_bin" ] || fail "cargo install did not produce skrills"
  mkdir -p "$bin_dir"
  mv "$built_bin" "$bin_dir/$bin_name"
  chmod +x "$bin_dir/$bin_name"
  echo "Built from source and installed $bin_name to $bin_dir"
}

install_hook_and_mcp()
{
  if [ "${SKRILLS_NO_HOOK:-0}" = 1 ]; then
    echo "Skipping hook/MCP registration (SKRILLS_NO_HOOK=1)"
    return
  fi
  if [ ! -x "$bin_dir/$bin_name" ]; then
    echo "Warning: binary not found at $bin_dir/$bin_name; skipping hook." >&2
    return
  fi

  # Determine base directory based on client if not explicitly set
  local base_dir="${SKRILLS_BASE_DIR:-}"
  if [ -z "$base_dir" ]; then
    if [ "${SKRILLS_CLIENT}" = "claude" ]; then
      base_dir="$HOME/.claude"
    else
      base_dir="$HOME/.codex"
    fi
  fi

  BIN_PATH="$bin_dir/$bin_name" SKRILLS_UNIVERSAL="${SKRILLS_UNIVERSAL:-0}" \
    SKRILLS_CLIENT="${SKRILLS_CLIENT}" SKRILLS_BASE_DIR="$base_dir" \
    SKRILLS_MIRROR_SOURCE="${SKRILLS_MIRROR_SOURCE:-}" \
    "$PWD/scripts/install-skrills.sh"
}


ensure_path_hint()
{
  [ "${SKRILLS_SKIP_PATH_MESSAGE:-0}" = 1 ] && return
  case ":$PATH:" in
    *:"${1}":*) ;; # already in PATH
    *) echo "Add $1 to your PATH (e.g., export PATH=\"$1:\$PATH\")" ;; esac
}

usage()
{
  cat <<'USAGE'
Install skrills and wire it into Codex/Claude.
Default behavior:
- Codex: MCP-first (registers skrills MCP server in ~/.codex/config.toml and updates AGENTS.md).
- Claude: hook-first (writes `~/.claude/hooks/prompt.on_user_prompt_submit` and registers MCP server).

Options:
  --local          Build from the current checkout with cargo and install that binary
  --install-path PATH  Override install directory (sets SKRILLS_BIN_DIR)
  --client codex|claude  Target client for hook/MCP paths (default: codex)
  --base-dir PATH   Override client base dir (default: ~/.codex or ~/.claude per client)
  -h, --help       Show this help

Environment:
  SKRILLS_BIN_DIR   install directory (default: $HOME/.codex/bin for codex, $HOME/.claude/bin for claude)
  SKRILLS_BIN_NAME  binary name (default: skrills)
  SKRILLS_TARGET    optional cargo --target triple for builds
  SKRILLS_GH_REPO   owner/repo for release download (default: athola/skrills)
  SKRILLS_VERSION   release tag (no leading v) if pinning a specific version
  SKRILLS_NO_HOOK   set to 1 to skip hook/MCP registration
  SKRILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
  SKRILLS_MIRROR_SOURCE  source directory for mirroring skills (default: $HOME/.claude)
                         automatically skipped if same as install location to prevent redundancy
  SKRILLS_SKIP_PATH_MESSAGE set to 1 to silence PATH hint
USAGE
}

parse_args()
{
  LOCAL_BUILD=0
  SKRILLS_CLIENT="codex"
  while [ $# -gt 0 ]; do
    case "$1" in
      --install-path)
        shift
        [ $# -gt 0 ] || fail "--install-path requires a path"
        SKRILLS_BIN_DIR="$1"
        ;;
      --install-path=*)
        SKRILLS_BIN_DIR="${1#*=}"
        ;;
      --client)
        shift
        [ $# -gt 0 ] || fail "--client requires codex or claude"
        SKRILLS_CLIENT="$1"
        ;;
      --client=*)
        SKRILLS_CLIENT="${1#*=}"
        ;;
      --base-dir)
        shift
        [ $# -gt 0 ] || fail "--base-dir requires a path"
        SKRILLS_BASE_DIR="$1"
        ;;
      --base-dir=*)
        SKRILLS_BASE_DIR="${1#*=}"
        ;;
      --local) LOCAL_BUILD=1 ;;
      -h|--help) usage; exit 0 ;;
      *) fail "unknown option: $1" ;;
    esac
    shift
  done
}

BUILD_LOCAL()
{
  bin_dir="$1"
  bin_name="$2"
  need_cmd cargo

  build_args="--release"
  build_target_dir="target/release"
  if [ -n "${SKRILLS_TARGET:-}" ]; then
    build_args="$build_args --target ${SKRILLS_TARGET}"
    build_target_dir="target/${SKRILLS_TARGET}/release"
  fi

  echo "Building locally with: cargo build $build_args"
  cargo build $build_args

  built_bin="$build_target_dir/$bin_name"
  [ -x "$built_bin" ] || fail "local build did not produce $built_bin"

  mkdir -p "$bin_dir"
  install -m 0755 "$built_bin" "$bin_dir/$bin_name"
  echo "Installed $bin_name from local build to $bin_dir"
}

# --- main ------------------------------------------------------------------
parse_args "$@"

# Auto-detect client if not explicitly set via env/flag
if [ -z "${SKRILLS_CLIENT:-}" ]; then
  SKRILLS_CLIENT="auto"
fi

detect_client_from_base() {
  local base_hint="$1"
  shopt -s nocasematch 2>/dev/null || true
  case "$base_hint" in
    *".claude"*|*"/claude"*|*"claude"*) echo "claude"; return ;; 
    *".codex"*|*"/codex"*|*"codex"*) echo "codex"; return ;;
  esac
  echo "" # unknown
}

probe_signature_files() {
  local base="$1"
  # Claude markers
  if [ -d "$base/hooks" ] && ls "$base/hooks" 2>/dev/null | grep -qi "prompt.on_user_prompt_submit"; then
    echo "claude"; return
  fi
  if [ -f "$base/config.toml" ] && grep -qi "claude" "$base/config.toml"; then
    echo "claude"; return
  fi
  if [ -f "$base/mcp_servers.json" ] && grep -qi "claude" "$base/mcp_servers.json"; then
    echo "claude"; return
  fi
  # Codex markers
  if [ -d "$base/hooks/codex" ]; then
    echo "codex"; return
  fi
  if [ -f "$base/config.toml" ] && grep -qi "codex" "$base/config.toml"; then
    echo "codex"; return
  fi
  if [ -f "$base/mcp_servers.json" ] && grep -qi "codex" "$base/mcp_servers.json"; then
    echo "codex"; return
  fi
  echo "" # unknown
}

if [ "$SKRILLS_CLIENT" = "auto" ]; then
  base_hint="${SKRILLS_BASE_DIR:-${SKRILLS_BIN_DIR:-$HOME/.codex}}"
  candidate=$(detect_client_from_base "$base_hint")
  if [ -z "$candidate" ]; then
    candidate=$(probe_signature_files "$base_hint")
  fi
  if [ -z "$candidate" ]; then
    SKRILLS_CLIENT="codex" # safe default: MCP-first
  else
    SKRILLS_CLIENT="$candidate"
  fi
fi

bin_name="$(BIN_NAME)"

# Set default bin_dir and config path based on client
if [ "$SKRILLS_CLIENT" = "claude" ]; then
  bin_dir="${SKRILLS_BIN_DIR:-$HOME/.claude/bin}"
  CONFIG_TOML="$HOME/.claude/config.toml"
else
  bin_dir="${SKRILLS_BIN_DIR:-$HOME/.codex/bin}"
  CONFIG_TOML="$HOME/.codex/config.toml"
fi

clean_legacy_codex_config

if [ "$LOCAL_BUILD" = 1 ]; then
  BUILD_LOCAL "$bin_dir" "$bin_name"
else
  asset_url=$(SELECT_ASSET_URL)
  if [ -n "$asset_url" ]; then
    if ! DOWNLOAD_AND_EXTRACT "$asset_url" "$bin_dir" "$bin_name"; then
      echo "Falling back to source build because binary extraction failed." >&2
      BUILD_FROM_SOURCE "$bin_dir" "$bin_name"
    fi
  else
    echo "Warning: no release asset found matching target $(TARGET) at $(API_URL)"
    BUILD_FROM_SOURCE "$bin_dir" "$bin_name"
  fi
fi

ensure_path_hint "$bin_dir"

# Also copy to ~/.cargo/bin for consistency with cargo install
cargo_bin_dir="$HOME/.cargo/bin"
if [ "$bin_dir" != "$cargo_bin_dir" ] && [ -d "$cargo_bin_dir" ]; then
  if cp "$bin_dir/$bin_name" "$cargo_bin_dir/$bin_name" 2>/dev/null; then
    chmod +x "$cargo_bin_dir/$bin_name"
    echo "Also installed $bin_name to $cargo_bin_dir"
  fi
fi

install_hook_and_mcp

# Optionally mirror Claude assets into Codex on install.
if [ "${SKRILLS_NO_MIRROR:-0}" != 1 ]; then
  if [ "${SKRILLS_CLIENT}" = "codex" ]; then
    if [ -d "$HOME/.claude" ]; then
      echo "Mirroring Claude skills/agents/commands into Codex..."
      if ! "$bin_dir/$bin_name" mirror >/dev/null 2>&1; then
        echo "Warning: skrills mirror failed; you can rerun manually with 'skrills mirror' once ~/.claude is available." >&2
      fi
    else
      echo "Skip mirror: ~/.claude not found. Run 'skrills mirror' after Claude is set up." >&2
    fi
  fi
fi
