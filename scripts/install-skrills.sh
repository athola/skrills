#!/usr/bin/env bash
# Install skrills MCP server registration for Codex or Claude.
# Skrills provides sync, validate, and analyze tools for skill management.
# Flags:
#   --universal        Also sync skills into ~/.agent/skills for cross-agent reuse.
#   --universal-only   Only perform the universal sync (no server install).
# Environment:
#   SKRILLS_MIRROR_SOURCE  Source directory for mirroring skills (default: ~/.claude).
#                          Automatically skipped if same as install location.
#   SKRILLS_HTTP           Bind address for HTTP transport (e.g., 127.0.0.1:3000).
#                          When set, installs systemd user service and configures MCP with URL.
set -euo pipefail

UNIVERSAL=0
UNIVERSAL_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=1 ;;
    --universal-only) UNIVERSAL=1; UNIVERSAL_ONLY=1 ;;
    *) echo "Unknown arg: $arg" >&2; exit 1 ;;
  esac
done
if [ "${SKRILLS_UNIVERSAL:-0}" != "0" ]; then
  UNIVERSAL=1
fi

# Preferred binary path (can be set by outer installer via BIN_PATH or SKRILLS_BIN).
BIN_PATH="${BIN_PATH:-${SKRILLS_BIN:-$HOME/.cargo/bin/skrills}}"
CLIENT="${SKRILLS_CLIENT:-auto}"
HTTP_ADDR="${SKRILLS_HTTP:-}"

detect_client_from_base() {
  local base_hint="$1"
  shopt -s nocasematch 2>/dev/null || true
  case "$base_hint" in
    *".claude"*|*"/claude"*|*"claude"*) echo "claude"; return ;;
    *".codex"*|*"/codex"*|*"codex"*) echo "codex"; return ;;
  esac
  echo ""
}

probe_signature_files() {
  local base="$1"
  if [ -d "$base/hooks" ] && ls "$base/hooks" 2>/dev/null | grep -qi "prompt.on_user_prompt_submit"; then
    echo "claude"; return
  fi
  if [ -f "$base/config.toml" ] && grep -qi "claude" "$base/config.toml"; then
    echo "claude"; return
  fi
  if [ -f "$base/mcp_servers.json" ] && grep -qi "claude" "$base/mcp_servers.json"; then
    echo "claude"; return
  fi
  if [ -d "$base/hooks/codex" ]; then
    echo "codex"; return
  fi
  if [ -f "$base/config.toml" ] && grep -qi "codex" "$base/config.toml"; then
    echo "codex"; return
  fi
  if [ -f "$base/mcp_servers.json" ] && grep -qi "codex" "$base/mcp_servers.json"; then
    echo "codex"; return
  fi
  echo ""
}

if [ "$CLIENT" = "auto" ]; then
  candidate=""
  if [ -n "${SKRILLS_BASE_DIR:-}" ]; then
    candidate=$(detect_client_from_base "$SKRILLS_BASE_DIR")
    if [ -z "$candidate" ]; then
      candidate=$(probe_signature_files "$SKRILLS_BASE_DIR")
    fi
  fi
  if [ -z "$candidate" ]; then
    if [ -n "${CLAUDE_CODE_SESSION:-}" ] || [ -n "${CLAUDE_CLI:-}" ] || [ -n "${__CLAUDE_MCP_SERVER:-}" ] || [ -n "${CLAUDE_CODE_ENTRYPOINT:-}" ]; then
      candidate="claude"
    fi
  fi
  if [ -z "$candidate" ]; then
    if [ -n "${CODEX_SESSION_ID:-}" ] || [ -n "${CODEX_CLI:-}" ] || [ -n "${CODEX_HOME:-}" ]; then
      candidate="codex"
    fi
  fi
  if [ -z "$candidate" ]; then
    candidate=$(detect_client_from_base "$BIN_PATH")
  fi
  if [ -z "$candidate" ]; then
    candidate=$(probe_signature_files "$HOME/.claude")
  fi
  if [ -z "$candidate" ]; then
    candidate=$(probe_signature_files "$HOME/.codex")
  fi
  if [ -z "$candidate" ]; then
    CLIENT="claude" # Default installer target when detection fails
  else
    CLIENT="$candidate"
  fi
