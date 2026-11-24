# Custom Adapters

Skrills syncs between Claude, Codex, Copilot and Cursor out of the box. To sync
with another tool, describe its config layout in one TOML file. No source
change or rebuild is needed.

## Register an adapter

Put one file per tool in `~/.skrills/adapters/`. The file name does not matter
beyond ending in `.toml`; the `name` key inside is what you type on the command
line.

A worked example for a hypothetical Windsurf-style layout, saved as
`~/.skrills/adapters/windsurf.toml`:

```toml
name = "windsurf"
root = "~/.windsurf"

skills_dir = "skills"            # ~/.windsurf/skills/<skill>/SKILL.md
commands_dir = "workflows"       # ~/.windsurf/workflows/<command>.md
agents_dir = "agents"            # ~/.windsurf/agents/<agent>.md
instructions_file = "rules.md"   # ~/.windsurf/rules.md

[mcp]
path = "mcp_config.json"         # ~/.windsurf/mcp_config.json
servers_key = "mcpServers"       # {"mcpServers": {"name": {...}}}
```

Use it like a built-in:

```bash
skrills sync-all --from claude --to windsurf --dry-run
skrills sync-all --from claude --to windsurf
skrills sync-all --from windsurf --to claude
skrills sync-rollback                        # undoes the last sync, custom targets included
```

Sync from a custom adapter to a built-in works too. With `--to` omitted, a
custom source goes to Claude for `sync-commands`, `sync-mcp-servers`,
`sync-preferences` and `sync-status`, and to all four built-in tools for
`sync-all`. A custom adapter is written only when you name it, never as one of
the "every other tool" targets.

## Schema

| Key | Required | Meaning |
|-----|----------|---------|
| `name` | yes | Lowercase letters, digits, `-` or `_`, starting with a letter or digit. Must not equal `claude`, `codex`, `copilot` or `cursor`, and must be unique across files. |
| `root` | yes | The tool's config directory. `~` and `~/...` expand to your home; the result must be absolute. |
| `skills_dir` | no | Directory of skill folders, relative to `root`. |
| `commands_dir` | no | Directory of slash command files, relative to `root`. |
| `agents_dir` | no | Directory of agent files, relative to `root`. |
| `instructions_file` | no | One markdown file, relative to `root`. |
| `[mcp]` `path` and `servers_key` | no | JSON file relative to `root`, and the top-level key that holds the servers object. Both are required when `[mcp]` is present. |

An unknown key is an error, so a typo such as `skill_dir` is reported instead of
silently ignored.

Only the categories you configure are supported. Everything else is skipped in
the sync report as unsupported by the target, as for any built-in that lacks a
category. Preferences, hooks and plugin assets are never supported by a custom
adapter.

## File formats

The adapter uses Claude's formats, the plainest of the built-ins:

- **Skills**: `<skills_dir>/<name>/SKILL.md` plus companion files in that
  folder. Read one level deep; the folder name is the skill name.
- **Commands and agents**: `<name>.md` files directly in the directory, copied
  byte for byte. Subdirectories are not read.
- **Instructions**: skrills keeps its synced text in a marked block of the file
  and leaves the rest of your file alone. The block is not read back as your own
  instructions.
- **MCP servers**: merged by name under `servers_key`. Other keys in the file and
  servers only the tool knows are kept. Servers are written in Claude's
  spelling: `command`, `args`, `env` for stdio; `type: "http"`, `url`,
  `headers` for HTTP; `disabled: true` for a disabled server. Reading also
  accepts `enabled: false`. `sse` entries and entries sync cannot carry are
  skipped. When the tool's own file holds such an entry under a name the
  source also has, sync leaves that entry as it is and the report says why.

If your tool needs different file contents (renamed frontmatter keys, another
server schema), a config file cannot express that. It needs a built-in adapter.

## Safety

Sync writes into the paths in this file, so every path is held inside `root`.
A path is rejected, when the file loads and again by `skrills doctor`, if it is
absolute, contains `..`, or resolves outside `root` through a symlink. A
symlink whose target does not exist yet is followed too, since a write would
follow it. A symlinked directory that stays inside `root` is fine. Two keys
that name the same path are rejected, since each kind would read the other's
files.

Snapshots work as for built-ins: files a sync overwrites or creates are
recorded, and `skrills sync-rollback` restores or removes them. `--interactive`
asks one question per category for custom targets. Rejecting a single item
writes back the target's own copy, which an adapter allows only once a
round-trip test shows that leaves the file as it was. The custom adapter has
no such test, and it merges the instructions and MCP files rather than copying
them.

## Validate with doctor

```bash
skrills doctor
```

For each adapter file, doctor prints the path and name, the categories it
supports, and what is wrong with it. Lines marked `!` are errors and make
doctor exit non-zero; lines marked `w` are warnings.

```
== custom adapters (/home/you/.skrills/adapters) ==
windsurf (/home/you/.skrills/adapters/windsurf.toml)
  i supports: skills, commands, agents, instructions, mcp_servers
  w root /home/you/.windsurf does not exist yet; sync creates it on the first write
/home/you/.skrills/adapters/broken.toml: not usable
  ! missing required key `name`
  ! `skills_dir` = `../elsewhere` leaves root: use a relative path without `..` that no symlink redirects out
```

Doctor checks:

- the file does not parse, or has an unknown key (reported with its path)
- required keys are missing
- the name is a built-in's, is not a lowercase word, or is declared by another file
- `root` is not absolute after `~` expansion (error) or does not exist (warning)
- a directory or file leaves `root`, or two keys name the same path
- the adapters directory cannot be read (one error line; the rest of the report still prints)

A file with an error is not usable. Naming it in `--from` or `--to` fails with its
path and problems, and an unknown name fails listing the built-in and configured
names.

## Library use

`skrills_sync::Platform` is unchanged. A side of a sync is a
`skrills_sync::SyncEndpoint`, either `Builtin(Platform)` or `Custom(name)`;
`sync_between_endpoints` and `create_endpoint_adapter` resolve it.
`ConfigAdapter` implements the same `AgentAdapter` trait as the built-ins. It
wraps an `AdapterConfig`, which only `AdapterConfig::from_toml` and
`load_adapter` build, so every config it gets has passed the checks above. Its
paths are read through methods such as `skills_dir()`.
