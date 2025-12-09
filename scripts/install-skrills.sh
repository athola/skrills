#!/usr/bin/env bash
# Install skrills hook + MCP server registration for Codex or Claude.
# Flags:
#   --universal        Also sync skills into ~/.agent/skills for cross-agent reuse.
#   --universal-only   Only perform the universal sync (no hook/server install).
# Environment:
#   SKRILLS_MIRROR_SOURCE  Source directory for mirroring skills (default: ~/.claude).
#                          Automatically skipped if same as install location.
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

# Determine base dir (may inform client detection)
if [ -n "${SKRILLS_BASE_DIR:-}" ]; then
  BASE_DIR="$SKRILLS_BASE_DIR"
elif [ "$CLIENT" = "claude" ]; then
  BASE_DIR="$HOME/.claude"
else
  # Default to .codex for auto-detection or explicit codex
  BASE_DIR="$HOME/.codex"
fi
mkdir -p "$BASE_DIR"

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
  candidate=$(detect_client_from_base "$BASE_DIR")
  if [ -z "$candidate" ]; then
    candidate=$(probe_signature_files "$BASE_DIR")
  fi
  if [ -z "$candidate" ]; then
    CLIENT="codex" # MCP-first default
  else
    CLIENT="$candidate"
  fi
fi

HOOK_DIR=""
HOOK_PATH=""
if [ "$CLIENT" = "claude" ]; then
  HOOK_DIR="$BASE_DIR/hooks"
  HOOK_PATH="$HOOK_DIR/prompt.on_user_prompt_submit"
fi
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

clean_legacy_codex_mcp_skills() {
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

  # Clean legacy hook files that used the old name.
  if [ -d "$HOOK_DIR" ]; then
    find "$HOOK_DIR" -maxdepth 1 -type f -name '*codex-mcp-skills*' -exec rm -f {} \; 2>/dev/null
  fi

  if [ "$removed" -eq 1 ]; then
    echo "Legacy codex-mcp-skills artifacts cleaned."
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
    echo "Installed default subagents config to $config_path (default_backend=codex)"
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
  if "$BIN_PATH" list >/dev/null 2>&1; then
    echo "Cache primed; subsequent startups will reuse the snapshot."
  else
    echo "Warning: cache warmup failed (continuing without primed cache)." >&2
  fi
}

if [ "$UNIVERSAL_ONLY" -eq 1 ]; then
  sync_universal
  exit 0
fi

clean_legacy_codex_mcp_skills
clean_invalid_claude_model
install_subagents_config

if [ -n "$HOOK_DIR" ]; then
  mkdir -p "$HOOK_DIR"
fi

write_hook() {
  local tmp
  tmp=$(mktemp)
  cat > "$tmp" <<'HOOK' || { echo "Warning: unable to write hook (permission denied?). Skipping hook install." >&2; rm -f "$tmp"; return; }
#!/usr/bin/env bash
# Inject SKILL.md content into Claude Code on prompt submit via skrills
set -euo pipefail

BIN="${SKRILLS_BIN:-$HOME/.cargo/bin/skrills}"
REPO="${SKRILLS_REPO:-$HOME/skrills}"
CMD_ARGS=(emit-autoload)

# Optionally capture prompt text from stdin (Claude Code passes event payload on prompt submit).
PROMPT_INPUT=""
if [ ! -t 0 ]; then
  if IFS= read -r -t 0.05 first_line; then
    rest=$(cat)
    PROMPT_INPUT="${first_line}${rest}"
  fi
fi

if [ -n "${SKRILLS_PROMPT:-}" ]; then
  PROMPT_INPUT="$SKRILLS_PROMPT"
fi

if [ -n "$PROMPT_INPUT" ]; then
  CMD_ARGS+=(--prompt "$PROMPT_INPUT")
fi

run_cmd() {
  if [ -x "$BIN" ]; then
    "$BIN" "${CMD_ARGS[@]}"
  elif [ -d "$REPO" ]; then
    (cd "$REPO" && cargo run --quiet -- "${CMD_ARGS[@]}")
  else
    echo "{}" && exit 0
  fi
}

OUTPUT=$(run_cmd || true)
if [ -n "${OUTPUT:-}" ]; then
  echo "$OUTPUT"
fi
HOOK
  if mv "$tmp" "$HOOK_PATH"; then
    chmod +x "$HOOK_PATH"
    echo "Hook written to $HOOK_PATH"
  else
    echo "Warning: unable to move hook into place at $HOOK_PATH" >&2
    rm -f "$tmp"
  fi
}
if [ "$CLIENT" = "claude" ]; then
  write_hook
