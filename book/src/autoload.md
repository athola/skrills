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

When you submit a prompt, `skrills` filters your skills to find the most relevant ones. Here's how it works:

1.  **Hook Timing**: The autoload process is triggered by the `UserPromptSubmit` hook that the installer creates at `~/.codex/hooks/codex/prompt.on_user_prompt_submit` (Codex does not ship this hook by default).
2.  **Intent Parsing**: `skrills` parses the prompt to understand its intent.
3.  **Cached Query**: It then queries a cached index of your skills to find the ones that match the intent.
4.  **Context Injection**: Only the matching skills are injected into the context window.

This process is designed to be efficient. It avoids loading all of your skills into the context window at once, and it doesn't need to read your skill files from the disk every time you submit a prompt.

If the hook file is missing, autoload will not run. Reinstall or create the hook manually to invoke `skrills serve`/autoload on prompt submission.
...
This allows you to log or control the size of the payload before it's sent to the model. The inputs are the same as for `autoload-snippet`: `prompt`, `embed_threshold`, `include_claude`, `max_bytes`, `auto_pin`, and `diagnose`.

## Caching

To improve performance, `skrills` uses two levels of caching:

-   **Discovery Cache**: This cache stores the list of discovered skills and has a configurable time-to-live (TTL). The TTL can be set with the `SKRILLS_CACHE_TTL_MS` environment variable or the `cache_ttl_ms` setting in the manifest file. The cache is invalidated by a file watcher (when using the `--watch` flag) or by manually running the `refresh-cache` command.
-   **Content Cache**: This cache stores the content of the skills, keyed by their path and a hash of their content. The cache is automatically refreshed when a file is changed or its hash no longer matches.

## Comparison with Other Loaders

The `skrills` autoloading approach differs from other skill loaders in a few key ways:

-   **Dynamic Loading**: `skrills` uses a prompt hook to parse the user's intent and load only the relevant skills. This is in contrast to approaches that preload all skill names and content into the prompt at startup.
-   **Local Caching**: Skills are cached locally, which means that the loader doesn't need to make network requests or tool calls to read `SKILL.md` files on every turn. This reduces latency and improves performance.
