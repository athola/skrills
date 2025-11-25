#!/usr/bin/env sh
# Install codex-mcp-skills and wire it into Codex (uv-style installer).
# Usage:
#   curl -LsSf https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh | sh
#   ./scripts/install.sh [--local]
# Env overrides:
#   CODEX_SKILLS_GH_REPO   owner/repo (default: athola/codex-mcp-skills)
#   CODEX_SKILLS_VERSION   release tag without leading v (default: latest)
#   CODEX_SKILLS_BIN_DIR   install directory (default: $HOME/.codex/bin)
#   CODEX_SKILLS_BIN_NAME  binary name (default: codex-mcp-skills)
#   CODEX_SKILLS_TARGET    explicit target triple override
#   CODEX_SKILLS_SKIP_PATH_MESSAGE  set to 1 to silence PATH reminder
#   CODEX_SKILLS_NO_HOOK   set to 1 to skip hook/MCP registration
#   CODEX_SKILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
# Flags:
#   --local  Build from the current checkout with cargo and install that binary
set -eu
# Some /bin/sh variants (dash/busybox) lack pipefail; try but ignore if unsupported.
(set -o pipefail 2>/dev/null) || true

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
  if [ -n "${CODEX_SKILLS_VERSION:-}" ]; then
    tag_arg="--tag v${CODEX_SKILLS_VERSION}"
  fi
  echo "No release asset available; building from source via cargo install..."
  export CARGO_HOME="$tmpdir/cargo-home"
  export CARGO_TARGET_DIR="$tmpdir/target"
  # Install into a temp root to avoid polluting user cargo/bin, then copy.
  cargo install --git "https://github.com/${repo}.git" $tag_arg --bin "codex-mcp-skills" --root "$tmpdir/cargo-root" --locked --force
  built_bin="$tmpdir/cargo-root/bin/codex-mcp-skills"
  [ -x "$built_bin" ] || fail "cargo install did not produce codex-mcp-skills"
  mkdir -p "$bin_dir"
  mv "$built_bin" "$bin_dir/$bin_name"
  chmod +x "$bin_dir/$bin_name"
  echo "Built from source and installed $bin_name to $bin_dir"
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
  BIN_PATH="$bin_dir/$bin_name" CODEX_SKILLS_UNIVERSAL="${CODEX_SKILLS_UNIVERSAL:-0}" \
    "$PWD/scripts/install-codex-skills.sh"
}

ensure_path_hint()
{
  [ "${CODEX_SKILLS_SKIP_PATH_MESSAGE:-0}" = 1 ] && return
  case ":$PATH:" in
    *:"${1}":*) ;; # already in PATH
    *) echo "Add $1 to your PATH (e.g., export PATH=\"$1:\$PATH\")" ;; esac
}

usage()
{
  cat <<'USAGE'
Install codex-mcp-skills and wire it into Codex.

Options:
  --local          Build from the current checkout with cargo and install that binary
  -h, --help       Show this help

Environment:
  CODEX_SKILLS_BIN_DIR   install directory (default: $HOME/.codex/bin)
  CODEX_SKILLS_BIN_NAME  binary name (default: codex-mcp-skills)
  CODEX_SKILLS_TARGET    optional cargo --target triple for builds
  CODEX_SKILLS_GH_REPO   owner/repo for release download (default: athola/codex-mcp-skills)
  CODEX_SKILLS_VERSION   release tag (no leading v) if pinning a specific version
  CODEX_SKILLS_NO_HOOK   set to 1 to skip hook/MCP registration
  CODEX_SKILLS_UNIVERSAL set to 1 to also sync ~/.agent/skills
  CODEX_SKILLS_SKIP_PATH_MESSAGE set to 1 to silence PATH hint
USAGE
}

parse_args()
{
  LOCAL_BUILD=0
  while [ $# -gt 0 ]; do
    case "$1" in
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
  if [ -n "${CODEX_SKILLS_TARGET:-}" ]; then
    build_args="$build_args --target ${CODEX_SKILLS_TARGET}"
    build_target_dir="target/${CODEX_SKILLS_TARGET}/release"
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
bin_name="$(BIN_NAME)"
bin_dir="${CODEX_SKILLS_BIN_DIR:-$HOME/.codex/bin}"

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
install_hook_and_mcp