else
  echo "Codex install: skipping hook install (using MCP instead)."
fi

# Configure MCP server for Claude Code using claude mcp add command
if [ "$CLIENT" = "claude" ]; then
  # Use the official claude mcp add command for proper registration
  if command -v claude >/dev/null 2>&1; then
    echo "Registering skrills MCP server with Claude Code..."
    if claude mcp add --transport stdio skrills -- "$BIN_PATH" serve 2>/dev/null; then
      echo "Successfully registered skrills MCP server"
    else
      echo "Warning: Failed to register MCP server using 'claude mcp add'. Falling back to manual configuration." >&2

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

# Configure MCP server for Codex in config.toml
if [ "$CLIENT" = "codex" ]; then
  # Register skrills MCP server in config.toml
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
      # Prepend to existing config.toml (skrills entry first)
      cat > "$tmp" <<MCP_ENTRY
# Skrills MCP server for skill management
[mcp_servers.skrills]
command = "$BIN_PATH"
type = "stdio"
args = ["serve"]

MCP_ENTRY
      # Append existing config.toml content
      cat "$CONFIG_TOML" >> "$tmp"
      mv "$tmp" "$CONFIG_TOML"
      echo "Registered skrills MCP server in $CONFIG_TOML"
    fi
  else
    # Create new config.toml with skrills MCP server
    mkdir -p "$(dirname "$CONFIG_TOML")"
    cat > "$CONFIG_TOML" <<CONFIG
# Skrills MCP server for skill management
[mcp_servers.skrills]
command = "$BIN_PATH"
type = "stdio"
args = ["serve"]
CONFIG
    echo "Created $CONFIG_TOML with skrills MCP server"
  fi

  # Install AGENTS.md with skill-loading instructions
  agents_md="$BASE_DIR/AGENTS.md"
  skrills_marker='<!-- skrills-integration-start -->'
  if [ -f "$agents_md" ] && grep -q "$skrills_marker" "$agents_md" 2>/dev/null; then
    echo "AGENTS.md already contains skrills integration"
  else
    tmp=$(mktemp)
    # Add skrills integration to AGENTS.md
    cat >> "$tmp" <<AGENTS_MD

<!-- skrills-integration-start -->
# Skrills Autoload Protocol

## MANDATORY: Skill Loading Protocol

Before responding to ANY user request, you MUST first call the `autoload-snippet` MCP tool to load relevant skills to complete the task.

### Required Workflow

1. **User sends a message**
2. **Immediately call `autoload-snippet`** with the user's message as the `prompt` parameter
3. **Read the returned skill content** - these are specialized instructions and patterns
4. **Use the loaded skills** to inform your response to the user

### Example Tool Call

\`\`\`json
{
  "name": "autoload-snippet",
  "arguments": {
    "prompt": "<paste the user's full message here>"
  }
}
\`\`\`

### Why This Matters

- Skills contain domain-specific best practices, patterns, and instructions
- They are dynamically matched to your current task
- Skipping skill loading means missing critical context required to perform your task correctly.
- The skill content is working instructions, not just reference material

### When to Load Skills

- At the START of every new user message
- When the user's task changes significantly within a conversation
- After any context reset

DO NOT skip this step. The skill content is essential for providing high-quality, contextually-appropriate responses.
<!-- skrills-integration-end -->
AGENTS_MD

    # If AGENTS.md exists, append to it; otherwise create it
    if [ -f "$agents_md" ]; then
      cat "$agents_md" >> "$tmp"
      mv "$tmp" "$agents_md"
      echo "Updated AGENTS.md with skrills integration"
    else
      mv "$tmp" "$agents_md"
      echo "Created AGENTS.md with skrills integration"
    fi
  fi
fi

if [ "$CLIENT" = "codex" ]; then
  warm_cache_snapshot
  echo ""
  echo "Install complete for Codex (MCP-first)."
  echo ""
  echo "The skrills MCP server has been registered in ~/.codex/config.toml"
  echo "and AGENTS.md has been updated with skill-loading instructions."
  echo ""
  echo "Skills will be auto-loaded via MCP when Codex starts."
else
  warm_cache_snapshot
  echo "Install complete for Claude Code."
  echo ""
  echo "The skrills MCP server has been registered and hook installed."
  echo "Skills will be auto-loaded on each prompt."
fi

if [ "$UNIVERSAL" -eq 1 ]; then
  sync_universal
fi