fi

# Determine base dir (may inform client detection)
if [ -n "${SKRILLS_BASE_DIR:-}" ]; then
  BASE_DIR="$SKRILLS_BASE_DIR"
elif [ "$CLIENT" = "claude" ]; then
  BASE_DIR="$HOME/.claude"
else
  BASE_DIR="$HOME/.codex"
fi
mkdir -p "$BASE_DIR"

MCP_PATH="$BASE_DIR/.mcp.json"
CONFIG_TOML="$BASE_DIR/config.toml"
REPO_ROOT="$(cd "${0%/*}/.." && pwd)"

clean_invalid_claude_model() {
  # Remove invalid model settings from Claude config.toml (e.g., gpt-5.1-codex-max)
  if [ "$CLIENT" = "claude" ] && [ -f "$CONFIG_TOML" ]; then
    if grep -q '^model[[:space:]]*=[[:space:]]*"gpt-' "$CONFIG_TOML"; then
      tmp=$(mktemp)
      # Remove lines with invalid GPT model settings
      sed '/^model[[:space:]]*=[[:space:]]*"gpt-/d' "$CONFIG_TOML" > "$tmp"
      if mv "$tmp" "$CONFIG_TOML"; then
        echo "Removed invalid model setting from $CONFIG_TOML"
      else
        rm -f "$tmp"
      fi
    fi
  fi
}

clean_legacy_artifacts() {
  local removed=0
  local legacy_bins=(
    "$HOME/.codex/bin/codex-mcp-skills"
    "$HOME/.cargo/bin/codex-mcp-skills"
  )
  for bin in "${legacy_bins[@]}"; do
    if [ -e "$bin" ]; then
      rm -f "$bin" && echo "Removed legacy binary $bin" && removed=1
    fi
  done

  # Remove legacy mcp_servers.json (now using .mcp.json for Claude)
  if [ "$CLIENT" = "claude" ]; then
    local legacy_mcp="$BASE_DIR/mcp_servers.json"
    if [ -f "$legacy_mcp" ]; then
      rm -f "$legacy_mcp" && echo "Removed legacy $legacy_mcp" && removed=1
    fi
  fi

  # Clean legacy entries from .mcp.json
  if [ -f "$MCP_PATH" ]; then
    if command -v jq >/dev/null 2>&1; then
      tmp=$(mktemp)
      jq 'del(.mcpServers["codex-mcp-skills"])' "$MCP_PATH" > "$tmp" \
        && mv "$tmp" "$MCP_PATH" \
        && echo "Removed codex-mcp-skills from $MCP_PATH" \
        || rm -f "$tmp"
    else
      python3 - <<'PY' "$MCP_PATH"
import json, os, sys, tempfile
path = sys.argv[1]
try:
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
except Exception:
    sys.exit(0)
servers = data.get("mcpServers")
if isinstance(servers, dict) and servers.pop("codex-mcp-skills", None) is not None:
    tmp_fd, tmp_path = tempfile.mkstemp()
    with os.fdopen(tmp_fd, "w", encoding="utf-8") as out:
        json.dump(data, out, indent=2)
        out.write("\n")
    os.replace(tmp_path, path)
    print(f"Removed codex-mcp-skills from {path}")
PY
    fi
  fi

  # Remove MCP server entries from config.toml (now only in .mcp.json)
  if [ "$CLIENT" = "claude" ] && [ -f "$CONFIG_TOML" ]; then
    tmp=$(mktemp)
    awk '
      BEGIN { skip = 0 }
      /^\[mcp_servers\./ { skip = 1; next }
      /^\[/ { if (skip) skip = 0 }
      { if (!skip) print }
    ' "$CONFIG_TOML" > "$tmp" && mv "$tmp" "$CONFIG_TOML" && \
      echo "Removed all MCP server entries from $CONFIG_TOML (now in .mcp.json)" || rm -f "$tmp"
  fi

  # Clean legacy hook files (emit-autoload hooks no longer needed)
  local hook_dir="$BASE_DIR/hooks"
  if [ -d "$hook_dir" ]; then
    find "$hook_dir" -maxdepth 1 -type f -name '*codex-mcp-skills*' -exec rm -f {} \; 2>/dev/null
    # Remove legacy skrills emit-autoload hook if it exists
    local legacy_hook="$hook_dir/prompt.on_user_prompt_submit"
    if [ -f "$legacy_hook" ] && grep -q "emit-autoload" "$legacy_hook" 2>/dev/null; then
      rm -f "$legacy_hook" && echo "Removed legacy emit-autoload hook" && removed=1
    fi
  fi

  # Remove legacy autoload instructions from AGENTS.md
  local agents_md="$BASE_DIR/AGENTS.md"
  if [ -f "$agents_md" ] && grep -q '<!-- skrills-integration-start -->' "$agents_md" 2>/dev/null; then
    tmp=$(mktemp)
    awk '
      /<!-- skrills-integration-start -->/{ skip=1; next }
      /<!-- skrills-integration-end -->/{ skip=0; next }
      !skip { print }
    ' "$agents_md" > "$tmp" && mv "$tmp" "$agents_md"
    echo "Removed legacy autoload instructions from AGENTS.md"
    removed=1
  fi

  if [ "$removed" -eq 1 ]; then
    echo "Legacy artifacts cleaned."
  fi
}

install_subagents_config() {
  local config_path="$BASE_DIR/subagents.toml"
  local example="$REPO_ROOT/docs/config/subagents.example.toml"
  if [ -f "$config_path" ]; then
    echo "Subagents config already present at $config_path"
    return
  fi
  if [ -f "$example" ]; then
    mkdir -p "$(dirname "$config_path")"
    cp "$example" "$config_path"
    echo "Installed default subagents config to $config_path (execution_mode=cli, cli_binary=auto)"
  else
    echo "Warning: default subagents config example missing at $example" >&2
  fi
}

sync_universal() {
  local AGENT_SKILLS="${AGENT_SKILLS_DIR:-$HOME/.agent/skills}"
  local SKRILLS_DIR="${SKRILLS_DIR:-$BASE_DIR/skills}"
  local MIRROR_DIR="${CODEX_MIRROR_DIR:-$BASE_DIR/skills-mirror}"

  # Determine mirror source (default: ~/.claude)
  local MIRROR_SOURCE="${SKRILLS_MIRROR_SOURCE:-$HOME/.claude}"

  # Prevent mirroring if source is same as install location
  if [ "$(cd "$MIRROR_SOURCE" 2>/dev/null && pwd)" = "$(cd "$BASE_DIR" 2>/dev/null && pwd)" ]; then
    if [ "$BASE_DIR" = "$HOME/.claude" ]; then
      echo "Skipping mirror: install location and mirror source are both ~/.claude (redundant)."
      return 0
    else
      echo "Warning: mirror source ($MIRROR_SOURCE) is same as install location ($BASE_DIR)."
      echo "Reverting to default mirror source: ~/.claude"
      MIRROR_SOURCE="$HOME/.claude"
      # Check again if reverted source is same as install location
      if [ "$(cd "$MIRROR_SOURCE" 2>/dev/null && pwd)" = "$(cd "$BASE_DIR" 2>/dev/null && pwd)" ]; then
        echo "Skipping mirror: reverted source is also same as install location."
        return 0
      fi
    fi
  fi

  mkdir -p "$AGENT_SKILLS"
  echo "Universal sync: copying skills from $MIRROR_SOURCE into $AGENT_SKILLS"

  copy_tree() {
    local src="$1"
    [ -d "$src" ] || return 0
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --update "$src"/ "$AGENT_SKILLS"/
    else
      (cd "$src" && tar -cf - .) | (cd "$AGENT_SKILLS" && tar -xf -)
    fi
  }

  # Mirror from the specified source location (iteratively find skills)
  if [ -d "$MIRROR_SOURCE" ]; then
    echo "Mirroring skills from $MIRROR_SOURCE..."
    if [ -x "$BIN_PATH" ]; then
      # Use skrills sync command to mirror from source
      SKRILLS_MIRROR_SOURCE="$MIRROR_SOURCE" "$BIN_PATH" sync || echo "Warning: sync-from-source failed (continuing)."
    fi
    # Also copy from mirror directory if it exists and is different from source
    if [ -d "$MIRROR_DIR" ] && [ "$(cd "$MIRROR_DIR" 2>/dev/null && pwd)" != "$(cd "$MIRROR_SOURCE" 2>/dev/null && pwd)" ]; then
      copy_tree "$MIRROR_DIR"
    fi
  else
    echo "Warning: mirror source $MIRROR_SOURCE does not exist; skipping mirror."
  fi

  # Copy from local skills directory if it exists and is different from mirror source
  if [ -d "$SKRILLS_DIR" ] && [ "$(cd "$SKRILLS_DIR" 2>/dev/null && pwd)" != "$(cd "$MIRROR_SOURCE" 2>/dev/null && pwd)" ]; then
    copy_tree "$SKRILLS_DIR"
  fi

  echo "Universal sync complete."
}

warm_cache_snapshot() {
  if [ ! -x "$BIN_PATH" ]; then
    return
  fi
  echo "Priming skrills skill cache (first scan)..."
  if "$BIN_PATH" validate --errors-only >/dev/null 2>&1; then
    echo "Cache primed; subsequent startups will reuse the snapshot."
  else
    echo "Warning: cache warmup failed (continuing without primed cache)." >&2
  fi
}

enable_codex_skills_feature() {
  # Codex skills are behind the experimental `skills` feature flag in ~/.codex/config.toml.
  [ "$CLIENT" = "codex" ] || return 0

  mkdir -p "$(dirname "$CONFIG_TOML")"
  if [ ! -f "$CONFIG_TOML" ]; then
    return 0
  fi

  python3 - <<'PY' "$CONFIG_TOML"
import io, os, sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    lines = f.read().splitlines(True)

def is_header(line: str) -> bool:
    s = line.strip()
    return s.startswith("[") and s.endswith("]") and not s.startswith("[[")

def header_name(line: str) -> str:
    return line.strip()[1:-1]

out = []
in_features = False
skills_set = False

for i, line in enumerate(lines):
    if is_header(line):
        if in_features and not skills_set:
            out.append("skills = true\n")
            skills_set = True
        in_features = (header_name(line) == "features")
        out.append(line)
        continue
    if in_features:
        stripped = line.strip()
        if stripped.startswith("skills") and "=" in stripped:
            out.append("skills = true\n")
            skills_set = True
            continue
    out.append(line)

if not skills_set:
    if out and not out[-1].endswith("\n"):
        out[-1] = out[-1] + "\n"
    if out and not out[-1].endswith("\n\n"):
        out.append("\n")
    out.append("[features]\n")
    out.append("skills = true\n")

new = "".join(out)
with open(path, "w", encoding="utf-8") as f:
    f.write(new)
PY
  echo "Enabled Codex experimental skills feature in $CONFIG_TOML"
}

install_systemd_service() {
  # Install skrills as a systemd user service for HTTP transport mode
  local bind_addr="$1"
  local service_dir="$HOME/.config/systemd/user"
  local service_file="$service_dir/skrills.service"

  # Check if systemd user services are available
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "Warning: systemctl not found. Cannot install systemd service." >&2
    echo "You'll need to start skrills manually: $BIN_PATH serve --http $bind_addr" >&2
    return 1
  fi

  # Check if user session is available
  if ! systemctl --user status >/dev/null 2>&1; then
    echo "Warning: systemd user session not available (are you in an SSH session without lingering?)." >&2
    echo "Try: loginctl enable-linger $USER" >&2
    echo "You can start skrills manually: $BIN_PATH serve --http $bind_addr" >&2
    return 1
  fi

  mkdir -p "$service_dir"

  cat > "$service_file" <<EOF
[Unit]
Description=Skrills MCP Server (HTTP Transport)
Documentation=https://github.com/athola/skrills
After=network.target

[Service]
Type=simple
ExecStart=$BIN_PATH serve --http $bind_addr
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
EOF

  echo "Created systemd user service at $service_file"

  # Reload systemd and enable/start the service
  if systemctl --user daemon-reload; then
    if systemctl --user enable skrills.service; then
      if systemctl --user restart skrills.service; then
        echo "Started skrills HTTP server on $bind_addr"
        # Give it a moment to start
        sleep 1
        if systemctl --user is-active --quiet skrills.service; then
          echo "Service is running. Check status with: systemctl --user status skrills"
        else
          echo "Warning: Service may have failed to start. Check: journalctl --user -u skrills" >&2
        fi
      else
        echo "Warning: Failed to start skrills service. Check: journalctl --user -u skrills" >&2
        return 1
      fi
    else
      echo "Warning: Failed to enable skrills service" >&2
      return 1
    fi
  else
    echo "Warning: Failed to reload systemd daemon" >&2
    return 1
  fi
}

if [ "$UNIVERSAL_ONLY" -eq 1 ]; then
  sync_universal
  exit 0
fi

clean_legacy_artifacts
clean_invalid_claude_model
install_subagents_config

# Configure MCP server for Claude Code
if [ "$CLIENT" = "claude" ]; then
  if [ -n "$HTTP_ADDR" ]; then
    # HTTP transport mode: install systemd service and register with URL
    echo "Installing skrills with HTTP transport on $HTTP_ADDR..."
    install_systemd_service "$HTTP_ADDR" || true

    # Construct the MCP URL (assume /mcp endpoint)
    MCP_URL="http://${HTTP_ADDR}/mcp"

    # Register with Claude Code using HTTP transport
    if command -v claude >/dev/null 2>&1; then
      echo "Registering skrills MCP server (HTTP) with Claude Code..."
      # Capture both stdout and stderr for debugging
      mcp_add_output=$(claude mcp add --transport http --scope user skrills "$MCP_URL" 2>&1) && mcp_add_success=1 || mcp_add_success=0
      if [ "$mcp_add_success" = "1" ]; then
        echo "Successfully registered skrills MCP server (HTTP)"
      else
        echo "Warning: 'claude mcp add' failed. Falling back to manual configuration." >&2
        [ -n "$mcp_add_output" ] && echo "  Details: $mcp_add_output" >&2
        # Fallback: Create .mcp.json manually
        mkdir -p "$(dirname "$MCP_PATH")"
        if command -v jq >/dev/null 2>&1; then
          if [ -f "$MCP_PATH" ]; then
            tmp=$(mktemp)
            jq --arg url "$MCP_URL" '.mcpServers."skrills" = {"type": "http", "url": $url}' "$MCP_PATH" > "$tmp"
            mv "$tmp" "$MCP_PATH" && echo "Updated skrills MCP server (HTTP) in $MCP_PATH" || rm -f "$tmp"
          else
            tmp=$(mktemp)
            jq -n --arg url "$MCP_URL" '{mcpServers: {skrills: {type: "http", url: $url}}}' > "$tmp"
            mv "$tmp" "$MCP_PATH" && echo "Created $MCP_PATH with skrills MCP server (HTTP)" || rm -f "$tmp"
          fi
        else
          python3 - <<'PY' "$MCP_PATH" "$MCP_URL"
import json, os, sys
mcp_path, mcp_url = sys.argv[1], sys.argv[2]
data = {"mcpServers": {}}
if os.path.exists(mcp_path):
    try:
        with open(mcp_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception:
        pass
data.setdefault("mcpServers", {})["skrills"] = {"type": "http", "url": mcp_url}
with open(mcp_path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print(f"Configured skrills MCP server (HTTP) in {mcp_path}")
PY
        fi
      fi
    else
      echo "Warning: 'claude' command not found. Cannot register MCP server automatically." >&2
      echo "Add to your Claude Code settings:" >&2
      echo "  {\"mcpServers\": {\"skrills\": {\"type\": \"http\", \"url\": \"$MCP_URL\"}}}" >&2
    fi
  else
    # Default: stdio transport mode
    if command -v claude >/dev/null 2>&1; then
      echo "Registering skrills MCP server with Claude Code..."
      # Capture both stdout and stderr for debugging
      mcp_add_output=$(claude mcp add --transport stdio --scope user skrills -- "$BIN_PATH" serve 2>&1) && mcp_add_success=1 || mcp_add_success=0
      if [ "$mcp_add_success" = "1" ]; then
        echo "Successfully registered skrills MCP server"
      else
        echo "Warning: 'claude mcp add' failed. Falling back to manual configuration." >&2
        [ -n "$mcp_add_output" ] && echo "  Details: $mcp_add_output" >&2

        # Fallback: Create .mcp.json manually if claude command fails
        mkdir -p "$(dirname "$MCP_PATH")"
        if command -v jq >/dev/null 2>&1; then
          if [ -f "$MCP_PATH" ]; then
            tmp=$(mktemp)
            jq '.mcpServers."skrills" = {"type": "stdio", "command": "'"$BIN_PATH"'", "args": ["serve"]}' "$MCP_PATH" > "$tmp"
            if mv "$tmp" "$MCP_PATH"; then
              echo "Updated skrills MCP server in $MCP_PATH"
            else
              echo "Warning: unable to update $MCP_PATH (permission denied?)" >&2
              rm -f "$tmp"
            fi
          else
            tmp=$(mktemp)
            jq -n --arg bin "$BIN_PATH" '{mcpServers: {skrills: {type: "stdio", command: $bin, args: ["serve"]}}}' > "$tmp"
            if mv "$tmp" "$MCP_PATH"; then
              echo "Created $MCP_PATH with skrills MCP server"
            else
              echo "Warning: unable to create $MCP_PATH (permission denied?)" >&2
              rm -f "$tmp"
            fi
          fi
        else
          # Fallback to Python
          python3 - <<'PY' "$MCP_PATH" "$BIN_PATH"
import json, os, sys
mcp_path, bin_path = sys.argv[1], sys.argv[2]
data = {"mcpServers": {}}
if os.path.exists(mcp_path):
    try:
        with open(mcp_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception:
        pass
data.setdefault("mcpServers", {})["skrills"] = {
    "type": "stdio",
    "command": bin_path,
    "args": ["serve"]
}
with open(mcp_path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print(f"Configured skrills MCP server in {mcp_path}")
PY
        fi
      fi
    else
      echo "Warning: 'claude' command not found. Cannot register MCP server automatically." >&2
      echo "Please manually run: claude mcp add --transport stdio skrills -- $BIN_PATH serve" >&2
    fi
  fi
fi

# Configure MCP server for Codex in config.toml
if [ "$CLIENT" = "codex" ]; then
  if [ -n "$HTTP_ADDR" ]; then
    # HTTP transport mode for Codex
    echo "Installing skrills with HTTP transport on $HTTP_ADDR..."
    install_systemd_service "$HTTP_ADDR" || true

    MCP_URL="http://${HTTP_ADDR}/mcp"

    # Remove any existing stdio config and add HTTP config
    if [ -f "$CONFIG_TOML" ]; then
      # Remove existing skrills entry
      tmp=$(mktemp)
      awk '
        BEGIN { skip = 0 }
        /^\[mcp_servers\.skrills\]/ { skip = 1; next }
        /^\[/ { if (skip) skip = 0 }
        { if (!skip) print }
      ' "$CONFIG_TOML" > "$tmp" && mv "$tmp" "$CONFIG_TOML"
    fi

    # Add HTTP MCP server entry
    mkdir -p "$(dirname "$CONFIG_TOML")"
    tmp=$(mktemp)
    cat > "$tmp" <<MCP_ENTRY
# Skrills MCP server for skill sync, validation, and analysis (HTTP transport)
[mcp_servers.skrills]
type = "http"
url = "$MCP_URL"

MCP_ENTRY
    if [ -f "$CONFIG_TOML" ]; then
      cat "$CONFIG_TOML" >> "$tmp"
    fi
    mv "$tmp" "$CONFIG_TOML"
    echo "Registered skrills MCP server (HTTP) in $CONFIG_TOML"
  else
    # Default: stdio transport mode
    if [ -f "$CONFIG_TOML" ]; then
      if grep -q '\[mcp_servers.skrills\]' "$CONFIG_TOML" 2>/dev/null; then
        # Ensure required type field exists; older installs may be missing it.
        if ! awk '
          /^\[mcp_servers\.skrills\]/{inside=1}
          inside && /^\[/ && !/^\[mcp_servers\.skrills\]/{inside=0}
          inside && /^[[:space:]]*type[[:space:]]*=/{found=1}
          END{exit(found?0:1)}
        ' "$CONFIG_TOML"; then
          tmp=$(mktemp)
          awk '
            BEGIN{inside=0;added=0}
            /^\[mcp_servers\.skrills\]/{inside=1}
            inside && /^\[/ && !/^\[mcp_servers\.skrills\]/{inside=0}
            {
              if(inside && /^[[:space:]]*type[[:space:]]*=/){added=1}
              print
              if(inside && /^[[:space:]]*command[[:space:]]*=/ && added==0){
                print "type = \"stdio\""
                added=1
              }
            }
            END{
              if(inside && added==0){
                print "type = \"stdio\""
              }
            }
          ' "$CONFIG_TOML" > "$tmp" && mv "$tmp" "$CONFIG_TOML"
          echo "Patched missing type=\"stdio\" for skrills in $CONFIG_TOML"
        else
          echo "skrills MCP server already registered in $CONFIG_TOML"
        fi
      else
        # Add skrills MCP server entry
        tmp=$(mktemp)
        cat > "$tmp" <<MCP_ENTRY
# Skrills MCP server for skill sync, validation, and analysis
[mcp_servers.skrills]
command = "$BIN_PATH"
type = "stdio"
args = ["serve"]

MCP_ENTRY
        cat "$CONFIG_TOML" >> "$tmp"
        mv "$tmp" "$CONFIG_TOML"
        echo "Registered skrills MCP server in $CONFIG_TOML"
      fi
    else
      # Create new config.toml with skrills MCP server
      mkdir -p "$(dirname "$CONFIG_TOML")"
      cat > "$CONFIG_TOML" <<CONFIG
# Skrills MCP server for skill sync, validation, and analysis
[mcp_servers.skrills]
command = "$BIN_PATH"
type = "stdio"
args = ["serve"]
CONFIG
      echo "Created $CONFIG_TOML with skrills MCP server"
    fi
  fi
  enable_codex_skills_feature
fi

if [ "$CLIENT" = "codex" ]; then
  warm_cache_snapshot
  echo ""
  echo "Install complete for Codex."
  echo ""
  if [ -n "$HTTP_ADDR" ]; then
    echo "The skrills MCP server has been registered in ~/.codex/config.toml (HTTP: $HTTP_ADDR)"
    echo "Service management: systemctl --user {status|restart|stop} skrills"
  else
    echo "The skrills MCP server has been registered in ~/.codex/config.toml"
  fi
  echo "Available tools: sync-skills, sync-commands, sync-all, validate-skills, analyze-skills"
else
  warm_cache_snapshot
  echo ""
  echo "Install complete for Claude Code."
  echo ""
  if [ -n "$HTTP_ADDR" ]; then
    echo "The skrills MCP server has been registered (HTTP: $HTTP_ADDR)"
    echo "Service management: systemctl --user {status|restart|stop} skrills"
  else
    echo "The skrills MCP server has been registered."
  fi
  echo "Available tools: sync-skills, sync-commands, sync-all, validate-skills, analyze-skills"
fi

if [ "$UNIVERSAL" -eq 1 ]; then
  sync_universal
fi
