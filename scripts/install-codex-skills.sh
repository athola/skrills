#!/usr/bin/env bash
# Install codex-mcp-skills into ~/.codex (hook + MCP server registration)
# Flags:
#   --universal        Also sync skills into ~/.agent/skills for cross-agent reuse.
#   --universal-only   Only perform the universal sync (no hook/server install).
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
if [ "${CODEX_SKILLS_UNIVERSAL:-0}" != "0" ]; then
  UNIVERSAL=1
fi

# Preferred binary path (can be set by outer installer via BIN_PATH or CODEX_SKILLS_BIN).
BIN_PATH="${BIN_PATH:-${CODEX_SKILLS_BIN:-$HOME/.cargo/bin/codex-mcp-skills}}"
HOOK_DIR="$HOME/.codex/hooks/codex"
HOOK_PATH="$HOOK_DIR/prompt.on_user_prompt_submit"
MCP_PATH="$HOME/.codex/mcp_servers.json"
CONFIG_TOML="$HOME/.codex/config.toml"
REPO_ROOT="$(cd "${0%/*}/.." && pwd)"

sync_universal() {
  local AGENT_SKILLS="${AGENT_SKILLS_DIR:-$HOME/.agent/skills}"
  local CODEX_SKILLS_DIR="${CODEX_SKILLS_DIR:-$HOME/.codex/skills}"
  local MIRROR_DIR="${CODEX_MIRROR_DIR:-$HOME/.codex/skills-mirror}"
  mkdir -p "$AGENT_SKILLS"
  echo "Universal sync: copying skills into $AGENT_SKILLS"
  copy_tree() {
    local src="$1"
    [ -d "$src" ] || return 0
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --update "$src"/ "$AGENT_SKILLS"/
    else
      (cd "$src" && tar -cf - .) | (cd "$AGENT_SKILLS" && tar -xf -)
    fi
  }
  # Refresh Claude mirror first if binary exists
  if [ -x "$BIN_PATH" ]; then
    "$BIN_PATH" sync || echo "Warning: sync-from-claude failed (continuing)."
  fi
  copy_tree "$CODEX_SKILLS_DIR"
  copy_tree "$MIRROR_DIR"
  echo "Universal sync complete."
}

if [ "$UNIVERSAL_ONLY" -eq 1 ]; then
  sync_universal
  exit 0
fi

mkdir -p "$HOOK_DIR"
write_hook() {
  local tmp
  tmp=$(mktemp)
  cat > "$tmp" <<'HOOK' || { echo "Warning: unable to write hook (permission denied?). Skipping hook install." >&2; rm -f "$tmp"; return; }
#!/usr/bin/env bash
# Inject SKILL.md content into Codex on prompt submit via codex-mcp-skills
set -euo pipefail

BIN="${CODEX_SKILLS_BIN:-$HOME/.cargo/bin/codex-mcp-skills}"
REPO="${CODEX_SKILLS_REPO:-$HOME/codex-mcp-skills}"
CMD_ARGS=(emit-autoload)

# Optionally capture prompt text from stdin (Codex passes event payload on prompt submit).
PROMPT_INPUT=""
if [ ! -t 0 ]; then
  if IFS= read -r -t 0.05 first_line; then
    rest=$(cat)
    PROMPT_INPUT="${first_line}${rest}"
  fi
fi

if [ -n "${CODEX_SKILLS_PROMPT:-}" ]; then
  PROMPT_INPUT="$CODEX_SKILLS_PROMPT"
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
write_hook

# Ensure mcp_servers.json exists
if [ ! -f "$MCP_PATH" ]; then
  mkdir -p "$(dirname "$MCP_PATH")"
  if ! cat <<'JSON' > "$MCP_PATH"; then
{
  "mcpServers": {}
}
JSON
    echo "Warning: unable to create $MCP_PATH (permission denied?)" >&2
  fi
fi

# Merge/insert codex-skills entry; prefer jq, fall back to Python stdlib
if command -v jq >/dev/null 2>&1; then
  tmp=$(mktemp)
  jq '.mcpServers."codex-skills" = {"type": "stdio", "command": "'"$BIN_PATH"'", "args": ["serve"]}' "$MCP_PATH" > "$tmp"
  if mv "$tmp" "$MCP_PATH"; then
    :
  else
    echo "Warning: unable to update $MCP_PATH (permission denied?)" >&2
    rm -f "$tmp"
  fi
elif command -v awk >/dev/null 2>&1; then
  tmp=$(mktemp)
  backup="${MCP_PATH}.bak.$(date +%s)"
  cp "$MCP_PATH" "$backup" 2>/dev/null || true
  awk -v bin="$BIN_PATH" '
    BEGIN {
      found = 0
    }
    {
      if ($0 ~ /"codex-skills"[[:space:]]*:/) { found = 1 }
      print
    }
    END {
      if (!found) {
        printf "%s\n", (NR ? "," : "{");
        print "  \"mcpServers\": {"
        print "    \"codex-skills\": {"
        print "      \"type\": \"stdio\","
        printf "      \"command\": \"%s\",\n", bin
        print "      \"args\": [\"serve\"]"
        print "    }"
        print "  }"
        if (NR) { print "}" }
      }
    }
  ' "$MCP_PATH" > "$tmp"
  if mv "$tmp" "$MCP_PATH"; then
    echo "Updated $MCP_PATH without jq (backup: $backup)"
  else
    echo "Warning: unable to update $MCP_PATH (permission denied?)" >&2
    rm -f "$tmp"
  fi
else
  echo "Warning: jq not found and awk unavailable; please add codex-skills entry to $MCP_PATH manually." >&2
fi

echo "Registered codex-skills MCP server in $MCP_PATH"

# Also ensure codex-skills is present in config.toml (Codex prefers this over mcp_servers.json)
if [ ! -f "$CONFIG_TOML" ]; then
  mkdir -p "$(dirname "$CONFIG_TOML")"
  if ! printf 'model = "gpt-5.1-codex-max"\n\n' > "$CONFIG_TOML"; then
    echo "Warning: unable to create $CONFIG_TOML (permission denied?)" >&2
  fi
fi
if [ -w "$CONFIG_TOML" ] && ! grep -q '\[mcp_servers\."codex-skills"\]' "$CONFIG_TOML"; then
  cat <<EOF >> "$CONFIG_TOML"

[mcp_servers."codex-skills"]
type = "stdio"
command = "$BIN_PATH"
args = ["serve"]
EOF
  echo "Registered codex-skills MCP server in $CONFIG_TOML"
elif grep -q '\[mcp_servers\."codex-skills"\]' "$CONFIG_TOML"; then
  # Ensure type is present in the existing section (Codex MCP now requires it).
  tmp=$(mktemp)
  awk '
    BEGIN { in_section = 0; found_type = 0 }
    /^\[mcp_servers\."codex-skills"\]/ {
      in_section = 1
      found_type = 0
      print
      next
    }
    /^\[/ {
      if (in_section && !found_type) { print "type = \"stdio\"" }
      in_section = 0
    }
    {
      if (in_section && $0 ~ /^type[[:space:]]*=/) { found_type = 1 }
      print
    }
    END {
      if (in_section && !found_type) { print "type = \"stdio\"" }
    }
  ' "$CONFIG_TOML" > "$tmp" && mv "$tmp" "$CONFIG_TOML"
  echo "codex-skills already present in $CONFIG_TOML (type ensured)"
else
  echo "Warning: unable to update $CONFIG_TOML (permission denied?)" >&2
fi

echo "Install complete. To mirror Claude skills: run 'codex-mcp-skills sync' (binary must be built)."

if [ "$UNIVERSAL" -eq 1 ]; then
  sync_universal
fi
