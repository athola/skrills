# Autoloading Skills

This document describes how `codex-mcp-skills` discovers and autoloads skills.

## Skill Discovery

Skills are discovered by searching a prioritized list of directories. When multiple skills with the same name are found, the one from the highest-priority directory is used.

The default discovery priority is:
1.  `~/.codex/skills`
2.  `~/.codex/skills-mirror` (a mirror of Claude skills)
3.  `~/.claude/skills`
4.  `~/.agent/skills`

You can override the discovery priority by creating a `~/.codex/skills-manifest.json` file. This file can also be used to configure other settings, such as `expose_agents` and `cache_ttl_ms`.

## Autoload Filtering

When a prompt is submitted, the autoloading system filters skills based on the following criteria:

-   **Hook timing**: We run on `UserPromptSubmit`, using the parsed intent to query a cached index and hydrate only the matching skills. This avoids front-loading all skills into the context window and prevents repeated filesystem walks per turn.
-   **Prompt Content**: The system tokenizes the prompt and searches for terms (three characters or longer) in the skill's name and the first 4KB of its content.
-   **Pinned Skills**: Manually pinned skills are always included. Auto-pinned skills from recent history are also included.
-   **Source Filtering**: Skills from the `claude` and `mirror` sources can be excluded by using the `--include-claude` flag.
-   **Byte Budget**: The `--max-bytes` flag sets a budget for the total size of the included skills. If a skill is too large, it will be truncated, and this will be noted in the diagnostics.
-   **Diagnostics**: A footer is added to the output with diagnostic information, including which skills were included, skipped, or truncated, and any duplicates that were found.

## Caching

To improve performance, `codex-mcp-skills` uses two levels of caching:

-   **Discovery Cache**: This cache stores the list of discovered skills and has a configurable time-to-live (TTL). The TTL can be set with the `CODEX_SKILLS_CACHE_TTL_MS` environment variable or the `cache_ttl_ms` setting in the manifest file. The cache is invalidated by a file watcher (when using the `--watch` flag) or by manually running the `refresh-cache` command.
-   **Content Cache**: This cache stores the content of the skills, keyed by their path and a hash of their content. The cache is automatically refreshed when a file is changed or its hash no longer matches.

## Approach vs other loaders (feature-level)

- Uses prompt-hooked, cached lookup: intent parsed on submit, pulls only relevant skills into context → lower tokens/latency.
- Avoids eager preloading of every skill name/content into prompts at startup.
- Keeps skill text local and cached; no per-turn network/tool round-trips just to read SKILL.md files.
