# Autoloading Skills

This document describes how `skrills` discovers and autoloads skills.

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

-   **Hook timing**: Autoloading activates on `UserPromptSubmit`. It uses the parsed intent to query a cached index and include only the matching skills. This approach avoids front-loading all skills into the context window and prevents repeated filesystem walks per turn.
...
This allows logging or controlling payload size before sending `additionalContext` to the model. Inputs mirror `autoload-snippet`: `prompt`, `embed_threshold`, `include_claude`, `max_bytes`, `auto_pin`, `diagnose`.

## Caching

To improve performance, `skrills` uses two levels of caching:

-   **Discovery Cache**: This cache stores the list of discovered skills and has a configurable time-to-live (TTL). The TTL can be set with the `SKRILLS_CACHE_TTL_MS` environment variable or the `cache_ttl_ms` setting in the manifest file. The cache is invalidated by a file watcher (when using the `--watch` flag) or by manually running the `refresh-cache` command.
-   **Content Cache**: This cache stores the content of the skills, keyed by their path and a hash of their content. The cache is automatically refreshed when a file is changed or its hash no longer matches.

## Approach vs other loaders (feature-level)

- Employs a prompt-hooked, cached lookup: intent is parsed on submission, pulling only relevant skills into context. This results in reduced token usage and lower latency.
- Avoids eager preloading of all skill names and content into prompts at startup.
- Keeps skill text local and cached, eliminating per-turn network or tool round-trips solely for reading SKILL.md files.
