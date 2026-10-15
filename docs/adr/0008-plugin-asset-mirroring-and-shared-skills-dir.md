# ADR 0008: Plugin-Asset Mirroring and the Shared Skills Directory

- Status: Accepted
- Date: 2026-10-15
- Amends: [ADR 0006](0006-cursor-rules-mapping.md)

## Context

ADR 0006 kept Cursor rules on the existing `instructions` methods of
`AgentAdapter` rather than adding methods only one adapter implements. Since
then three plugin-asset methods were added to the trait without an ADR:
`read_plugin_assets` (only Claude implements it), `write_plugin_assets` and
`preview_plugin_assets` (only Cursor implements them). They let sync mirror a
whole Claude plugin into `~/.cursor/plugins/local/<plugin>/`, scripts and
hooks included, because skills that call a plugin's scripts failed in Cursor
without them.

In 0.9.1 Codex skills moved to `~/.agents/skills`, a directory that other
tools load too. The vendor documentation read on 2026-10-15 says:

| Tool | User-level skill directories it loads |
|---|---|
| Codex | `~/.agents/skills` |
| Cursor | `~/.agents/skills`, `~/.cursor/skills`, and for compatibility `~/.claude/skills` and `~/.codex/skills` |
| GitHub Copilot CLI | `~/.copilot/skills`, `~/.agents/skills` |

Sources: <https://cursor.com/docs/context/skills>,
<https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills>.

So a skill synced from Claude to Cursor can load in Cursor twice: once from
`~/.claude/skills` directly and once from the copy in `~/.cursor/skills`. A
Claude-to-Codex sync adds a third copy in `~/.agents/skills`. Copilot sees
two copies after Claude-to-Copilot plus Claude-to-Codex syncs.

## Decision

1. The plugin-asset methods stay on `AgentAdapter` with no-op defaults. This
   departs from ADR 0006's interface-segregation argument on purpose: the
   orchestrator drives every phase through one trait, and a second trait
   for one reader and one writer would need its own dispatch and dry-run
   path for the same result. Adapters without a plugin cache or mirror keep
   the defaults.
2. Cursor's flat skill sync stays on. Users with a Cursor that predates
   `~/.claude/skills` loading, or a Codex or Copilot source, depend on it.
3. Every skill directory skrills writes carries a `.skrills-synced` marker
   file. Unmarked directories are never removed. A later release can offer a
   dedupe that removes only marked copies a tool already loads from another
   directory; until then duplicates are documented, not deleted.
4. `~/.agents/skills` is the shared target for Codex. Every adapter writes a
   skill to one folder name (the last name segment in kebab case), so a
   skill keeps one folder in every directory above.

## Alternatives Considered

1. **Stop syncing Claude skills to Cursor.** Rejected for now: it would
   drop skills for Cursor versions and sources the compatibility loading
   does not cover, and existing copies could not be told apart from the
   user's own skills.
2. **Delete copies a tool already loads.** Rejected: copies written before
   the marker cannot be identified safely.
3. **A separate `PluginAssetAdapter` trait.** Rejected as above.

## Consequences

- Cursor can list a synced Claude skill more than once. The marker makes a
  safe cleanup possible later.
- Cursor full-plugin-mirror trees carry the `.skrills-mirror` marker but not
  `.skrills-synced`.
