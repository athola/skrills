# Changelog

## 0.9.1 - Unreleased

- **Fix: skills synced to Codex load there**: Codex loads user skills from `~/.agents/skills` (learn.chatgpt.com/docs/build-skills), and its documentation no longer lists `~/.codex/skills`, where sync wrote them. Skills now go to `~/.agents/skills/<name>/SKILL.md`, one level deep with the frontmatter `name` set to the folder name; every other byte is kept. A nested source skill such as `synced/<id>/morning` lands in `morning/`. Skills already in `~/.codex/skills` are still read when Codex is the source and are not moved. Agents, which sync stores as `agent-*` skills, go to `~/.agents/skills/agent-<name>` too.
  This applies to every Claude-to-Codex path: `skrills sync`, `mirror` (which now honours `--dry-run` for skills), `sync-all`, the TUI and the `sync-from-claude` and `sync-all` MCP tools all go through the Codex adapter, where they used to copy the `~/.claude` tree into `~/.codex/skills` with its nesting kept. A Codex config the skills flag cannot be written to is reported as a warning on a successful skill write. The Codex skill-trace probe and `create-skill` on a Codex-only install also use `~/.agents/skills`. Discovery and `skrills setup --universal` read and write `~/.agents/skills` in place of `~/.agent/skills`, which no tool loads. **MCP output change:** `sync-from-claude`'s structured `report` drops `synced` (per-skill names) and adds `warnings`.
- **Fix: Copilot sync writes where Copilot CLI reads on macOS**: Copilot CLI reads `COPILOT_HOME`, else `~/.copilot`. On macOS skrills resolved `~/Library/Application Support/copilot` (the platform config directory) whenever no XDG or `~/.copilot` directory existed, so synced skills, agents and MCP servers went where Copilot never looks. Sync and `skrills setup` now share one lookup: `COPILOT_HOME`, then an existing `$XDG_CONFIG_HOME/copilot` (or `~/.config/copilot` off macOS), then `~/.copilot`. Content already in the old macOS directory is not moved.
- **Fix: skills synced to Cursor load there**: Cursor loads a `SKILL.md` only when its frontmatter has `name`, equal to the folder name, and `description`. Sync stripped the frontmatter and put the description on the first line, so no flat-synced skill was in a form Cursor loads. The Cursor copy now keeps `name`, `description` and `disable-model-invocation`, and a skill without a description is named in the report. The other Claude fields still ride in the trailing comment that a sync back out of Cursor restores.
- **Fix: plugin hooks run in Cursor**: a mirrored plugin's `hooks/hooks.json` was copied in Claude's nested PascalCase layout, which Cursor does not read. It is now rewritten to Cursor's flat camelCase entries, each carrying its group's `matcher`, with `${CLAUDE_PLUGIN_ROOT}` replaced by the mirror's absolute path. Events Cursor lacks are dropped and named in the report; a file that does not parse is copied unchanged with a warning.
- **Fix: Claude hooks no longer run twice in Cursor**: Cursor loads hooks from `~/.claude/settings.json` itself and runs every match from `~/.cursor/hooks.json` too, so Claude-to-Cursor sync no longer copies top-level hooks there. The report lists them as skipped with the reason.
- **Fix: read-only agents stay read-only in Cursor**: Cursor agents have no tool list, so a Claude agent limited to read tools could write in Cursor. One whose `tools` hold none of `Write`, `Edit`, `MultiEdit`, `NotebookEdit` or `Bash` is now written with `readonly: true`, which a sync back to Claude removes again.

### Owner decisions

Follow-up to the review: items that needed a decision rather than a fix.

- **sync**: settings.json and mcp.json rewrites keep the user's key order instead of sorting keys (serde_json `preserve_order`).
- **sync**: Breaking: `SyncValidationOptions` loses `validate`, `strict` and `create_backups`; nothing read them.
- **sync**: every target writes a skill to one folder, the last segment of its name in kebab case, so `synced/abc/morning` is `morning/` in Claude, Codex, Copilot and Cursor alike and `a/b` no longer collides with `ab`. Claude and Copilot targets previously synced under other names (`My_Skill/`, nested paths) get a new folder; the old one is left in place.
- **sync**: each skill directory the skill writers create (Claude, Copilot, Cursor flat and `plugins/local`, `~/.agents/skills`) now holds a `.skrills-synced` marker so a later cleanup can tell skrills copies from skills you made. Directories copied by the Cursor full plugin mirror are not marked yet. Nothing is deleted.
- **sync**: `sync`, `mirror`, `sync-from-claude`, `sync-all --include-marketplace` and the MCP sync tools read marketplace skills when asked; `sync-all` and the MCP tools never did. Breaking: `skrills_server::sync::sync_skills_from_claude_to_codex` and `sync_skills_only_from_claude` are removed; use `skrills_sync::sync_claude_skills_to_codex`. Files a SKILL.md links to outside its own directory are no longer copied.
- **cold-window**: an engaged kill switch now stops syncs in other processes. cold-window writes its pid to `~/.skrills/kill-switch.pid` while the switch is engaged and removes it on exit; `sync-all`, `mirror` and the MCP sync tools refuse to write while that file names a running process. A file left by a dead process is ignored and removed.
- **sync**: Breaking: `skrills_sync::Platform` replaces platform strings: `create_adapter`, `sync_between` and `skill_delivery` take `Platform`; `default_target_for` and `is_valid_platform` are removed (use `Platform::default_target` and `str::parse::<Platform>`). A `clap` feature on skrills_sync and skrills-validate derives `ValueEnum`; the CLI's own copies of these enums are gone. Flag spellings are unchanged.
- **mcp**: `sync-skills` reports an unknown platform as "Invalid 'from'/'to' value" like the other sync tools.
- **cli**: `skrills validate --help` describes the `--target` values with skrills-validate's wording; the accepted values are unchanged.
- **Dependencies**: YAML frontmatter is parsed with `serde_norway` instead of the deprecated `serde_yaml`, whose `unsafe-libyaml` backend is unmaintained (RUSTSEC-2024-0449).
- **Build**: the unpublished test helper package is named `skrills-test-utils` to match the other crates; run its tests with `cargo test -p skrills-test-utils`.
- **Build**: `make test`, `make test-unit` and `make test-integration` now run tests in parallel by default, as CI does; set `TEST_THREADS=1` to serialize.
- **Build**: `crates/test-utils/tests/crate_boundaries.rs` fails when skrills-server exports a non-server module or re-exports another crate, when a crate other than skrills_sync defines `enum Platform` (ADR 0009), or when skrills-analyze depends on skrills-intelligence without `optional = true` (ADR 0010).
- **Build**: the workspace declares `rust-version = "1.89"` (std `File::lock`), so older compilers stop with an MSRV message instead of dependency errors; running the tests needs Rust 1.93.1+ (`serial_test` 4).
- **Cold-window TUI**: Breaking: `Esc` quits again when nothing is open to back out of. It closes the topmost overlay, then clears zoom, then clears the hint filter, and only then quits, matching the skill dashboard.
- **Dashboard**: `Esc` closes the help overlay instead of quitting while help is open; with help closed it still quits.
- **Cold-window TUI**: master-ack (`A`) keeps standing CAUTION/ADVISORY/STATUS alerts hidden while they keep firing; it used to hide them for one tick only. An acked alert shows again once it resolves and recurs. Breaking: `ColdWindowState::master_ack_version` is replaced by `acked_non_warnings` (a fingerprint set).
- **server**: Breaking: skrills-server drops the deprecated `manifest` and `runtime` modules, `mcp_gateway::{GatewayConfig, ToolUsageStats, MCP_GATEWAY_TOOL_NAMES}` and `discovery::{CliType, ParseCliTypeError, ENV_CLI_TYPE, detect_cli_type, read_agent_file, read_agent_file_for_cli, agent_file_path, agent_file_path_for_cli}` (deprecated in 0.9.0/0.9.1, no callers in skrills; `SKRILLS_CLI_TYPE` is no longer read anywhere). The crate-wide `deprecated = "allow"` is gone, so deprecations in its dependencies now fail the lint instead of passing silently.
- **subagents**: Several skrills servers (one per client) sharing `~/.codex/subagents/runs.json` no longer drop each other's runs: writes merge into the file under a lock (`runs.json.lock`), and a server starting up fails only unfinished runs whose owning process is gone instead of every unfinished run. Breaking: `RunRecord` has a new public `owner: Option<RunOwner>` field.
- **discovery**: The CLI commands that take `--skill-dir`, the dashboard and `/api/skills` now pick their default skill roots the way the MCP server does: `~/.claude/skills` and the Claude plugin cache only with `SKRILLS_INCLUDE_CLAUDE=1`, marketplaces only with `SKRILLS_INCLUDE_MARKETPLACE=1` too, and the manifest's `priority` order. Without the variable, `skrills validate`, `analyze`, `metrics` and the other commands no longer scan `~/.claude` by default. Breaking: `skrills_discovery::skill_roots_or_default` takes a `&RootSettings`; new `RootSettings` and `skill_roots`.
- **agent / multi-cli-agent**: Codex now runs agents as `codex exec --sandbox workspace-write "<prompt>"` instead of `codex --yolo exec --timeout_ms 1800000`: agent files found in plugin caches no longer get an unsandboxed, approval-free Codex, and the `--timeout_ms` flag, which Codex does not have, is gone. `--dangerously-bypass-sandbox` restores the old unsandboxed run when asked for; `multi-cli-agent` refuses it with the Claude backend. Codex refuses to start a sandboxed run outside a git repository or trusted directory.
- **scripts**: Every shell script now sources `scripts/logging.sh`, keeps its logic in functions, and takes `-h`/`--help` and `-x` (xtrace). Diagnostic messages that went to stdout (for example `publish_crates.sh` progress, `install.sh` "Installed ..." and PATH hint) now go to stderr through `log`; lines tools parse (lint "clean" verdicts, `--print-order`, audit `[shim]` lines, dogfood and test-install reports) stay on stdout.
- **build**: `make lint-shell` runs the new `scripts/shellcheck.sh`, which adds an `shfmt` formatting check to the shellcheck gate; `make check-deps` reports `shfmt`, and CI installs shfmt 3.13.1 by pinned digest.
- **sync**: a dry run right after a sync no longer reports a nested skill (`synced/abc/morning`) as a write; the preview compares the folder the skill is written to.
- **cold-window**: `SKRILLS_KILL_SWITCH_FILE` (an absolute path) moves the kill-switch file; `make test` and friends set it so a running cold-window does not fail the suite.
- **analyze**: the cold-window hint scorer from skrills-intelligence is behind the default-on `intelligence-scorer` feature; with `default-features = false` token counting builds without skrills-intelligence and hints keep collector order (ADR 0010).
- **Docs**: ADR 0008 (plugin-asset mirroring, the shared `~/.agents/skills` directory and which tools load which skill directories), ADR 0009 (one sync engine; discovery, sync and setup moved out of skrills-server) and ADR 0010.
- **crates**: Breaking: `skrills_server::discovery` is removed. Its helpers moved to `skrills_discovery` (`merge_extra_dirs`, `agent_roots`, `agents_manifest`, `collect_skills`, `collect_agents`, `read_skill`, `resolve_skill`, `resolve_agent`, `tokenize_prompt`, `trigram_counts`, `trigram_similarity`, `trigram_similarity_checked`, `cosine_similarity`, `read_prefix`, `relative_path`, `DEFAULT_AGENT_RUN_TEMPLATE`, `DEFAULT_EMBED_PREVIEW_BYTES`, `DEFAULT_EMBED_THRESHOLD`); `skill_roots` is now `skrills_discovery::skill_roots_from_env`; `priority_labels`, `priority_labels_and_rank_map` and `is_skill_file` were re-exports and are used from `skrills_discovery` directly; `AGENTS_TEXT` and the `AGENTS_*SECTION_*` markers moved to `skrills_sync`; `AGENTS_URI`, `AGENTS_NAME`, `AGENTS_DESCRIPTION`, `ENV_EXPOSE_AGENTS` are now private to the server (ADR 0009 decision 2).
- **crates**: Breaking: `skrills_server::sync` is removed. `MirrorReport`, `mirror_source_root`, `sync_agents` and `sync_agents_only_from_claude` are now in `skrills_sync` (ADR 0009 decision 2).
- **crates**: Breaking: `skrills_server::setup` is removed; it is now the `skrills-setup` crate (`Client`, `SetupConfig`, `is_setup`, `is_first_run`, `prompt_first_run_setup`, `interactive_setup`, `run_setup`, `ensure_codex_skills_feature_enabled`, plus `SetupConfig::include_marketplace` and an `include_marketplace` argument on `interactive_setup`). The server still uses it to report whether setup ran.
- **crates**: Breaking: `skrills_server::config` is removed; the `~/.skrills/config.toml` loader (`Config`, `ServeConfig`, `load_config`, `try_apply_config_to_env`, `apply_config_to_env`) is now `skrills::config` in the CLI crate, the only user.
- **setup**: `skrills setup --universal --include-marketplace` also copies skills from `~/.claude/plugins/marketplaces`, as `skrills sync --include-marketplace` does. Off by default because those skills belong to plugins that are not installed; `SKRILLS_INCLUDE_MARKETPLACE` sets it too.
- **agent / multi-cli-agent**: new `--timeout <seconds>` (default 1800, `0` = no limit) kills the Codex or Claude CLI when the run takes longer and exits with an error naming the limit. Runs had no limit since the unsupported `--timeout_ms` flag was dropped.
- **subagents / skill generation**: the MCP subagent Codex backend and LLM skill generation now build their `codex exec` argv through the same launcher as `skrills agent`; the argv each one runs is unchanged.
- **sync (Cursor)**: skill directories inside mirrored plugins under `~/.cursor/plugins/local/` now carry the `.skrills-synced` marker, like flat skill copies, so cleanup can tell them from skills the user made.

### Review fixes

A full review of the 0.9.1 branch found the issues below. Each fix has a test that failed before it.

#### Sync

- **Codex sync**: enabling the skills feature flag appended a second `[features]` table when `config.toml` defined `features` as an inline table or dotted keys, leaving a file Codex rejects; the key is now set where `features` already lives, and a config.toml that does not parse is left untouched with an error.
- **Cursor plugin mirror**: a plugin refused this run (name collision, symlinked or blocked directory) or hidden by an unreadable plugin cache directory had its existing mirror pruned; such mirrors are now kept, and `--dry-run` no longer lists them as prune targets.
- **Cursor plugin mirror**: a plugin file that could not be read this run caused its previous mirrored copy to be deleted as "no longer shipped"; the plugin is now mirrored without removing anything that run.
- **Cursor plugin mirror**: an unmarked `~/.cursor/plugins/local/<name>` holding the user's own files had plugin files written into it, and paths in the way deleted, before ownership was checked; such a directory is now left untouched, and an unreadable subtree counts as the user's.
- **Sync writes**: one skill, command or agent that could not be written aborted the rest of that phase on every run; it is now reported as refused and the remaining items are written.
- **Sync writes**: two items whose names differ only in case (`Foo`, `foo`) overwrote each other on case-insensitive file systems; the second is now refused and the first kept.
- **Sync readers**: a symlinked command, agent, rule or instruction file was followed and its target (any file on disk) copied into every other tool; symlinks and hidden paths are now skipped, as the skill readers already did.
- **Cursor hooks**: a `hooks.json` entry without `command` (a prompt hook) or with a fractional `timeout` failed the whole hooks phase, and unknown entry fields were dropped; entries are now carried as written and malformed ones skipped with a warning.
- **Cursor skills**: a key nested under a mapping (`metadata:\n  name: other`) was read as the skill's own `name`, so the skill was written without the folder name and Cursor skipped it silently; nested keys are now ignored.
- **Cursor skills**: a source skill with no frontmatter was copied verbatim, which Cursor does not load; it now gets `name: <folder>` and a warning that the description is missing.
- **Cursor rules**: a rule with its own `description` and no globs was rewritten with a generated `Rule: <name>` description, changing when Cursor selects it; the source description is now kept.
- **Cursor agents**: a `tools:` block list written at the key's own indent left orphan `- Read` lines in the Cursor frontmatter and marked agents with Bash or Write as `readonly`; the list is now recognized.
- **Claude source skills**: a plugin skill with the same folder name as one of the user's own skills replaced it whenever the plugin file was newer; the user's skill is now always kept.
- **Claude source plugins**: a `.md` file in any `commands`/`agents` directory inside a plugin (for example `docs/commands/README.md` or a skill's `agents/` module) was synced as a command or agent; only the plugin's top-level directories count now.
- **Dry run**: after a real sync to Codex, `--dry-run` kept reporting every renamed or nested skill as "would write"; the preview now compares against what the Codex writer would actually produce.
- **Claude MCP servers**: sync read and wrote MCP servers in `~/.claude/settings.json`, which Claude Code does not load them from; it now uses the user-scope `mcpServers` in `~/.claude.json`. Servers an earlier skrills version wrote into `settings.json` are no longer read.
- **Codex agents**: Claude agents synced to Codex were written to `~/.codex/skills`, which Codex no longer loads; they now go to `~/.agents/skills/agent-<name>` with `name` matching the folder. Copies left in `~/.codex/skills` by earlier versions are still read but not removed.
- **Model sync**: a Claude model synced to Codex or Copilot was replaced by `gpt-4o`/`gpt-4o-mini`, ids Codex no longer offers (and the reverse mapped OpenAI ids onto Claude shorthands); with no current mapping, the target now keeps its own model and the report says the model was left unchanged.
- **Cursor plugin mirror**: a symlink at `.skrills-mirror` had the file it points to overwritten by the marker, and a symlinked `plugins/local` entry could be pruned; neither is touched now.
- **Config writes**: writing through a symlink whose target did not exist yet replaced the link with a plain file; the link is now kept and its target created.
- **Codex agents**: a user's own `agent-<name>` skill was overwritten and then hidden as an agent; it is now refused. `config.toml` is no longer edited when every skill or agent was refused.
- **Kill switch**: an engaged kill switch refused writes with "token usage 0 exceeded ceiling 0"; the refusal now says the switch is engaged without inventing numbers (new `SyncError::KillSwitchEngaged`).
- **Claude source**: when two commands shared a name (`commands/commit.md`, `commands/git/commit.md`) the one synced depended on directory order, and skills, agents and hooks came back in random order; the top-level command now wins and items are returned sorted by name.
- **Skill validation**: autofix marked a skill syncable whenever it changed anything, even if another error remained; the fixed skill is now re-validated. Autofix no longer runs on non-UTF-8 skills, which it used to return with U+FFFD in place of every invalid byte.
- **Skip existing commands**: a new command was skipped because a Claude plugin or marketplace shipped one with the same name, though nothing in `~/.claude/commands` would have been overwritten; only the writer's own directory counts now.
- **Dry run**: an MCP target config that does not parse was previewed as "would write" every server while the real run refused; the dry run now reports the same failure.
- **Cursor MCP servers**: one entry with an unexpected value (a numeric `env` value, `"enabled": "false"`) failed the whole MCP phase, and SSE servers were synced as `http`; such entries are now skipped with a warning.
- **Cursor frontmatter**: a description that continued on an indented second line was cut to its first line; continuation lines are now joined.
- **Skill modules**: a nested directory with its own `SKILL.md` was copied into the parent skill without that `SKILL.md`; nested skills are no longer treated as modules.
- **Copilot agents**: an agent named `team/reviewer` was written to a nested file the Copilot reader never reads, so it was rewritten on every sync; it is now written flat as `team-reviewer.agent.md`.
- **Codex, Cursor and Copilot readers**: one unreadable file in a commands, agents, rules, skills or instructions directory failed that phase on every run; it is now skipped with a warning.
- **Cursor plugin hooks**: `${CLAUDE_PLUGIN_ROOT}` was pasted unquoted into hook commands, so a path with shell syntax changed the command, and `$CLAUDE_PLUGIN_ROOT_DIR` became `<root>_DIR`; only the whole variable is replaced now, and only with a path free of shell syntax (otherwise the variable is kept and a warning shown).
- **Codex source**: a stray `.md` file in the shared `~/.agents/skills` (a README) was synced everywhere as a skill, and skills copied there from `~/.codex/skills` by an earlier version were read twice; flat files are now read only from `~/.codex/skills`, and the older copy is skipped.
- **Cursor plugin skills**: the plugin manifest was read from the real `~/.claude` whatever the sync source was, written through a symlinked plugin directory, and never updated after a plugin version bump; it now comes from the skill's own plugin cache, symlinked directories are left alone, and skrills-made manifests are refreshed.
- **Concurrent syncs**: two syncs into the same target (CLI and MCP server) could interleave their read-modify-write of config files and lose an update; a sync now waits for a lock on `<target>/.skrills-sync.lock`.
- **Plugin mirror memory**: a full mirror held every cached plugin's files in memory with only a per-file cap; reading now stops at 512 MiB in total (and that run removes nothing from the mirror), and a file that grows during the read is not read whole.
- **Claude hooks**: hooks synced into Claude were written as `~/.claude/hooks/*.md` files that Claude Code never runs, yet counted as written; they are now reported as skipped with a pointer to the `hooks` section of `settings.json`.
- **Copilot MCP servers**: remote (http) servers were dropped when writing to and reading from Copilot CLI's `mcp-config.json`; they are now synced with `type`, `url` and `headers`, and entries get `type: "local"`/`"http"` and a `tools` list (`["*"]` unless one is already set).
- **Copilot instructions**: sync warned that `~/.copilot/instructions` was only a staging location, though Copilot CLI loads it; the warning is gone, Cursor `globs` are now written as Copilot's `applyTo` (and read back as `globs`), and nested instruction files are read.

#### MCP server

- **Dashboard API**: `/api/mcp-servers` printed passwords and tokens carried in URL-shaped args (`postgresql://user:pw@db/app`, `--url=https://h/mcp?token=...`); user info and query values in such args are now replaced with `[redacted]`.
- **Cold-window dashboard**: `skrills cold-window --browser` answered any `Host`, so a page on a DNS-rebound domain could read the alert stream; requests naming anything other than localhost, 127.0.0.1 or ::1 now get 403.
- **HTTP server**: an empty `--auth-token`/`auth_token = ""` turned auth on and then accepted an `Authorization: Bearer` header with no token; the server now refuses to start with a blank token.
- **HTTP server**: browser MCP clients failed CORS preflight because `Mcp-Session-Id`, `Mcp-Protocol-Version`, `Last-Event-ID` and DELETE were not allowed and `mcp-session-id` was not readable; they are now allowed and exposed.
- **HTTP server**: `Authorization: bearer <token>` (lower-case scheme) was refused with 401; the scheme is now matched case-insensitively.
- **MCP skill-diff**: a very large `context_lines` panicked the request in debug builds and dropped the last changed line in release; it is now capped at 1000 lines.
- **MCP create-skill**: project analysis, session-history parsing and `git log` polling ran on the async worker and stalled other requests on the same server; they now run off the worker.
- **HTTP server**: every MCP `initialize` rebuilt the skill cache and subagent registry and sessions were unbounded; sessions now share one service, and a 65th live session is refused.
- **MCP resources**: Copilot skills (`~/.copilot/skills` and the XDG `copilot/skills` path) were missing from `list_resources`/`read_resource` though the dashboard listed them; they are now served.
- **MCP validate-skills**: `target: "copilot"` and `target: "all"`, both listed in the tool schema, failed with "Unknown target"; they now validate and report a Copilot-valid count.
- **MCP tool schemas**: clients that enforce `additionalProperties: false` could not send `include_marketplace`, `skip_existing_commands`, `max_languages` or create-skill's `project_dir`, and every sync tool advertised a `force` flag nothing read; each schema now lists exactly the arguments its tool reads.
- **MCP errors**: missing or invalid tool arguments were reported as internal errors (-32603), inviting clients to retry; they are now invalid-params errors (-32602).
- **Browser dashboard**: with `--auth-token` the dashboard page itself answered 401 and its API calls carried no token; the page and its assets now load without a token, and the page sends the token (from `/#token=...` or a prompt) on every API call, so the MCP Servers panel works when auth is on.
- **recommend-skills**: two skills that link each other listed the other twice (as dependency and dependent), inflating `total_found` and using a `limit` slot; it is now listed once, as a dependency.
- **MCP fetch-pdf**: a PDF already in the cache could not be fetched when offline or when Unpaywall was down, because Unpaywall was asked first; the cache is now checked first (the result's `url` is null for a cached PDF).
- **MCP sync tools**: `sync-commands`, `sync-mcp-servers`, `sync-preferences`, `sync-status` and `sync-all` accepted the same tool as source and target and rewrote its configuration through its own writer; they now refuse that, and unknown platform names, as invalid params.
- **TLS auto certificates**: an interrupted or concurrent start could leave `~/.skrills/tls` with a certificate and a key from different generations, which was then reused and failed every handshake; the files are now written atomically and a mismatched pair is regenerated.
- **MCP recommend-skills-smart**: every call re-parsed all Claude and Codex session history; parsed usage is now reused for 60 seconds and capped at 200 000 events.
- **MCP search-papers**: a repeated source name made one outbound request per repetition; sources are now queried once each. Results were concatenated by source and then truncated, so the default limit returned arXiv papers only; results now alternate between sources, DOI-less duplicates are dropped by title, and per-source errors appear in the text.
- **MCP fetch-pdf**: a PDF URL whose host name resolved to a private or link-local address (for example 169.254.169.254) was fetched, and several reserved literal ranges passed the check; resolved addresses are now filtered and those ranges refused.
- **Research tools**: knowledge-graph nodes and the PDF cache grew without limit from MCP input; node fields are now limited to 16 KiB, the graph to 100 000 nodes, and the PDF cache to 2 GiB with the oldest PDFs removed first.
- **MCP skill-diff**: every call walked and hashed every skill root; the listing is now reused for the cache TTL and refreshed when the file watcher sees a change.
- **Skill discovery cache**: on a home without `~/.codex` the discovery snapshot was never saved, and concurrent processes could leave half-written JSON; the directory is now created and the file replaced atomically.
- **Dashboard API**: `/api/rules` served hook commands verbatim, including tokens such as `Authorization: Bearer ...` or `API_KEY=...`; credentials in commands are now replaced with `[redacted]`.
- **MCP resources**: Cursor skills in `~/.cursor/skills` were not served by `list_resources`/`read_resource`; that root is now discovered.
- **MCP gateway**: `describe-mcp-tool` and `list-mcp-tools` did not know the subagent tools `list_tools` advertises, and snake_case names were not found; both now cover every advertised tool under either spelling.
- **Skill tracing**: `enable-skill-trace` with the include flags wrote markers and `.md.bak` files into the Claude plugin cache, marketplace checkouts and the Codex mirror; those trees are no longer instrumented, and the tool no longer offers `include_cache`, `include_marketplace` or `include_mirror`.
- **Build**: the HTTP transport did not compile on FreeBSD and other Unix targets besides macOS and Linux; they now build and open the browser with `xdg-open`.
- **MCP resources**: reading a deleted skill returned an internal error and contents were labelled `text`; a missing resource is now `resource_not_found` (-32002), a malformed URI invalid params, and contents are `text/markdown`.
- **MCP create-skill**: with the default `method: "both"` and no working LLM, the call reported success when GitHub search found something, though no skill was created; it now reports an error and lists the GitHub results.
- **MCP suggest-new-skills**: short patterns matched inside unrelated words (`ci` in "precision", `auth` in "author", `doc` in "docker"), so gaps were flagged for almost every project, and suggested names such as `c++-best-practices` were not valid folder names; matching is now by whole word and names are slugged (`cpp-best-practices`).
- **MCP list-mcp-tools**: the listing order changed between runs and twelve tools had no category (`category=trace` returned two of four trace tools; research tools could not be filtered); the listing is now sorted and every tool has a category, research tools under `research`.
- **MCP recommend-skills**: `limit: 0` silently returned nothing and any size was accepted; the limit is now clamped to 1..100 as in the other tools.
- **Browser dashboard**: the skill list never refreshed despite "Auto-refreshes every 30s" (only the count did); the loaded list is now reloaded on each refresh.
- **HTTP server**: when the requested port was taken the server silently moved to another, and a port-qualified `--allowed-hosts` entry then refused every request with 403; with such an entry the server now fails to start instead.
- **Skill previews**: a file exactly at the preview size was shown with a trailing "…" and a cut through a multi-byte character showed a replacement character; previews now end on a whole character and mark only real truncation.
- **MCP resources**: `SKRILLS_EXPOSE_AGENTS=0` (or `1`, `FALSE`) was silently ignored and AGENTS.md stayed exposed; 0/1, true/false, yes/no and on/off are now accepted in any case, and other values are logged.
- **Dashboard metrics**: `sync-from-copilot` and `sync-from-cursor` were recorded as pushes; they are now recorded as pulls.
- **MCP skill-diff**: a skill installed only in `~/.agents/skills` was not found for Codex, and the description promised a frontmatter comparison the tool never made; Codex copies there are now compared and the description says what the tool returns.
- **Build**: a default `skrills-server` build compiled the ratatui/crossterm dashboard crate though no server code uses it; it is now a test-only dependency.
- **Build**: `skrills-server` library consumers compiled `tracing-subscriber`, which only its tests use; it is now a test-only dependency.

#### CLI, state and snapshots

- **mirror**: `--dry-run` copied agents and rewrote AGENTS.md; it now writes nothing.
- **mirror**: refreshed `AGENTS.md` in the current directory, rewriting whatever repository it ran from; it now refreshes `~/.codex/AGENTS.md`.
- **pre-commit-validate**: `--staged` validated every staged `.md` (README, CHANGELOG) as a skill and failed; it now checks only `SKILL.md` and `*.skill` files.
- **metrics**: text output named every top hub and the largest skill "SKILL.md"; it now prints the skill folder name.
- **validate / sync-all**: `--autofix` never ran for `--target copilot` / `--to copilot`; it now repairs skills that fail the selected target.
- **analyze / recommend / search-skills**: `--suggestions false`, `--include-quality false` and `--include-description false` were parse errors; they now turn the option off. The bare flag still means true; `search-skills --include-description <query>` now reads `<query>` as the flag's value, so put the flag after the query.
- **cold-window**: `--skill-dir` help named `SKRILLS_EXTRA_SKILL_DIRS`, which nothing reads; it now names `SKRILLS_SKILL_DIR`.
- **cold-window**: `--tick-rate-ms` only set the first tick in adaptive mode; it now sets the adaptive base for every tick.
- **sync-pull**: `--source` printed "not yet implemented" and exited 0 with `skills_pulled: 0`; it now exits with an error. The suggested `skrills sync --from claude` was not a valid command; the hint now says `skrills sync`.
- **skill-profile / skill-usage-report**: text labelled all-time counts as "last N days"; it now says the counts are all-time and the period selects skills by last use.
- **skill-score**: a name matched any substring of the absolute path (`test` scored every skill under `test-projects/`); it now needs an exact skill name and reports an ambiguous one. Unreadable skills are now logged instead of dropped.
- **skill-score**: a skill without frontmatter received 25/25 for validation; the validation score now reflects `validate --target all`.
- **skill-import**: importing a skill directory copied only `SKILL.md`, dropping `scripts/`, `references/` and `assets/`; it now copies the tree, skipping hidden entries and symlinks.
- **skill-diff**: the diff compared line N to line N, so one inserted line marked every later line changed; it now prints a line-matched unified diff with `@@` hunk headers. A very large `-C` no longer overflows.
- **skill-deprecate / skill-import --force**: rewrote `SKILL.md` in place, so a crash mid-write could truncate it; the file is now replaced through a synced temp file and a rename.
- **cert install**: checked only the PEM header text and could pair a new certificate with the old key; it now parses the certificate and refuses a key (given with `--key`, or already installed) that does not match its public key, and refuses a certificate with no `--key` when no key is installed. A PKCS#1 RSA key must be converted to PKCS#8 first.
- **SKRILLS_SKILL_DIR**: an empty value or a stray `:` added the working directory as a skill root and hid the default roots; empty entries are now ignored, and the value is split with the platform separator (`;` on Windows).
- **skrills-state**: `check_connectivity` returned `Offline` when no probe target resolved; it now returns `Unknown`, as documented.
- **cold-window snapshot**: research findings from GitHub serialized their channel as `"git-hub"` and a producer sending `"github"` was rejected; the channel is now `"github"`, and `"git-hub"` is still read.
- **CLI help**: `skill-usage-report --format` advertised html/markdown, `skill-diff --format` advertised summary, `skill-import` advertised URL and git sources, `tui` advertised pin management, and `multi-cli-agent --backend` had no help; the help now describes what each accepts.
- **skill-import**: a local directory named `*.git` was refused as a git URL; an existing path is now imported.
- **skill-deprecate**: a skill with `deprecated: false` was reported as already deprecated; the flag is now set to true. `skill-catalog` reads the flag the same way.
- **cold-window**: `--research-rate` help now says the quota resets on every run.
- **skill-rollback**: outside a git repository it failed with a git error; it now reports no history. A rollback no longer stages the restored file.
- **skill-import**: exited 0 when the skill already existed and nothing was imported; it now exits with an error. A frontmatter `name` that differs from the target folder is now rewritten to the folder name.
- **skill-diff**: never searched `~/.cursor/skills`; it now does. A skill found once reports `identical: null` instead of `true`, sources print as `claude`/`extra0` instead of Rust debug names, and the token difference uses the same estimate as `analyze`.
- **skill-catalog**: `--source claude` hid Claude plugin skills (discovered as `cache`/`marketplace`); they are now included. `--category` help says it is not implemented.
- **cert install**: a failed certificate rename left the new key beside the old certificate; the old key is now restored and temp files removed. A special file such as `/dev/zero` passed the size check and was read without limit; reads are now capped at 10 MB.
- **cert status**: an unreadable key aborted text output midway and printed `null` in JSON; both now report it as unreadable (`key_error` in JSON).
- **serve**: `--auth-token`, TLS, CORS and allowed-hosts flags were ignored without `--http`, and `--trace-wire` with it, with no notice; a warning now names them.
- **serve**: `--http --tls-auto --watch` generated certificates before refusing `--watch`; it now refuses first.
- **serve**: `SKRILLS_AUTO_PERSIST` saved analytics only after a clean stdio exit; it now saves on every exit, HTTP included.
- **serve --list-tools**: omitted the MCP gateway tools the server serves; they are now listed (subagent tools still are not).
- **skrills (no subcommand)**: ignored `SKRILLS_HTTP`, `SKRILLS_TRACE_WIRE` and the TLS/auth variables that `skrills serve` reads; it now reads them the same way.
- **serve --auth-token**: help now warns that a token in argv is visible through `ps` and never prints the environment value.
- **doctor**: exited 0 when it reported problems; it now exits non-zero when any `!` line is printed.
- **dashboard**: ignored `SKRILLS_SKILL_DIR`; it now merges it with `--skill-dir` like other commands.
- **multi-cli-agent --dry-run**: printed only the backend name; it now prints the full command, including `--yolo` for Codex.
- **search-skills**: `--threshold 5` or `nan` was accepted and returned everything; values outside 0.0 to 1.0 are now rejected.
- **recommend-skills-smart**: `--auto-persist` saved nothing under `--include-usage false`, and a failure to build analytics failed the recommendation; it now saves in both cases and a failure is a warning.
- **config file**: a parse error in `~/.skrills/config.toml` was printed twice; it is now printed once.
- **skills manifest / SKRILLS_CACHE_TTL_MS**: a manifest that failed to parse, or a TTL such as `5m`, fell back to defaults with no message; a warning now names the file or variable and the error.
- **skrills-state**: of two validation-cache writes in the same second, `get_latest_for_path` could return either; it now returns the later one.
- **skrills-state**: the crate description and README described pin persistence removed in 0.3.1; `tempfile` is no longer a runtime dependency.
- **HOME**: an empty or relative `HOME` put skill, cache and mirror paths under the working directory; it is now treated as unset. `cert`, `skill-import` and `skill-profile` use the same home lookup as other commands.
- **cold-window snapshot**: an infinite alert threshold serialized as `null` and could not be read back; `AlertBand::new` now rejects it.
- **sync-all**: stopped at the first failing target, leaving later ones unsynced; it now tries every target and reports which succeeded and which failed.
- **skill-profile / skill-usage-report**: `last_used` was always null; it now carries the recorded last use (RFC 3339), and "Generated:" prints an RFC 3339 time instead of epoch seconds.
- **CLI**: `--include-description`, `--include-quality` and `--suggestions` can be turned off with `=false` (e.g. `--include-description=false`); the bare flag still turns them on and no longer takes the next word as its value.
- **cold-window**: `--alert-budget` must be at least 3, the smallest ceiling that holds ordered advisory, caution and warning tiers.

#### Discovery and intelligence

- **discovery**: skills under `~/.cursor/skills` were all named `SKILL` and every one but the first was dropped as a duplicate, with README and reference files listed as skills; each SKILL.md now gets its own name, and only `.cursor/rules` roots treat `.md`/`.mdc` files as rules, named by their path so `a/x.mdc` and `b/x.mdc` no longer collide.
- **discovery**: a skill directory named `build`, `dist`, `target` or `vendor` was skipped; it is now discovered when it holds a SKILL.md directly.
- **discovery**: a scan root that sat anywhere under a directory named `agents` listed every markdown file as an agent; only an `agents` directory inside the root counts now.
- **discovery**: the default roots now include `~/.cursor/skills`, where skill sync writes for Cursor.
- **usage analytics**: Codex session files were read whole with no size or count limit, and one invalid UTF-8 byte dropped the whole session silently; they are now streamed with the same caps as Claude sessions, a bad line costs only that line, and skipped files are logged as warnings.
- **recommendations**: a recommendation score could exceed the documented 0-10 range without limit (one co-used skill with a large count outweighed every other signal); each score component is now capped and the total is held to 10.
- **recommendations**: usage recorded under a SKILL.md path or a `plugin:name` reference was never matched to the `skill://` URI being scored, so recency and co-use signals rarely applied; usage is now keyed by skill name on both sides.
- **usage analytics**: prompt-keyword confidence measured how common a word was, not how specific it was to a skill, and every word seen once was stored in `analytics_cache.json`; confidence is now the share of the word's uses by its top skill, one-off words are dropped and at most 500 entries are kept.
- **create-skill**: LLM generation through the Claude CLI ran with the user's tool permissions and MCP servers, so a crafted skill description could trigger allowed tools; it now runs with all tools and MCP servers disabled.
- **create-skill**: any LLM reply containing `---` (for example a refusal with a markdown rule) was saved as a SKILL.md and reported as success; a reply is now accepted only when it opens with frontmatter that has a name and a description, and a fenced example inside the skill no longer truncates it.
- **GitHub skill search**: the minimum-stars filter added a `stars:` qualifier that GitHub code search does not support, so it had no effect; results are now filtered by their repository's star count.
- **usage analytics**: a session timestamp before 1970 wrapped to a date far in the future and read as "just used"; it now counts as unknown.
- **usage analytics**: one invalid UTF-8 byte in `history.jsonl` lost the whole command history; history files are now streamed and a bad line costs only that line.
- **usage analytics**: a FIFO or a `/dev/zero` link named `*.jsonl` in a session directory could hang `serve` at startup; non-regular files are now skipped.
- **usage analytics**: subagent logs that the parser never reads invalidated the analytics cache, so it was rebuilt on almost every start; only session files the parsers read count now.
- **usage analytics**: the analytics cache, which holds keywords from prompts, was written world-readable; it is now created readable only by its owner, a cache dated in the future is treated as stale, and `command_history` is filled.
- **usage analytics**: past the 20,000-file limit, Claude sessions were dropped in directory order; the newest sessions are now kept.
- **usage analytics**: Codex `skills-history.json` entries without a session id were all treated as one session, so every pair of skills in the file looked co-used; each entry is now its own session.
- **create-skill**: a skill name or description containing `{description}` or `{context_section}` was expanded inside the generation prompt; values are now inserted literally.
- **create-skill**: `generate_skill_sync` panicked when called inside a tokio runtime; it now returns an error.
- **GitHub skill search**: the language filter accepted extra qualifiers (`rust repo:x/y`), code-search qualifiers such as `symbol:` and `content:` were not stripped from queries, a 429 response gave no rate-limit hint, and a query emptied by sanitizing searched every SKILL.md; the language is now validated, those qualifiers are stripped, 429 maps to the rate-limit message, an empty query is refused, and `per_page` is kept within 1-100.
- **cold-window hints**: a hint with an infinite or huge impact could score NaN and rank first; such inputs are now bounded and a non-finite score counts as 0.
- **discovery**: SKILL.md files over 1 MiB were read in full twice per rescan; they are now change-tracked by their stat and their frontmatter is read from the first 64 KiB.
- **discovery**: skill, agent and rule order (and which duplicate was kept) depended on the filesystem's directory order; walks are now sorted by name.
- **discovery**: hook rule files that could not be read vanished silently, their description was the whole matching line and any line containing "run" counted as the command; unreadable files are now logged, and `description`, `command` and `enabled` are read from the file's keys.
- **discovery**: `extra<N>` source labels could not be parsed back, AGENTS.md references kept one- and two-character tokens, `requires_tools` reported an agent that inherits all tools as needing none, an agent without a `name` was named `agents/rev.md` instead of `rev`, `RuleCategory` serialized as `precommit` while its manifest identifier is `pre-commit`, and `--skill-dir` paths under `~/.claude/plugins/cache` or `~/.codex/skills-mirror` were labelled Claude or Codex; each now behaves as documented (old `precommit` values still read).
- **project context**: a `pyproject.toml` without dependency tables hid `requirements.txt`; a README without a leading heading had no description; a nonexistent project path returned an empty profile; Flask, Rocket, Koa and NestJS apps were not classed as services while any library with a Dockerfile was; equal-count languages were all marked primary; a workspace dependency table turned dev-dependencies into normal ones; `feat!:` commit prefixes were kept as keywords; and `git log` run from a git hook read the hook's repository. Each is fixed.
- **recommendations**: explanation text said "Used 691 months ago" and "Found 1 recommendations", treated millisecond timestamps as recent forever and printed "Matches:" for both project and prompt matches; it now uses years, the singular, reads milliseconds, and labels the two match kinds differently. A quality score outside 0-1 or given twice no longer distorts the total, and multi-word queries are matched against description phrases.
- **create-skill**: `CreationMethod` serialized as `GitHubSearch` while every other interface uses `github`; it now serializes as `github`, `llm`, `empirical` and `both` (old names still read). An exported `CODEX_HOME` no longer makes skill generation pick the Codex CLI, and CLI availability no longer depends on a `which` binary.

#### Subagents and analysis

- **analyze/resolve**: a dependency reached first through an optional edge and later through a required one was reported optional; it and what it requires are now reported required.
- **analyze/cold-window alerts**: a fired token alert disappeared for one tick whenever the total dipped into the hysteresis zone (between the clear level and the threshold) and re-fired on the next; it now stays active until the total drops to the clear level.
- **analyze/cold-window alerts**: `LayeredAlertPolicy::new` with a ceiling at or below 50K kept the 20K/50K tiers, so any builder panicked with `MisorderedTiers`; `new` now scales the tiers like `for_budget`, and builder panics state the violated rule.
- **analyze/cold-window engine**: an engine built with `with_strategies` alerted on budget breach but never engaged the kill switch; it now takes the ceiling from the alert policy.
- **analyze/deps**: links inside fenced code blocks were reported as dependencies (and as missing files), and URLs kept trailing backticks, quotes and punctuation; fenced code is now skipped and those tails are trimmed.
- **analyze/tokens**: a file starting with `---` that never closed was counted entirely as frontmatter (prose and code 0); such an opener is now counted as body text.
- **subagents**: with no `backend` argument the CLI run spawned `codex` whatever the client or `cli_binary` setting; it now uses `cli_binary` from subagents.toml, then a configured `default_backend`, then the detected client (default `claude`).
- **subagents**: an API-mode agent run ignored the call's `backend` and always sent the adapter's default model; `backend` now outranks the agent's model, and an agent `model` that is a full model id (not `opus`/`sonnet`/`haiku`/`inherit`) is sent to the API.
- **subagents**: `timeout_ms` on an API-mode run was ignored (the call returned after about 31 s with the run still Running); it is now rejected with an error naming the API timeout variables.
- **subagents**: `output_schema` on a CLI run was silently ignored and any `template_id` was accepted; a CLI run with `output_schema` and a `template_id` the selected backend does not list are now errors.
- **subagents**: an unknown run id gave `"status": null` from get-run-status, a tool error from get-run-events and a protocol error from stop-run; all three now return the same tool error with `error` and `run_id`.
- **subagents**: a `stop-run` sent right after a CLI run started could arrive before the child was spawned; the run showed Canceled while the child then ran to completion with its tools. The child is now not spawned for a run already stopped.
- **subagents**: a CLI child writing a long line without a newline grew server memory without bound and stored the whole line in a stream event; each line is now held and stored up to 16 KiB, with `truncated: true` on a cut line.
- **subagents**: a CLI run whose child left a background process holding stdout open was recorded as `Failed: CLI timed out` although the child had finished; the run now ends when the child exits, after reading remaining output for up to 2 s.
- **subagents**: stopping or timing out a CLI run killed only the `claude`/`codex` process and left the shells, builds and MCP servers it started running; on unix the child now runs in its own process group and the whole group is killed.
- **subagents**: runs.json was written pretty-printed and renamed into place without an fsync, so a crash could leave an empty file that was then moved aside with all history; it is now written compact and synced before and after the rename.
- **subagents**: there was no limit on concurrent CLI subagent runs; a service now runs at most 8 CLI children at once and refuses further runs with an error until one ends.
- **subagents**: stopping an API-mode run only changed its status; the HTTP request kept running and its reply was still appended to the canceled run. Stop now aborts the request, a finished run takes no more events, and the reply is no longer split into one stream event per word.
- **subagents**: API-mode runs defaulted to model ids the endpoints reject (`claude-code` for Claude, `gpt-5-codex`, which Chat Completions does not serve, for Codex); the defaults are now `claude-opus-5-5` and `gpt-6-astra`. `SKRILLS_CLAUDE_MODEL` / `SKRILLS_CODEX_MODEL` still override them.
- **subagents**: Codex API runs with `output_schema` sent `strict` beside `type` instead of inside `json_schema`, and `tracing` sent a boolean metadata value; `strict` is no longer sent and the metadata value is the string `"true"`.
- **subagents**: under Cursor the default CLI binary was `cursor`, the editor launcher, so a run opened the editor and was recorded as succeeded; a Cursor client now falls back to `claude`, and an exported `CODEX_HOME` no longer outranks Cursor's session markers.
- **subagents**: `claude` and `codex` CLI runs got the prompt as a command-line argument, visible to other local users in `ps`, failing past about 128 KiB, and written to the debug log; the prompt now goes to the child on stdin and the log records only its length.
- **analyze/cold-window alerts**: a token budget of 3 or 4 put the advisory threshold at 0, so an Advisory fired at 0 tokens; the advisory threshold is now at least 1. An alert's `fired_at_ms` now reports when it first fired instead of the current tick.
- **analyze/cold-window skills**: a skill directory reached through several symlinks was attributed to whichever name readdir returned first, so its source id could change between ticks; entries are now walked in name order. A directory holding both `SKILL.md` and `skill.md` is counted once.
- **analyze/cold-window skills**: every tick re-read and re-tokenized every SKILL.md; unchanged files now reuse their count, and a SKILL.md over 1 MiB is reported instead of read.
- **analyze/graph**: `resolve` listed a skill as its own dependency when it sat on a cycle, `has_cycle` reported a cycle for a skill that only depends on one, and a very long dependency chain could overflow the stack; all three are fixed.
- **analyze/tokens**: token totals counted a newline after the last line even when the file had none, and rounded each section up before adding them, overstating the total by up to 3; `~~~` and four-backtick fences were read as prose. Totals are now rounded once, count only real newlines, and every scanner uses one CommonMark fence rule.
- **analyze/optimize**: suggestions flagged YAML comments and long `description:` values as duplicate headings and long paragraphs, counted `TODOS` and TODOs inside code examples, treated `# Usage` and `## Usage` as duplicates, and hid the unused-directory suggestion when any external URL contained the directory name; all fixed. `Suggestion` gains a `kind` field so the quality score no longer depends on message wording.
- **analyze/deps**: sizing a skill's `scripts/` or `assets/` walked every file below it (a vendored `node_modules` meant tens of thousands of stats); the walk now stops 8 levels deep or after 10 000 entries, with a `walk_limit_reached` warning.
- **analyze/deps**: any `.md` link or path containing "skill" was classed as a skill dependency and never checked for existence, so a broken `[guide](guide.md)` went unreported; only `SKILL.md` links are skills now, and every local link is checked.
- **analyze/resolve**: one skill with a missing dependency made `GraphBuilder::build` fail for every skill; the graph now builds and only resolutions that reach the broken skill fail. The trait resolver no longer resolves a skill twice when it is named both as `source:name` and `name`, and `InMemoryRegistry::list_skills` is sorted.
- **subagents**: `get-run-history` accepted any `limit` (silently using 20 for bad values and up to 500 runs for large ones) and copied every run to answer; `limit` must now be 1 to 50 and only the returned runs are copied. `get-run-events` takes a `limit` (default and maximum 1000) and `has_more` now says whether more events follow.
- **subagents**: an API response body was read whole whatever its size and stored in runs.json; at most 4 MiB is now read and a cut reply is marked truncated. A run whose start could not be saved stayed Running forever; it is now marked Failed.
- **subagents**: a malformed `SKRILLS_CODEX_BASE_URL` or `SKRILLS_CLAUDE_BASE_URL` made the adapter drop the API key, model and timeout and report "API key not set"; runs now fail with the base URL error and the other settings are kept.
- **subagents**: a per-call `cli_binary` such as `claude.sh` or `CODEX.bat` passed the allowlist because only the file stem was compared; only the exact names `claude` and `codex` (and their `.exe` on Windows) are accepted now.
- **subagents**: CLI output over 1 MiB was cut without notice; the completion event now carries `truncated` and the run status says the output was truncated.
- **subagents**: an agent `model` such as `GPT-4` or `Claude-3-Opus` fell through to the default backend (case-sensitive substring match), while ids merely containing `o1`/`o3` went to Codex; models are now matched case-insensitively by known prefix.
- **subagents**: under Claude Code, an agent defined in both `~/.codex/agents` and `~/.claude/agents` resolved to the Codex copy; the running client's own copy now wins, and `list-agents` is ordered by name.
- **subagents**: a timeout variable set to `0` (`SKRILLS_CLI_TIMEOUT_MS`, `SKRILLS_*_TIMEOUT_MS`) timed every run out at once; 0 is now rejected with a warning and the default used. A `cli_binary` with surrounding spaces is trimmed instead of rejected.
- **subagents**: the unimplemented `download-transcript-secure` tool was advertised to models, which then called a tool that always errors; it is no longer listed. The `run-subagent-async` description now says what it does.
- **subagents**: CLI children got a cleared environment that dropped `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_AUTH_TOKEN`, the Bedrock/Vertex settings and `CODEX_API_KEY`, so users signed in that way got an unauthenticated child; those variables are now passed through.

#### Dashboard, metrics, research, validation

- **validate**: autofix panicked on descriptions over 500 bytes that contained multibyte characters; it now truncates on a character boundary and counts characters, as Codex and the Agent Skills spec do.
- **validate**: autofix rewrote the whole frontmatter from `name` and `description`, deleting `version`, `depends`, `allowed-tools` and other keys; it now replaces only those two entries and keeps the rest, including comments.
- **validate**: generated frontmatter is now quoted whenever plain YAML would misread it (leading `-`, `*`, `[`, quotes, backslashes, `true`, numbers), so autofix no longer writes files that fail to parse.
- **validate**: Codex and Copilot name/description limits are counted in characters instead of UTF-8 bytes, so non-Latin text within the limit is no longer rejected.
- **validate**: validating all targets dropped Copilot issues whose message matched a Codex issue, so a skill could be Copilot-invalid with no Copilot issue listed; issues are now merged per target and severity.
- **validate**: the summary for `--target both` counted every skill as Copilot-valid although Copilot was never checked; unchecked targets are no longer counted.
- **validate**: validating a directory that does not exist returned "0 skills, all valid"; it now returns an error.
- **validate**: one unreadable or non-UTF-8 `SKILL.md` aborted the whole directory run; that file is now reported as an error and the others are still validated.
- **validate**: frontmatter after a UTF-8 byte-order mark is now recognized.
- **validate**: the unused `watch` feature of `skrills-validate` is no longer on by default, so consumers stop compiling `notify` and `libc` for it.
- **metrics**: one NULL in a nullable column (check lists, duration, sync status) made validation summary/history, sync history and top skills fail, which left the dashboard showing every skill unvalidated; NULLs now read as empty or zero.
- **metrics**: the sync summary counted only literal `success` and `failed`; legacy `complete` rows now count as successes, unknown statuses as failures, and in-progress syncs are left out of the success rate.
- **tome**: old-style arXiv ids such as `hep-th/9901001v1` lost their archive and collided across archives; ids now keep the archive and drop the version suffix, so versions of one paper deduplicate.
- **tome**: re-tracking a paper without a DOI, year or citation count erased the stored values; they are now kept. `add_citation` reports whether the edge was stored.
- **dashboard**: skill rows always showed 0 invocations and `[--]` because the dashboard queried metrics by its shortened display name while the server records them under the discovery name; rows now read the recorded metrics.
- **dashboard**: after a terminal read error the dashboard waited forever for input (Ctrl+C included, since raw mode turns it into a key); it now exits.
- **cold-window**: the collapsed research pane never showed the "N new" badge because the live TUI did not feed it snapshots; it now does.
- **cold-window**: hint pins were never loaded from or saved to `~/.skrills/cold-window-pins.json` by the live TUI; they now persist across restarts.
- **cold-window**: `d` and `P` acted on the top row instead of the selected one, so a second `P` unpinned the first hint; they now act on the selected row.
- **cold-window**: an unreadable pin file was silently replaced on the next pin; it is now logged and kept as `cold-window-pins.json.corrupt`, and saves go through a temporary file.
- **cold-window**: moving the cursor past the bottom of a pane selected rows that were never drawn; the alert, hint and research lists now scroll to keep the selected row visible.
- **dashboard**: an unreadable agent MCP config showed as "no servers" with nothing logged; the failure is now logged and shown in the activity feed.
- **validate**: `validate_all` validated a symlinked `SKILL.md` as well as its target; links are now skipped as documented.
- **validate**: an empty frontmatter block (`---` then `---`) was not recognized, and reported body line numbers ignored blank lines around the fences; both are fixed.
- **validate**: autofix no longer takes a description from inside a fenced code block, reports unterminated frontmatter as an error instead of "no change", backs up `skill.markdown` as `skill.markdown.bak` without overwriting an earlier backup, writes through a symlinked skill instead of replacing the link, lowercases non-ASCII names, and falls back to the directory name when a name has no letters or digits.
- **validate**: Codex and Copilot issues for an over-long or empty name/description cite the line of that key; missing-key issues no longer point at lines 2 and 3.
- **validate**: the Claude check counts line length in characters, counts only line-start fences when looking for an unclosed code block, and names a `SKILL.md` after its directory, as autofix does.
- **validate**: structured dependencies now reject an empty or malformed name and source, and the compact form accepts sources with digits, `-` or `_`.
- **metrics**: a huge baseline window no longer disables baselines; `flush_to_disk` can overwrite an earlier copy and rejects non-UTF-8 paths; invocations with a NULL outcome count as failed; recent events report a corrupt validation row instead of skipping it; retention purges NULL-dated rows inside one transaction; values above `i64::MAX` are stored saturated; a NaN quantile is rejected.
- **tome**: the dispatcher forgets fingerprints whose window has lapsed; the quota file is fsynced before rename; cache TTLs are capped so a huge TTL no longer stores a never-expiring row, and `purge_expired` removes stale rows; arXiv text is entity-decoded with whitespace collapsed and empty queries are not sent; CrossRef keeps organisational authors; DOIs are path-encoded; a zero limit sends no request; knowledge-graph search orders mixed timestamp formats by time; resolving a TRIZ parameter against itself suggests nothing.
- **dashboard**: hints with NaN scores sort deterministically; a resolved warning that fires again is shown and rings; older snapshots are ignored; a discovery error no longer freezes the other panels; plugin names such as `auth-2fa` are no longer merged with others; activity colours come from the event outcome; metric events written by the server or CLI now appear in the activity feed; the MCP panel says how many servers it could not show.
- **test-utils**: `set_env_var` restores a non-UTF-8 original; the workflow pin check also scans composite actions; the release check skips `## Unreleased` and fenced headings.
- **Validation**: a first line of `----` or `---foo` no longer opens a frontmatter block, matching sync and token counting.

#### Build, scripts and docs

- **docs**: the CLI reference covered about half the subcommands and showed a `cert install --cert` form that does not parse. It now documents every subcommand and `serve` flag, a test fails when one is missing, and the skill-loading MCP tools are no longer shown as CLI commands.
- **docs**: ADR 0004 described independent intelligence-crate versioning that the release gate rejects; it is now marked superseded by lock-step versioning.
- **install**: `install.sh` ran its steps at top level, so a `curl | sh` cut off mid-transfer could run part of an install. All steps now run from `main`, called on the last line.
- **build**: `make ci` reformatted the tree instead of checking it and skipped the script, install and markdown gates; it now runs the same checks as CI. `make demo-multi-cli-agent` ran zero tests (wrong package) and now runs them. Dogfood targets use `gtimeout` when GNU `timeout` is absent and stop with an install hint when neither exists.
- **ci**: markdown lint ran only in the local pre-commit hook; the CI hygiene job now runs it.
- **build**: `make` recipes now run bash with `pipefail`, so a failing command piped into `head` or `grep` fails the target, and the demo checks for "not found" errors assert instead of ending in `|| true`. `make release` checks formatting instead of rewriting files, `make install` installs to cargo's own bin directory instead of `./.cargo/bin`, `make help` lists `cold-window`, `lint-reachability`, `lint-shell` and `verify-publish`, and `make check-deps` reports the tools the pre-commit and dogfood targets need.
- **build**: dogfood scratch files moved from a predictable `/tmp/skrills-dogfood-<pid>` path to a private `mktemp -d` directory under `TMPDIR`.
- **lint**: new `make lint-shell` (shellcheck at warning level) runs in `lint-hygiene` and in CI. The prose lint now matches banned words in any case and inflection (`Leveraging`, `seamlessly`). The module-reachability lint accepts `#[cfg(test)] mod tests;` and `mod r#type;`. `lint-markdown.sh` lints the whole repository when run from a subdirectory.
- **release**: `publish_crates.sh` asks crates.io for the exact version before publishing instead of reading `cargo search` ranking, and no longer sleeps 20 seconds between crates. `verify_publish_order.sh` prints a clear error under bash older than 4 and understands glob workspace members, `publish = []` and `publish.workspace = true`.
- **install**: Ctrl-C during `install.sh`'s download now stops the install (exit 130) instead of continuing against a deleted temp directory.
- **docs**: ADRs 0002, 0005 and 0007, the architecture overview, and the book's overview, cold-window, persistence, sync and MCP pages updated to match the code: skill roots, Codex skill and agent locations, MCP config locations, research quota lifetime, tool counts, and subagent CLI binary selection.
- **build**: the setup demos failed on any machine with the `claude` CLI installed, because they looked for `~/.claude/.mcp.json` while setup registers through `~/.claude.json`; they now accept either, start from a clean registration, and the universal demo checks `~/.agents/skills`. `make demo-cli` stopped at `validate` on a fixture without frontmatter and overwrote the repository's `AGENTS.md` through `sync-agents`; it now runs to the end inside the demo home.

## 0.9.0 - 2026-10-13

- **Feature: MCP sync writes HTTP servers to Codex**: Codex runs streamable-HTTP servers from `config.toml`, but sync skipped every one. An HTTP server is now written with `url`, and its headers go to the keys Codex reads, because Codex expands no `${VAR}` inside a header: `Authorization: Bearer ${VAR}` becomes `bearer_token_env_var`, a header whose whole value is `${VAR}` goes to `env_http_headers`, and literal values to `http_headers`. A server with no `url`, or with a header that only partly references a variable, is skipped and named in the report. Switching an entry between stdio and HTTP drops the other transport's keys.
- **Security: the cold-window browser page runs under the dashboard's CSP**: `skrills cold-window --browser` served its page with an inline script and stylesheet and no Content-Security-Policy. The script and stylesheet are now `/static/cold_window.js` and `/static/cold_window.css`, every response from that router carries the same policy as `serve --http`, and the budget bar and lag notice use stylesheet classes instead of `style=` attributes, which the policy refuses.
- **Security: `[serve] project_roots` limits which projects MCP clients can have analyzed**: when set, the `project_dir` a client passes to recommend-skills-smart, analyze-project-context, suggest-new-skills and create-skill must resolve, after canonicalising, inside one of the listed roots, so `..` and symlinks cannot escape; anything else is refused with invalid params. Unset keeps today's behaviour, with a startup warning on a non-loopback HTTP bind (SA-23).
- **Fix: the installer finishes**: `install.sh` names a client for `skrills setup` (`SKRILLS_CLIENT`, else each of `~/.claude`, `~/.codex`, `~/.copilot`, `~/.cursor` that exists, else claude) instead of failing at its last step, and setup no longer truncates its own binary when `--bin-dir` reaches it through a symlink.
- **Change: run event indices stay stable past 10,000 events**: dropped events are summarized by a leading `events_dropped` event and `get-run-events` reports absolute indices (RT-31).
- **Change: `skrills metrics` and `skrills recommend` share the server's code**: one dependency-graph builder, one metrics computation and one relationship walk, so the CLI and the MCP tools agree (SA-44).
- **Deprecated for removal**: the intelligence behavioural pipeline entry points, `ResolveError::VersionConflict`, `ResolutionResult.success`, `WarningKind::FileReadFailed`, `TomeError::PdfNotFound`, `skrills_server::{manifest, runtime}`, `GatewayConfig`, `ToolUsageStats` and `MCP_GATEWAY_TOOL_NAMES` have no callers and will be removed in a breaking release.
- **Security: `setup --universal` no longer copies all of `~/.claude` into `~/.agent`**: it copied `.credentials.json` and session transcripts along with the skills. It now syncs skill directories only, and no longer writes into `~/.codex/skills`. Setup refuses to overwrite an MCP JSON config that does not parse or is not an object, instead of replacing it with an empty one.
- **Security: Secrets stay with the server**: `serve --http` reads `[serve] auth_token` from the config file itself and refuses to start when the file cannot be read. Subagent CLI children get a cleared environment plus an allowlist, a per-call `cli_binary` must be `claude`, `codex` or the configured binary, `runs.json` is written 0600, and `multi-cli-agent --backend claude` no longer falls back to an approval-free `codex --yolo` run. Skill generation sends the GitHub token only to GitHub, ignores the `GITHUB_*_BASE_URL` overrides outside tests, and caps request time and body size. `/api/mcp-servers` redacts env values, secret-looking args and URL credentials.
- **Security: MCP tools and research clients are bounded**: `create-skill` refuses a `target_dir` outside the skill roots and no longer overwrites a `SKILL.md`. `fetch-pdf` requires https, refuses loopback and private targets (redirects included), caps downloads at 100 MiB and checks the `%PDF-` signature. Tome clients cap response bodies at 8 MiB and always use a client with a timeout; knowledge-graph search treats `%` and `_` literally and returns at most 200 nodes. `git log` for project context runs with a 10 s timeout and without the repository's pager, fsmonitor or signature hooks. Agent file paths are checked and quoted before they reach an agent prompt.
- **Security: Third-party GitHub Actions are pinned to commit SHAs**: actions that ran from mutable tags now name a full SHA with the version in a comment, so a moved tag can no longer change what CI and the release job run without a diff here. Dependabot updates the pins weekly, and `crates/test-utils/tests/workflow_pins.rs` fails on an unpinned third-party action.
- **BREAKING: `cold-window` shows only real data unless `--demo` is passed**: the invented token growth, hints and research findings used to fill the panes by default. Without `--demo` the engine shows skills from `--skill-dir` (and `SKRILLS_EXTRA_SKILL_DIRS`) and plugin health. `--alert-budget` must be at least 2; a ceiling at or below 50K now scales `Advisory` and `Caution` to 20% and 50% of it so every tier can still fire. The 20K/50K tiers are no longer raised by the warmed-up baseline, which hid steadily growing token totals. `--browser` fails at once when its port is taken, and `--tick-rate-ms` has a floor, `--no-adaptive` included.
- **BREAKING: Commands fail loudly instead of reporting success**: `validate` exits non-zero when a skill has an error, cannot be read or cannot be fixed. `create-skill` and the other tool-backed commands exit non-zero when the tool reports an error, and `recommend` does on an unknown skill. `serve --http --watch` is refused (there is no watcher to start over HTTP), `skill-catalog --category` is refused (skills have no category field), and `cert renew` refuses to replace a certificate that is not self-signed unless `--force` is given. Tracing goes to stderr, so `--format json` stdout carries only the JSON document, and the first-run prompt appears only on a terminal.
- **Fix: `sync-all --validate` and `--autofix` work**: the `--from` skill tree is validated for the strictest target before any target is written, and any remaining error aborts every target with nothing written. `--autofix` fixes the source first, without a backup, and under `--dry-run` only previews the fix. (#235)
- **Change: Copilot prompt files are written as `.prompt.md`**: files written with the old `.prompts.md` suffix are still read.
- **Change: The dashboard reads the shared metrics database**: `skrills dashboard` opens, and creates if missing, `~/.skrills/metrics.db` instead of a private in-memory one. Opening the persistent database deletes rows older than 30 days. `download-transcript-secure` is reported as unsupported by the Codex adapter.
- **Fix: Sync writes are safe and the report is truthful**: config and skill files are written atomically, user config files keep a one-generation `.skrills-bak`, and MCP servers are merged by name instead of replacing the target's list. CLAUDE.md is written inside a marked block and Cursor `hooks.json` is merged. Writers refuse symlinked destinations and names that collide after sanitizing, write a changed module even when `SKILL.md` is unchanged, and skip invalid UTF-8. A failing phase marks the report failed and the other phases still run; an artifact the target cannot write is reported as skipped; dry runs compare against the target. Plugin artifacts come from the newest cached version and honour `exclude_plugins`. Claude frontmatter and agent `tools`/`isolation` survive a round trip through Cursor. Codex MCP servers and the model are read from and written to `~/.codex/config.toml` (`[mcp_servers.<name>]` tables, top-level `model`) with `toml_edit`, merged by name with comments, key order and unmanaged servers kept; a malformed `config.toml` is refused, and `config.json` is still read when `config.toml` has neither. `skrills_sync::Artifact` is a neutral alias of `Command`. Skill sync no longer mirrors linked files outside the skill directory and skips linked files over 10 MiB.
- **Fix: Setup, uninstall and doctor match what setup writes**: Claude registration uses `claude mcp add --scope user`, and setup detects and removes it. Codex uninstall removes the `[mcp_servers.skrills]` table and the AGENTS.md skill and agent sections, and the `[features]` edit matches the `skills` key exactly. `doctor` checks `~/.codex/config.toml`, where setup registers skrills, reads a legacy `mcp_servers.json` only when one exists, and resolves a bare command on `PATH`. (SB-40)
- **Fix: CLI commands act on every flag**: `analyze`, `metrics`, `recommend`, `skill-catalog`, `skill-deprecate`, `skill-score`, `skill-rollback`, `pre-commit-validate` and `validate` search every default skill root when no `--skill-dir` is given. `skill-deprecate` and `skill-rollback` match a skill by exact name and refuse an ambiguous one; rollback refuses a file with uncommitted changes. `skill-import` writes `<name>/SKILL.md`, so imported skills are discovered. `skill-profile` and `skill-usage-report` read the real analytics cache. `skill-catalog` sorts before `--limit` and no longer panics on multi-byte descriptions. `pre-commit-validate --staged` validates the staged content. `skill-diff` compares every copy of a skill with a real aligned line diff. `metrics --format json` keeps its keys with no skills.
- **Fix: Certificates and serving**: `cert install` checks the certificate and key before writing and installs them together; `cert renew` keeps the old pair until the new one is written; the key is always 0600 and the TLS directory 0700. Auto-generated certificates are regenerated before they expire. `serve` reports the status of the certificate it actually uses, HTTPS falls back to another port like HTTP, and `--open` opens `localhost` for a wildcard bind. Synchronous MCP tools run off the async workers. A missing `uri`, a bad `direction` or an unknown tool name return invalid params (-32602), and `resolve-skill`/`resolve-agent` prefer an exact name. The `sync-from-claude` tool no longer prints to stdout, which corrupted the stdio JSON-RPC stream.
- **Fix: Subagent runs are supervised**: `stop-run` kills the CLI child, CLI runs honour `timeout_ms`, stdout and stderr drain concurrently, and the child gets a null stdin and is killed on drop. CLI runs call `claude --print -- <prompt>` and `codex exec -- <prompt>`. The Codex API URL resolves to `/v1/chat/completions`. A stopped run can no longer flip to Succeeded, runs left Running at shutdown are marked Failed, a corrupt `runs.json` is moved aside, and the store is capped at 500 runs. Claude API runs default to 16000 max tokens (`SKRILLS_CLAUDE_MAX_TOKENS`) and mark a reply cut at the cap as truncated.
- **Fix: Dashboard and cold-window TUI**: no panic on a multi-byte activity line or a zero-sized frame; Ctrl+C quits the dashboard; refreshes run off the UI loop; the terminal and previous panic hook are restored if setup fails. A standing WARNING rings the bell once. Cold-window cadence respects min/max on every branch, symlinked skill and plugin directories are collected, same-named skills in different plugins stay distinct, and token estimates match `skrills analyze`. Hint scoring uses a true half-life, so a hint 14 days old keeps half its score.
- **Fix: Analysis, intelligence and research**: token counts no longer treat a later `---` rule as frontmatter and measure characters, not bytes. Dependency analysis ignores anchors, `mailto:` and titled links and no longer probes outside the skill directory. Recommendation scoring matches technologies on whole tokens and decays the recently-used bonus. Usage analytics read the real Claude Code and Codex session formats, and the cache is rebuilt when stale and written atomically. LLM skill generation times out after 300 s and runs `codex exec`. Project detection handles PEP 508 lines and Cargo workspace tables. Tome reports HTTP 429 as `RateLimited` with the `Retry-After` delay, and the quota dispatcher's writes can no longer land out of order. Metrics tolerate NULL columns and concurrent first opens.
- **Fix: Discovery and validation**: skill hashes cover the whole file, so a same-size edit past the first kilobyte that kept its mtime is no longer missed (every hash changes once on upgrade). Agent discovery skips a file that vanishes mid-scan instead of failing whole, and accepts list-form `tools` and `skills` in frontmatter. Frontmatter closes only on a bare `---` line. `validate --autofix` writes through a temp file and rename, so an interrupted fix cannot truncate `SKILL.md`. A skipped symlink is logged at `info` and an unreadable skill file at `warn`.
- **Docs: Claims checked against the code**: CONTRIBUTING gives the real test commands and environment variables, the README no longer says skill generation needs an API key (it runs the `claude` or `codex` CLI), and the crate graph in `docs/architecture.md` is rebuilt from the manifests. The FAQ and installation guide say where setup registers Codex (`config.toml` only) and that setup installs no hooks. The cold-window guide documents `--demo` and the scaled alert tiers, and the book changelog gains the 0.9.0 section and the 0.8.2 `Esc` change.
- **Security: `Host` is validated on every route, not only `/mcp`**: rmcp's check lived inside the MCP service, and the dashboard and REST API are merged ahead of it, so a page on a rebound domain still read `/api/skills` with a 200. A middleware over the whole router applies the loopback names plus `allowed_hosts` to every request. Allow-list entries are parsed as URI authorities at startup, so an entry with a scheme fails `serve` with the entry named instead of answering 403 forever. A non-loopback bind with no allow-list warns at startup. `cors_origins` also switches on rmcp's `Origin` validation, except for a wildcard, which is warned about with or without a token. CORS moved outside the auth layer, so browser preflights are answered instead of getting 401. `ServeConfig` redacts the token in `Debug`.
- **Fix: Default builds lost the dashboard, the cold-window TUI and `serve --watch`**: the crate split moved the `dashboard` and `watch` cfgs into `skrills` without adding them to its default features, and an import used only behind `watch` made every default build fail under `-D warnings`. The cli defaults now include both, `skrills-server` is depended on with `default-features = false` so the cli list is the only switch, and `make lint` and CI run clippy under all features, default features and no features. The manifests drop eight unused dependencies from `skrills` and seven from `skrills-server`.
- **Fix: Flags that were parsed and ignored now refuse**: `validate --watch` exits non-zero and names the working form. (`sync-all --validate` and `--autofix` refused here first and now work; see above.) `skrills dashboard` in a build without the feature exits non-zero instead of logging an error and returning 0.
- **Fix: One Cursor skill-delivery rule for every caller**: `sync-all`, `sync-to-cursor`, the CLI and `sync-status` used three different rules, so MCP `sync-all` wrote plugin skills twice and a Codex or Copilot source lost its skills. `skrills_sync::skill_delivery` decides once: a Cursor target takes the full mirror, and the orchestrator leaves plugin-origin skills out of the flat copy only when the mirror carries them. The Cursor writer now removes files a plugin no longer ships, replaces a file where a directory is needed and the reverse, clears a dropped executable bit, refuses two plugins that sanitize to one directory, refuses a symlinked plugin directory, skips a manifest-less batch, and prunes only directories carrying a `.skrills-mirror` marker, so hand-made plugins and mirrors from older builds are never deleted. Warnings reach the summary and `--dry-run` lists prune targets. `PluginAsset` loses its stored `hash` field.
- **Fix: Sync skip reasons name the right end**: a source with no plugin-asset reader is reported as `SourceCannotRead`, the CLI summary prints the description, and Codex's `write_agents` is reachable through `write_support`.
- **Fix: The canonical frontmatter splitter kept a trailing `\r`**: the `\r\n---` arm was dead, so a CRLF file reached Cursor with a stray `\r`. The splitter strips it, and the two delegation-only wrappers in sync and discovery are gone.
- **Fix: Six gates could pass without running**: the module-reachability lint checks the file's parent module (a reused stem in another module no longer satisfies it), anchors on the repo root, exits 2 when it scans nothing, and resolves `#[path]` values with directories. The dogfood contracts refuse an `install.sh` slice whose markers drifted, which used to source and run the installer's main body, and no longer enable errexit for the rest of the run. `verify-publish` and `dogfood-precommit` exit 1 from their skip branches (`SKIP_VERIFY_PUBLISH=1` opts out). CI runs the hygiene lints and the pytest suite.
- **Fix: `install.sh` verifies the release `.sha256` sidecar before extracting** (`SKRILLS_SKIP_CHECKSUM=1` opts out), reports "no release asset found" instead of a jq error on a document without `.assets`, and its awk fallback picks the right target from single-line JSON.
- **Fix: Failures name the row, the path and the reason**: a corrupt validation row reports its skill and column through one decoder shared by summary, history and export, and the TUI clears its counts and reports it instead of freezing. Discovery skips only a vanished file while hashing and fails the scan with the path for anything else. A failed `git log` in project analysis warns and records `git_keywords_error` on the profile.
- **Change: The advertised MCP protocol is capped at 2025-11-25**: rmcp 3.4 also advertised 2026-07-28, whose stateless session handling has not been validated here. Older clients negotiate as before, and a client asking for the newer revision falls back.
- **Refactor: Tool definitions go through one builder**: 36 server tools use a shared `tool(..)` helper and `with_raw_output_schema`, with the serialized list unchanged and pinned by a table test. `hex::encode` replaces four hand-rolled hex loops. `tool_ok` and `tool_err` replace the `tool_result(.., bool)` helper in the server and subagents crates, so no call site carries an unlabelled flag. `ensure_codex_skills_feature_enabled` failures are warned and appended to the output at five sites that discarded them, so `skrills sync`, `mirror`, the TUI and the MCP sync tools no longer report success while Codex loads nothing.
- **Fix: A mistyped key in `~/.skrills/config.toml` was dropped silently**: serde ignores unknown keys, so `allowed_host` left every non-loopback client with 403 and no explanation, and `auth-token` started the HTTP server without the token the operator thought they had set. Startup now prints one warning per unknown key, by dotted path (`serve.allowed_host`, or `server` for a misnamed table). The known keys in the same file still apply. Rejecting the file instead would be worse: a parse failure discards the whole config, so one typo would also drop a correctly spelled `auth_token`. `Config` and `ServeConfig` capture unknown keys through a flattened map, so the known-key set cannot drift from the struct fields. Four tests pin the dotted paths, the hyphenated `auth-token` case, and that known keys survive.
- **Security: Close RUSTSEC-2026-0285 in rustls** (severity 5.3, TLS 1.3 handshake messages accepted across encryption level boundaries). `cargo update` moves `rustls` 0.23.40 to 0.23.45 along with 221 other lockfile entries. The same pass clears three unsound-code warnings: `anyhow` 1.0.102 to 1.0.104 (RUSTSEC-2026-0190), `lru` 0.16.4 to 0.18.4 (RUSTSEC-2026-0253), `event-listener` 5.4.1 to 5.4.2 (RUSTSEC-2026-0221), and drops `scc` (RUSTSEC-2026-0205) from the tree. `cargo audit` reports no vulnerabilities. The two unmaintained-crate warnings (`paste` through leptos, `proc-macro-error2`) still fire and are unchanged.
- **Migration: rmcp 1.8 to 3.4**, which follows the 2026-07-28 MCP specification. `rmcp::model::Content` is now `ContentBlock`. `ServerHandler::call_tool` and `read_resource` return `CallToolResponse` and `ReadResourceResponse`, whose other variants carry the new input-required and task flows. Every skrills tool completes in one step, so `call_tool` wraps its result in `CallToolResponse::Complete`. `ListToolsResult` and `ListResourcesResult` are built with `with_all_items`, `Resource` is a flat struct built with `Resource::new(..).with_description(..).with_mime_type(..)` in place of `RawResource`, and `Meta` is `MetaObject`. Older clients are unaffected: a stdio session that initializes with `2024-11-05` or `2025-06-18` negotiates that version, lists 49 tools, and gets tool results with no new-spec fields.
- **Chore: Major dependency upgrades**: `rusqlite` 0.39 to 0.40 (metrics, state, tome), `tower-http` 0.6 to 0.7, `dirs` 6 to 7, `base64` 0.22 to 0.23, `sha2` and `blake2` 0.10 to 0.11, and dev-only `serial_test` 3 to 4 and `mockall` 0.14 to 0.15. Two needed code changes. `digest` 0.11 output no longer implements `LowerHex`, so the four `format!("{:x}", ..)` sites in `crates/discovery`, `crates/sync` and two sync tests build the hex string per byte. A known-answer test (`hash_content_matches_known_sha256_vector`) pins the encoding, because sync compares these strings to decide a file is unchanged. `time` 0.3.55 deprecates `format_description::parse`, replaced by `parse_borrowed::<1>` in `crates/tome/src/knowledge_graph.rs`, which keeps the same grammar and return type. `dirs` 7's one breaking change is `preference_dir` on Windows, which skrills never calls. `reqwest` 0.13.5 still depends on `tower-http` 0.6, so both versions are in the tree until reqwest moves.
- **Testing: Cover the behavior changes in this release that shipped without a test**: Eight script tests under `tests/unit/` and ten Rust tests, plus stronger assertions in two existing tests. Scripts: both slop lints exit 2 when ripgrep itself fails (driven by an `rg` shim on `PATH` that exits 2), and a new `test_lint_module_reachability.py` pins the orphan failure and every declaration form the lint accepts (`mod`, `pub(crate) mod`, `#[path]`, `foo/mod.rs`, crate roots). Rust: `download-transcript-secure` returns a tool-level error; `allowed_hosts` reaches `SKRILLS_ALLOWED_HOSTS` and an existing env var wins; every server tool keeps its title and annotations, with an output schema on exactly `sync-from-claude` and `validate-skills`; the live HTTP transport answers 403 to a foreign `Host` and accepts an allow-listed one; `Box<dyn AgentAdapter>` forwards `read_support` and `write_support`; a Claude-to-Copilot plugin-asset sync reports the target as unsupported; and `crates/cli/tests/subprocess_wait.rs` drives the real binary against a git repository to prove the process can still wait on children. The Cursor integration test now asserts the mirrored scripts, hooks, sources and skill bodies it had only set up, and the traversal-refusal test asserts the warning. Each test was checked by breaking the behavior it guards and watching it fail.
- **Refactor: Move the CLI surface out of `skrills-server` into `skrills`**: `skrills-server` held the clap definitions, 47 subcommand handlers, the dispatcher, the sync TUI, the doctor and the cold-window CLI, while `crates/cli` was a 10-line `main.rs`. `cli/`, `commands/`, `app/dispatcher.rs`, `tui.rs`, `doctor.rs` and `cold_window_cli.rs` are now in `crates/cli`, which becomes a library plus binary. `skrills-server` drops from 30,866 to 20,543 lines and keeps the MCP server and the engine the handlers call. `setup.rs` stays in `skrills-server` because the MCP path calls it too. The cost is a wider public API and a semver commitment: `config`, `setup`, `sync`, `tool_schemas` and `tls_auto` become `pub`, along with `MirrorReport`, `SkillService::has_skill`, `SkillService::get_direct_dependencies` and about a dozen tool methods. `cache`, `handler`, `metrics_types` and `skill_trace` stay crate-private, since nothing outside the crate uses them. The CLI build is no lighter, since `crates/cli` still depends on `skrills-server` for `app`, `sync`, `setup` and `http_transport`.
- **Refactor: One frontmatter splitter, owned by `skrills-validate`**: Three functions split the `---` block and disagreed on line endings: `crates/sync` stripped one line ending and a trailing `\r`, `crates/validate` stripped every leading newline and kept the `\r`, and `crates/discovery` carried two more copies. A skill could be split one way for validation, another for discovery and a third when written to Cursor. `skrills-validate`'s splitter is canonical because it has proptest coverage against arbitrary and binary input, and sync and discovery delegate to it. The sync splitter returns owned strings as a result. Cursor's field parser keeps its own block-scalar handling, since merging the field parsers would change which YAML each side accepts.
- **Fix: `FieldSupport` gains a direction, ending silent no-op syncs**: `FieldSupport` had no read/write axis and the orchestrator consulted the target's copy only, so one flag meant "reads it" for Claude and "receives it" for Cursor. A Cursor-to-Claude plugin-asset sync passed the gate, read nothing, wrote nothing, and reported success. `AgentAdapter` replaces `supported_fields()` with `read_support()` and `write_support()`, both required, so an adapter cannot inherit a direction it does not implement. Claude declares no plugin-asset writer and Cursor no plugin-asset reader. The orchestrator gates the read on the source and the write on the target, and records `SkipReason::SourceCannotRead` or `SkipReason::UnsupportedField` naming the end that could not do it.
- **Fix: Panic on a multibyte GitHub search query**: `sanitize_github_query` searched a `to_lowercase()` copy and sliced the original with that offset. Lowercasing is not byte-length preserving (U+0130 is 2 bytes and lowercases to 3), so `sanitize_github_query("\u{130}repo:")` panicked with an out-of-bounds slice. Reachable from the `search-skills-github` MCP tool and from `create-skill`, which builds the query from an unvalidated description. Replaced with an ASCII case-insensitive search over the original string, which is exact because every operator is ASCII.
- **Fix: `install.sh` broke on a release note containing a backslash escape**: The script piped the release JSON with `echo`. Its shebang is `env sh`, and under dash and macOS `/bin/sh` `echo` expands `\n` into a real newline, which jq rejects as an unescaped control character. Latent only while releases carry empty notes. Now `printf '%s\n'`. The `| head -n1` is gone too: under `pipefail`, `head` closing the pipe early makes jq exit 141 and takes the installer with it, so selection is `first(...)` inside the jq program. A new dogfood scenario runs the slice through `/bin/sh`, where the old code fails.
- **Fix: Subcommands could not wait on child processes**: `run()` installed `SIG_IGN` with `SA_NOCLDWAIT` for `SIGCHLD`. `waitpid` then returns `ECHILD`, so every `Command::status()` and `Command::output()` in the process failed with "No child processes". The visible symptom was `analyze-project-context` returning an empty `git_keywords` list in a repository with history. The handler and `crates/server/src/signals.rs` are removed, and `book/src/process.md` now states the invariant.
- **Fix: Two lint gates failed open**: ripgrep exits 0 on a match, 1 on no match and 2 on error. `scripts/lint-prose-slop.sh` and `scripts/lint-rust-decoration.sh` tested it directly in an `if`, so a bad glob, an unreadable file or a malformed pattern read as "no match" and the gate printed "lint clean" in both the pre-commit hook and CI. Both now exit 2 with "did not run". `scripts/dogfood-contracts.sh` also asserts `SELECT_ASSET_FROM_JSON` is defined after sourcing its slice of `install.sh`, so a drifted marker can no longer skip the asset-selection scenarios silently.
- **Fix: A corrupt validation row was reported as a passing skill**: `get_validation_summary` and `export_validation_report` decoded `checks_passed` and `checks_failed` with `unwrap_or_default()`, and an empty failure list is exactly how a skill counts as valid. Both now return `MetricsError::Json`.
- **Fix: Cursor plugin manifest and skill bodies could land in different directories**: `write_skills` joined the raw `plugin_name` into a path while `ensure_cursor_plugin_manifest` joined the sanitized one, so a name the sanitizer rewrites (`my.plugin`) gave Cursor a plugin with no skills, and `../` in upstream metadata could steer the write out of `plugins/local`. The root cause was two functions named `sanitize_name`. The Cursor-local pass-through is deleted and call sites name `sanitize_name_kebab`.
- **Fix: `download-transcript-secure` reported success while unimplemented**: The tool is advertised in the list and returned `is_error: false` with "not yet implemented", so a caller branching on `is_error` read it as a transcript. It now returns a tool-level error with `status: "unimplemented"`.
- **Fix: One unreadable file emptied skill discovery**: A `file_hash` failure aborted the whole parallel scan, so a single file vanishing mid-walk (routine while `~/.claude/plugins/cache` is rewritten) returned zero skills and the dashboard showed an empty list. A vanished file is now skipped, matching the frontmatter read below it. Any other failure to stat or hash a file (a permission error, an I/O error) fails the scan with the path named, since silently dropping a skill from discovery, AGENTS.md and the MCP resource list was the quieter bug. The dashboard keeps its last skill list and reports the failure in the activity feed instead of blanking. No test covers this: the failure needs a file to disappear between the walk and the stat, and the walker filters dangling symlinks before they reach `file_hash`.
- **Chore: Delete five never-compiled files, add a module-reachability lint**: `conflict.rs`, `preview.rs`, `snapshot.rs` and `adapters/external.rs` in `crates/sync`, and `scaffolding.rs` in `crates/intelligence`, were under `src/` with no `mod` declaration, so rustc, clippy and coverage skipped 3,734 lines. `external.rs` documented a runtime extension point (`~/.skrills/adapters.toml`) that does not exist in the binary. `scripts/lint-module-reachability.sh`, wired into `make lint-hygiene`, fails when a file under `crates/*/src` is not declared by a `mod` item or a `#[path]` attribute. Also in the Makefile: `dogfood-precommit` gains a `build` prerequisite so a missing or stale binary cannot pass the gate, `demo-gateway` stops reporting only its last loop iteration, and `verify-publish`, `test-scripts`, `cold-window`, the `plugin-*` targets and the new lint are `.PHONY`.
- **Security: Close three advisories in transitive dependencies**: `cargo update` moves `crossbeam-epoch` 0.9.18 to 0.9.20 (RUSTSEC-2026-0204, invalid pointer dereference in the `fmt::Pointer` impl for `Atomic`/`Shared`), `h2` 0.4.13 to 0.4.19 (RUSTSEC-2026-0258, unbounded empty DATA frames), and `quinn-proto` 0.11.14 to 0.11.17 (RUSTSEC-2026-0185, severity 7.5, remote memory exhaustion from unbounded out-of-order stream reassembly). Lockfile only, no manifest or source change. `cargo audit` drops from four vulnerabilities to one.
- **Security: Upgrade rmcp to 1.x, closing RUSTSEC-2026-0189** (severity 8.8, DNS rebinding in the Streamable HTTP server transport). `crates/server/src/http_transport.rs` mounts `StreamableHttpService`, the affected component, and added no Host allowlist of its own, so `skrills serve --http` was exposed: a page on an attacker's domain that resolves to the loopback address still sends its own name in `Host`, and CORS does not stop it. rmcp 1.x validates `Host` and defaults to `localhost`, `127.0.0.1` and `::1`.
- **Migration: rmcp 1.x model types are `#[non_exhaustive]`**: `Tool`, `CallToolResult`, `ReadResourceResult`, `RequestContext` and `CallToolRequestParam` can no longer be built with struct literals from outside rmcp (functional update syntax does not help either), so ~90 sites across nine files moved to the constructors: `Tool::new` with `with_title` / `with_raw_output_schema`, `CallToolResult::success` / `::error`, `ReadResourceResult::new`, `RequestContext::new`, `CallToolRequestParam::new().with_arguments()`. Because no rmcp constructor takes structured content alongside caller-chosen text content, a `tool_result` helper (`crates/server/src/mcp_result.rs`, and one local to `crates/subagents/src/service.rs`) sets `structured_content` after construction. `ListResourcesResult` and `ListToolsResult` gained a `meta` field. No behavior change at the protocol surface: 2562 tests pass.
- **New: `skrills serve --allowed-hosts <HOSTS>`** (env `SKRILLS_ALLOWED_HOSTS`): binding a non-loopback address is documented (`--http 0.0.0.0:3000`), and Host validation rejects it unless the operator names the host clients use. Supplied hosts extend rmcp's loopback list rather than replace it, so adding one cannot silently disable the rebinding protection. Settable three ways, matching `cors_origins`: the flag, `SKRILLS_ALLOWED_HOSTS`, or `allowed_hosts` under `[serve]` in the config file. Five tests pin the default, the extension, that loopback survives, flag and env parsing, and config-file parsing.
- **Fix: Two install-selection scenarios never ran on macOS**: `scripts/dogfood-contracts.sh` sourced the asset-selection functions out of `scripts/install.sh` through `source <(sed ...)`. Bash 3.2, the stock macOS `/bin/bash` the Makefile invokes, stops reading a sourced process substitution partway, so `SELECT_ASSET_FROM_JSON` went undefined and both sidecar-selection scenarios died with `command not found` instead of running. The slice is written to a temp file under the script's existing `$WORK` directory and sourced from there, which every bash version reads whole. CI runs the same script on Linux bash 5, where it always worked, which is why the gap survived.
- **Fix: `make dogfood-precommit` swallowed a failing contract run**: the recipe separated the two dogfood scripts with `;`, so make saw only the last script's exit status and a failing `dogfood-contracts.sh` passed the gate silently. Chained with `&&`.
- **Chore: `make verify-publish` finds a bash 4+ when one exists**: `scripts/verify_publish_order.sh` needs associative arrays, and the target previously skipped outright on any machine whose `bash` was 3.2. It now probes the usual Homebrew locations first and only skips when no bash 4+ is present, with a message naming the fix.
- **Chore: gitignore `.DS_Store`**: Finder metadata appeared as untracked noise in every `git status` on macOS.
- **Fix: Cursor plugin sync carried manifests only**: `CursorAdapter::write_plugin_assets` skipped every asset whose relative path was not `.claude-plugin/plugin.json`, so `~/.cursor/plugins/local/<plugin>/` held a manifest and nothing else. Skill bodies and runtime scripts never reached Cursor, and because `sync_skills` is off for Cursor targets (`sync_to_cursor_tool` in `crates/server/src/app/tools.rs`, and the CLI sync paths in `crates/cli/src/dispatcher.rs`), no other path carried them either: skills that shell out to a companion script (for example `abstract`'s `makefile_dogfooder.py`) failed under Cursor while working under Claude Code. The writer now mirrors the whole plugin tree, each file at its path within the plugin, renaming the `.claude-plugin/` prefix to `.cursor-plugin/`. Unchanged files are still skipped by content hash, a relative path that escapes its plugin directory is refused with a warning instead of written, and the executable bit is preserved on Unix. Six tests in `crates/sync/src/adapters/cursor/tests.rs` pin skill-body and script placement, the unchanged-skip, the manifest rename (asserting no `.claude-plugin/` directory survives), traversal refusal, and the executable bit. `plugin_assets_ignores_non_manifest_files` is replaced by `plugin_assets_writes_script_to_local_plugin_dir`, which asserts the opposite of what it used to. Verified against a real tree with `skrills sync-all --from claude --to cursor`: 659 assets written, 1478 unchanged, and 251 `SKILL.md` files under `~/.cursor/plugins/local/`, matching the 251 in the active plugin versions under `~/.claude/plugins/cache/` exactly.
- **Chore: Clear two clippy 1.97 errors**: `question_mark` in `crates/tome/src/dispatcher.rs` (the outer `match` on `std::fs::read` whose `Err` arm only re-wrapped and returned) and `useless_borrows_in_formatting` in `crates/dashboard/src/app.rs`. Both are behavior-preserving. The workspace does not pin a toolchain, so `make lint` and the pre-commit hook failed on clippy 1.97 before this.

## 0.8.2 - 2026-05-30

- **Feature: Minimalist TUI interface model for the cold-window surface**: Adopts the research-backed "lazygit/gitui model" (see `book/src/cold-window.md`, "Design model and research basis"): a minimal fixed default surface with all depth behind modal overlays. New in `skrills_dashboard::cold_window`: a pane focus model (`Tab`/`Shift-Tab` cycle; the focused pane gets a bold cyan border and a `>` title marker that reads without color), a single keymap table (`keymap.rs`) that feeds both the `?` help overlay and the contextual hint line so a binding can never silently miss the docs (audit tests enforce disjoint key codes and modifier-free reachability for every action), a Vec-backed modal overlay stack (`overlay.rs`, the gitui `popup_stack` pattern in-tree), per-pane selection cursors (`Up`/`Down`/`j`/`k`) with `Enter` drill-down detail overlays, `z` pane zoom, a new **Compact** responsive tier (< 45 columns: only the focused pane renders; focus is visibility) for phone-width SSH sessions, and a 20x6 minimum-size guard. A `:` command palette (k9s pattern) lists every command with case-insensitive filtering; `Enter` executes by replaying the command's key through the normal routing, so palette behavior can never drift from the keybindings, and a sync test fails if a palette entry replays a key no binding owns. The status bar now renders its content borderless with the focused pane's key hints right-aligned (truncating with an ellipsis while always keeping `? help`), fixing a latent bug where the `Borders::TOP` block at the bar's 1-row height swallowed the content and the running TUI showed only a title rule.
- **BREAKING: `Esc` no longer quits the cold-window TUI**: it closes the topmost overlay, then clears zoom, and does nothing at the base surface, matching lazygit/gitui/k9s convention. Quit with `q` (which also closes an open overlay first) or `Ctrl-C`.
- **Feature: Cold-Window TUI surface (`skrills cold-window --tui`)**: Completes the terminal surface the v0.8.0 cold-window panes were built for. A new `skrills_dashboard::cold_window::tui` module composes the alert, hint, research, and status-bar panes into a single frame (`draw`), routes keystrokes (`handle_key`: global `q`/`Esc`/`Ctrl-C` quit, with the disjoint per-pane keymaps `A`/`d`, `0`-`5`/`P`, `R` forwarded), and drives a crossterm raw-mode event loop (`run`) that subscribes to `ColdWindowEngine::subscribe`, the same broadcast bus the browser surface consumes, so `--tui` and `--browser` can run together. The loop installs a panic hook that restores the terminal, repaints on snapshot/key/resize plus a 250ms floor, rings the terminal bell on a newly-fired WARNING (suppressed by `--no-bell`), and tears down within the existing 2-second shutdown budget. The server's `cold-window --tui` path builds the live research-quota closure from the same `BucketedBudget` the browser status bar uses. The previously `#[ignore]`d SC3 assertion (`cold_window_tui_startup_under_five_hundred_ms`) now measures the real ingest-plus-render path against a `TestBackend` and passes under the 500ms budget; new unit tests cover composite layout across terminal sizes, quit-key routing, Ctrl-C vs. plain-`c`, and master-ack delivery. `--tui` requires a TTY and returns a clear error otherwise; behind a non-default build without the `dashboard` feature it reports that the feature was not compiled in.
- **Feature: Responsive `--tui` layout**: The cold-window panes now re-flow live on terminal resize across three width tiers, factored into a pure `plan_layout(area, research_collapsed) -> ColdWindowLayout` function (`skrills_dashboard::cold_window::tui`) that `draw` consumes, so the geometry is unit-tested with `Rect`s, no terminal required. **Wide** (≥ 80 columns): alerts over hints in a 60% left column with research filling the 40% right column. **Medium** (60-79 columns): the same two-column topology but a slimmer research column (68% left) so the alert/hint text keeps its width. **Narrow** (< 60 columns, e.g. a phone session or split pane): every pane stacks full-width top to bottom, and a collapsed research pane shrinks to a fixed three-line badge so alerts and hints keep the room (expanded, it earns a 35/30/35 share). The one-line status bar stays pinned to the bottom row in every tier. Also fixes `ResearchPaneState`'s `Default`: it now delegates to `new()` (starts collapsed) rather than the `#[derive(Default)]` `collapsed = false`, which would have silently opened the pane on launch against the documented "collapsed by default" contract; a regression test pins `Default` to `new()` and a `draw`-level test drives the expanded research path through the narrow and medium tiers so a wrong vertical slot can't truncate the live render undetected.

## 0.8.1 - 2026-05-09

- **Fix: Synthesize `plugin.json` for manifest-less plugins (full-mirror only)**: Plugins shipped without `.claude-plugin/plugin.json` (e.g. `typescript-lsp` and `pyright-lsp` from `claude-plugins-official`) were silently dropped from the target's plugin list during `claude --to cursor` sync, with only a `WARN` line emitted. `crates/sync/src/adapters/claude/plugin_assets.rs` now emits a minimal synthesized manifest: `name` and `version` derived from the cache directory layout, `description` from the README's first non-heading prose line (with the fallback string `"Plugin '<name>' (manifest synthesized by skrills sync)"` when no README is present), empty `commands` / `skills` / `agents` / `hooks` arrays matching on-disk reality, and a `_synthesized: true` marker so future audits can distinguish generated manifests from real ones. Real manifests are never overwritten; synthesis only runs when the plugin walk finds no `plugin.json`, and is gated to `full_mirror=true` so Codex-style skill-only sync targets stay untouched. Verified end-to-end with `skrills sync-all --from claude --to cursor`: the prior warning logs for missing `.claude-plugin/plugin.json` manifests are replaced by `INFO Synthesized .claude-plugin/plugin.json for manifest-less plugin` lines (with `plugin=` and `version=` structured fields), and valid `plugin.json` files land under `~/.cursor/plugins/local/{typescript-lsp,pyright-lsp}/.cursor-plugin/`.
- **Testing: Pin manifest-synthesis contract**: Three new and strengthened tests in `crates/sync/src/adapters/claude/mod.rs` lock the synthesis behavior against silent regressions, complementing the pre-existing `read_plugin_assets_does_not_synthesize_when_real_manifest_present` (which guards real-manifest preservation). `read_plugin_assets_synthesizes_missing_manifest_in_full_mirror` (strengthened) asserts the description equals the exact first prose line from the README and that `_synthesized: true` plus empty component arrays are present. `read_plugin_assets_synthesized_description_falls_back_when_no_readme` covers the no-README path against the documented fallback string. `read_plugin_assets_does_not_synthesize_in_skill_mirror_mode` confirms synthesis is gated to `full_mirror=true` so the skill-mirror code path leaves manifest-less plugins as-is.

## 0.8.0 - 2026-04-28

- **Feature: Cold-Window Real-Time Analysis (`skrills cold-window`)**: New continuously-refreshing analysis surface for the skrills ecosystem. Adds a `skrills-snapshot` wire-format crate and a `cold_window` module across `skrills-analyze`, `skrills-intelligence`, `skrills-metrics`, `skrills-tome`, `skrills-dashboard`, and `skrills-server`. Each tick re-reads authoritative state, runs a 4-tier alert policy with hysteresis and min-dwell (CAS-inspired tier model and ISA-18.2 alarm-management patterns), ranks hints via a recency-weighted multi-signal scorer, attributes tokens per-source (skill / plugin / MCP / conversation), and gates external research fetches through an AlertManager-style dispatcher with restart-resilient quota persistence at `~/.skrills/research-quota.json`. Browser surface ships in v0.8.0 (`--browser` flag); TUI panes are implemented as library code with 49 passing tests and mount in a follow-up. Token thresholds are research-backed: 20K = Advisory (Anthropic API quadratic inflection per the Feb 2026 HN "Expensively Quadratic" analysis), 50K = Caution (Willison "too many MCPs" overhead range), 80% budget = Warning, 100% = kill-switch engaged. Defense-in-depth XSS posture: server-side `html_escape`, then browser-side `DOMParser` and `replaceChildren`. Adaptive cadence borrows the Linux scheduler's 0.7 / 0.9 load-ratio thresholds. 178 cold-window-specific tests and 2 doctests; full brief, spec, plan, and war-room artifacts under `docs/cold-window-*.md`.
- **Testing: Release-Consistency Invariants**: New filesystem-walking tests in `crates/test-utils/tests/release_consistency.rs` guard the workspace against version drift and command-manifest drift. Five invariants: (1) all `crates/*/Cargo.toml` versions agree; (2) `plugins/skrills/.claude-plugin/plugin.json` version matches the workspace crate version; (3) every command path in `plugin.json.commands` exists on disk; (4) top-level `plugins/skrills/commands/*.md` count equals `plugin.json.commands.length`; (5) `.claude-plugin/marketplace.json` plugin entries (and optional `metadata.version`) agree with the workspace and each entry's `source` path exists on disk. Pattern adapted from `claude-night-market` v1.9.3 (`plugins/sanctum/tests/test_release_consistency.py`). RED-on-drift demonstrated for each invariant before commit.
- **Makefile: `release-consistency` Target**: New focused entry point (`make release-consistency`) for pre-tag verification. Wraps `cargo test -p skrills_test_utils --test release_consistency`. The existing `make test` and `make test-integration` continue to pick this up via the `--test '*'` filter.
- **Makefile: `dogfood-all` Surface Coverage**: New dogfood targets exercise every v0.8.0 surface end-to-end against real fixtures. `dogfood-cold-window-headless` runs the engine for 3 s with a 200 ms cadence and asserts clean `SIGTERM` exit. `dogfood-cold-window-chaos` runs `--no-adaptive` with `--alert-budget 1` to force the kill-switch path. `dogfood-cold-window-browser` boots `cold-window --browser` on `:18888`, validates the rendered HTML declares `EventSource` listeners for `{alert, hint, research, status}`, opens a 2 s `curl -N` against `/dashboard.sse` and confirms the engine emits matching `event:` names plus ≥4 `data:` payloads, then sends `SIGTERM` and asserts shutdown completes inside the spec § 3 / TASK-031 2 s budget, the parity guarantee that `crates/server/tests/cold_window_parity.rs` verifies via the broadcast bus, but at the HTTP contract layer. `dogfood-tui` and `dogfood-dashboard` validate the TTY-or-graceful-refusal contract for both interactive subcommands. `dogfood-skill-diff` validates `skrills skill-diff <name> --format json` produces JSON with `.skill` and `.found_in` keys. `plugin-{validate,modernize,registrations}-direct` exercise the in-tree Python ports (`scripts/validate_plugin.py`, `scripts/check_hook_modernization.py`, `scripts/update_plugin_registrations.py`) directly rather than via the `audit-plugins.sh` shim, ensuring local code is exercised even when `NIGHT_MARKET_ROOT` would otherwise route to the upstream sanctum scripts. `dogfood-all` aggregates the lot.
- **Fix: `Commands::Dashboard` TTY Guard**: Added an `is_terminal()` guard to the `Dashboard` arm of `crates/server/src/app/dispatcher.rs` mirroring the existing guard in `crates/server/src/tui.rs:20-22`. Prior to this fix, `skrills dashboard </dev/null` would crash inside `skrills_dashboard::Dashboard::run` with `Error: No such device or address (os error 6)` when termios syscalls hit a non-TTY stdin; the subcommand now returns `Error: Dashboard requires a TTY` with rc=1 instead. Surfaced by the new `dogfood-dashboard` target. Includes a unit test in `crates/server/src/tui.rs` for the symmetric `tui_flow` guard (the dispatcher arm is integration-tested by the dogfood target).

## 0.7.7 - 2026-04-15
- **Refactor: Manifest-Only Plugin Sync**: Rewrote `write_plugin_assets` from full cache mirroring (`plugins/cache/`) to manifest-only writing (`plugins/local/<plugin>/.cursor-plugin/plugin.json`). Cursor natively discovers plugin content from `~/.claude/plugins/cache/`, so only registration manifests need to be synced. Stale plugin directories are pruned from `plugins/local/` when no longer in the sync set.
- **NEW: Plugin-Aware Skill Writing**: Skills with a `PluginOrigin` are written to `plugins/local/<plugin>/skills/<name>/SKILL.md` instead of the flat `skills/` directory. Cursor's plugin system discovers them as installed plugins. `.cursor-plugin/plugin.json` manifests are auto-created per plugin.
- **NEW: `PluginOrigin` Type**: Added `PluginOrigin` struct to `Command`, tracking which plugin (name, publisher, version) an artifact originated from. Source adapters set this when reading from plugin caches.
- **NEW: Validation Cache**: SQLite-backed `ValidationCache` in `crates/state/src/cache.rs` stores validation results with staleness indicators (fresh/recent/aging/stale) for offline `skrills validate`. Includes auto-cleanup of entries older than a configurable retention period.
- **NEW: Network Detection**: `NetworkStatus` module (`crates/state/src/network.rs`) provides `check_connectivity()` with configurable timeout and `is_online()` convenience function for graceful offline degradation.
- **NEW: GitHub Action**: Reusable composite action at `.github/actions/validate-skills/` validates skills in pull requests. Configurable targets, strictness, and path. Reports issues as GitHub annotations.
- **NEW: Interactive Sync Preview**: Added `interactive` field to `SyncParams` and `preview.rs` module (scaffolded) for per-file accept/reject diffs before writing.
- **Refactor: `strum` Derives for Knowledge Graph**: `NodeKind` and `EdgeKind` now use `strum::EnumIter` and `strum::EnumCount` instead of manually maintained `all()` methods, providing compile-time completeness guarantees.
- **Fix: `cargo fmt` Violations**: Resolved formatting issues in `claude.rs`, `cursor/tests.rs`, and `knowledge_graph.rs` that were failing CI.
- **Fix: Marketplace Manifest**: Updated `.claude-plugin/marketplace.json` from stale 0.6.1 to 0.7.7 (36 MCP tools, added Cursor to description).
- **Testing**: 8 manifest-sync and stale-plugin pruning tests (replaces 6 legacy cache-mirroring tests). 1 async snake_case normalization test covering 4 research tools. 21 validation cache tests. 5 network detection tests. 43 integration test sites updated for new `interactive` and `plugin_origin` fields.

## 0.7.6 - 2026-04-12
- **NEW: Plugin Assets Sync**: Cursor sync now mirrors plugin runtime scripts (Python, shell), binaries, and source packages from `~/.claude/plugins/cache/` to `~/.cursor/plugins/cache/`. Preserves directory hierarchy so skills can reference companion scripts at the same relative paths. Controlled by the `sync_plugin_assets` flag in `SyncParams` (default: enabled).
- **NEW: `PluginAsset` Type**: Added `PluginAsset` struct to the sync common schema, with publisher, plugin name, version, relative path, content, hash, and executable permission tracking.
- **NEW: Claude Adapter `read_plugin_assets()`**: Walks the plugin cache collecting all files except those handled by other sync paths (`skills/`, `commands/`, `agents/`) and runtime artifacts (`tests/`, `.venv/`, `__pycache__/`, `node_modules/`, `.git/`). This captures scripts, binaries, hooks, source packages, and config files.
- **NEW: Cursor Adapter `write_plugin_assets()`**: Writes plugin assets to `~/.cursor/plugins/cache/` mirroring the Claude structure. Supports hash-based change detection and preserves executable permissions on Unix.
- **Testing**: Added 18 tests covering plugin asset read/write, exclusion rules, version picking, executable detection, serialization roundtrip, report totals, and integration flows (dry-run, unchanged detection, hierarchy preservation).

## 0.7.5 - 2026-04-07
- **Fix: Cursor Skill Descriptions**: Cursor adapter now preserves skill `description` as a plain-text first line instead of discarding it. Handles YAML block scalars (`>-`, `>`, `|-`, `|`), multi-line quoted strings, and strips surrounding quotes. `model_hint` remains as an HTML comment. Cursor shows the description as the skill subtitle instead of `<!-- model_hint: ... -->`.
- **Refactor: Research Handlers**: Hoisted function-level imports to module scope in `research.rs`, reducing per-call overhead and improving readability.
- **Refactor: Serde-Based Parsing**: Replaced manual `match` arms for `NodeKind`, `EdgeKind`, and `Parameter` parsing with `serde_json::from_value`, eliminating duplication between enum definitions and parsers.
- **Fix: Search Papers Error Reporting**: `search_papers_tool` now sets `is_error: true` when all sources fail, instead of silently returning an empty result.
- **Fix: Discussion Limit Clamping**: `search_discussions_tool` clamps the `limit` parameter to 100 to prevent excessive API requests.
- **Fix: PDF Download Hardening**: `fetch_pdf_tool` adds a 30-second timeout and HTTP status check before writing downloaded PDFs.
- **Fix: Unpaywall Error Logging**: `resolve_doi_tool` logs Unpaywall lookup failures via `tracing::warn` instead of silently swallowing errors.
- **Refactor: Cache Directory**: Extracted `ResearchCache::cache_dir()` as the canonical source for the skrills-tome cache path. Both `open()` and server-side `tome_cache_dir()` now delegate to it.
- **Refactor: Knowledge Graph Timestamps**: Changed `Node.created_at` from `String` to `OffsetDateTime` with RFC 3339 serde, ensuring type-safe timestamp handling.
- **Refactor: Enum Helpers**: Added `NodeKind::all()` and `EdgeKind::all()` for exhaustive enumeration, used by schema sync tests.
- **Fix: Snake-Case Tool Names**: All 36 MCP tools now accept both kebab-case (`search-papers`) and snake_case (`search_papers`) names via universal `replace('_', "-")` normalization in `call_tool()` dispatch.
- **Fix: Legacy Timestamp Fallback**: `parse_rfc3339` falls back to SQLite's `datetime('now')` format (`YYYY-MM-DD HH:MM:SS`) for knowledge graph databases created before 0.7.5, treating legacy timestamps as UTC.
- **Testing**: Added `cache_dir_creates_directory`, `node_kind_all_covers_every_variant`, `edge_kind_all_covers_every_variant`, `parse_rfc3339_accepts_legacy_sqlite_format`, and `parse_rfc3339_accepts_rfc3339_format` tests.

## 0.7.4 - 2026-04-02
- **NEW: Research MCP Tools**: 9 tools exposing the `tome` crate over MCP: `search-papers`, `search-discussions`, `resolve-doi`, `fetch-pdf`, `query-knowledge-graph`, `add-knowledge-node`, `link-knowledge`, `track-citations`, and `resolve-contradiction` (27 → 36 tools total).
- **Fix: Paper Model Refinements**: Improved `Paper` model fields and research handler argument parsing.

## 0.7.3 - 2026-03-29
- **NEW: MCP Tool Filtering**: `McpServer` now carries `allowed_tools` and `disabled_tools` fields. All four adapters (Claude, Codex, Copilot, Cursor) read and write these fields, enabling per-server tool whitelists and blacklists during sync.
- **NEW: MCP Servers Dashboard Panel**: The TUI dashboard displays a dedicated MCP servers panel showing server name, source adapter, transport type, command, and tool filter indicators. The browser dashboard includes a matching panel fetched from `/api/mcp-servers`.
- **NEW: `/api/mcp-servers` Endpoint**: REST API endpoint aggregating MCP server configurations from all four adapters with tool filtering metadata.
- **Fix: Configurable Unpaywall Email**: `UnpaywallClient::new()` accepts a custom email parameter instead of a hardcoded default, enabling users to comply with Unpaywall's polite pool requirements.
- **Fix: `OffsetDateTime` for Discussions**: `Discussion::created_at` changed from `Option<String>` to `Option<OffsetDateTime>` with RFC 3339 serde, replacing stringly-typed timestamps with proper date-time values.
- **Fix: SQL Upsert for Knowledge Graph**: `INSERT OR REPLACE` replaced with `INSERT ... ON CONFLICT DO UPDATE` to preserve `created_at` timestamps during node and edge updates. Same fix applied to `track_paper` in citation tracker.
- **Fix: Portal Estimates**: Improved estimate display in `skrills-portal.html`.
- **Fix: Validate Comment Docs**: Clarified `ValidationSummary::all_valid` doc comment to specify it is the intersection of all target validations.
- **Refactor: Shared `split_frontmatter`**: Promoted `split_frontmatter` from the Cursor adapter to `crates/sync/src/adapters/utils.rs` for cross-adapter reuse.
- **Testing**: Added parser tests for arXiv, CrossRef, HN Algolia, OpenAlex, and Semantic Scholar clients. Added Cursor adapter tests for subdirectory rule collisions and agent frontmatter output format. Added sync error path tests for same-source rejection. Added serde roundtrip tests for `Discussion` and `UnpaywallClient` constructor tests. Added MCP tool-filtering roundtrip tests for Claude, Codex, Copilot, and Cursor adapters. Added dashboard MCP panel rendering tests and `/api/mcp-servers` integration tests.

## 0.7.2 - 2026-03-25
- **Fix: Cursor Sync Reliability**: Improved bidirectional sync for Cursor adapter: agents, commands, rules, skills, and utility modules hardened with better error handling and edge-case coverage.
- **NEW: Shorthand Sync Commands**: Added `/sync-from-cursor`, `/sync-from-claude`, and `/sync-from-copilot` slash commands for quick one-direction syncs.
- **NEW: `tome` Research Crate**: Academic paper search, citation extraction, knowledge graph, PDF retrieval, rate limiting, and caching (`skrills-tome`).
- **CLI**: `--open` flag now enforces `--http` as a prerequisite via clap `requires`. All sync commands updated to include Cursor as a valid source/target.
- **Testing**: Extended Cursor adapter test coverage; added `parse_serve_open_without_http_is_error` CLI test.
- **Known behavior**: First sync to Cursor will rewrite all empty-body agent files due to frontmatter format normalization (trailing `\n---\n` vs `\n---`). This is a one-time operation with no data loss.
- **Internal**: Bumped all workspace crate versions from 0.7.1 to 0.7.2.

## 0.7.1 - 2026-03-21
- **NEW: `multi-cli-agent` Command**: Launch agents via Claude or Codex CLI with automatic backend detection and fallback routing.
- **NEW: `--open` Flag**: Auto-launch the browser dashboard on `skrills serve`.
- **HTTP Transport**: Port fallback when the default port is busy, trying up to 10 consecutive ports.
- **Dashboard**: Light-mode support for the browser dashboard.
- **Testing**: Added edge-case tests for empty skill list navigation (`Home`/`End` keys), `find_binary` boundary conditions, `resolve_backends` invariants, and `bind_with_fallback` port exhaustion.

## 0.7.0 - 2026-03-19
- **NEW: Cursor IDE Support**: Full bidirectional sync for Cursor IDE (`.cursor/`).
  - Rules: Sync between Cursor `.mdc` files and Claude `CLAUDE.md` / Copilot `*.instructions.md` with mode derivation (`alwaysApply`, globs, agent-requested).
  - Skills: Sync to/from `.cursor/skills/{name}/SKILL.md`.
  - Commands: Sync to/from `.cursor/commands/{name}.md`.
  - Agents: Sync with field translation (`background` ↔ `is_background`).
  - Hooks: Sync with event name translation (`PascalCase` ↔ `camelCase`).
  - MCP: Sync via `.cursor/mcp.json`.
- **NEW: CLI Flags**: Added `--from cursor` and `--to cursor` for sync-all and sync-* commands.
- **NEW: MCP Tools**: `sync-from-cursor` and `sync-to-cursor` (25 → 27 tools total).
- **NEW: Slash Commands**: `/sync-from-cursor` and `/sync-to-cursor` for plugin users.
- **Refactor: Shared Adapter Utilities**: Extracted common sync logic from per-adapter modules into `crates/sync/src/adapters/utils.rs` and `traits.rs`.
- **Refactor: Orchestrator**: Reworked sync orchestrator to support four-way sync routing.
- **Refactor: Subagent Backends**: Simplified Claude and Codex backend implementations.
- **Architecture**: Added ADR 0006 documenting the Cursor rules mapping strategy.
- **Internal**: Bumped all workspace crate versions from 0.6.1 to 0.7.0.

## 0.6.1 - 2026-03-05
- **Fix**: Tests that relied on case-only filename differences (`MySkill`/`myskill`/`MYSKILL`) now use distinct names to avoid collisions on case-insensitive filesystems (macOS APFS/HFS+).
- **Fix**: Validation test uses `Skills.md` instead of `skill.md` for wrong-casing detection, preventing false passes on case-insensitive filesystems.
- **Internal**: Bumped all workspace crate versions from 0.6.0 to 0.6.1.
- **Internal**: Updated marketplace.json MCP tool count from 24 to 25.

## 0.6.0 - 2026-02-15
- **Discovery**: Frontmatter-based skill deduplication across roots using `name` field and Jaccard similarity on descriptions. Skills with matching frontmatter identity (name and similar description) are consolidated regardless of path differences.
- **Dashboard**: Sortable skills panel (press `s` to toggle alphabetical/discovery order), stateful list scrolling via `ListState`, and multi-location display showing all roots where a skill was discovered.
- **Dashboard**: Extracted `SkillLocation` struct to track per-skill discovery sources and paths. Metrics panel now shows location count and source details.
- **Dashboard**: Browser-based dashboard served alongside MCP on the HTTP transport via Leptos SSR. Replaced previous HTMX+Alpine implementation.
- **API**: REST endpoints for skills (`/api/skills`, `/api/skills/{name}`) with pagination and TTL-based caching via `std::sync::RwLock` with poison recovery.
- **API**: Metrics endpoints (`/api/metrics/events`, `/api/metrics/skills/{skill}`) for querying invocation data.
- **Internal**: Migrated Axum route syntax from `:param` to `{param}` for Axum 0.8 compatibility.
- **Fix**: Corrected plugin.json command paths from `../commands/` to `./commands/`.

## 0.5.6 - 2026-01-28
- **Testing**: Added BDD-style unit tests for skill management modules (deprecation, pre-commit, profiling, rollback, usage-report) covering serialization, YAML escaping, git log parsing, file filtering, percentage calculations, and version hash validation.

## 0.5.5 - 2026-01-26
- **NEW: TLS Certificate Management**: Added `skrills cert` subcommand with `status`, `renew`, and `install` operations for managing TLS certificates. Certificate validity is displayed on server startup.
- **Refactor: Copilot Adapter**: Split monolithic `copilot.rs` into focused modules (agents, commands, mcp, paths, preferences, skills, utils, tests) for improved maintainability and testability.
- **Testing**: Added unit tests for certificate parsing covering missing file handling, valid PEM parsing, and invalid PEM detection.

## 0.5.4 - 2026-01-25
- **Testing**: Added BDD-style tests for configuration loading (backend/config.rs) covering defaults, custom values, error handling, and edge cases
- **Testing**: Added directory validation tests (validate/lib.rs) for skill discovery, hidden directory handling, multi-target validation, and edge cases
- **Documentation**: Updated README to reflect 37 CLI commands and added skill management section
- **Internal**: Improved test coverage with 407 new lines of tests across core modules

## 0.5.3 - 2026-01-24
- **Bug Fix**: Fixed CLI binary selection in subagent service to use the `backend` parameter when spawning CLI subprocesses. Previously, running in Codex would incorrectly spawn the `claude` binary instead of `codex`, causing process spawn failures. (#133)
- **NEW: Config File Support**: Security and serve options can now be configured via `~/.skrills/config.toml`. Precedence: CLI > ENV > config file. (#103)
- **NEW: TLS Auto-Generation**: Added `--tls-auto` flag to auto-generate self-signed TLS certificates for development. Certificates are stored in `~/.skrills/tls/` and reused across server restarts. (#104)
- **NEW: Skill Management Commands**: Added 9 new CLI commands for skill lifecycle management:
  - `skill-deprecate`: Mark skills as deprecated with migration guidance
  - `skill-rollback`: Revert skills to previous git versions
  - `skill-profile`: View skill execution statistics from analytics cache
  - `skill-catalog`: Browse and search all available skills with filtering
  - `skill-import`: Import skills from local paths
  - `skill-usage-report`: Generate detailed usage reports
  - `skill-score`: Calculate quality scores for skills (0-100 scale)
  - `sync-pull`: Placeholder for future remote registry integration
  - `pre-commit-validate`: Validate skill files for git hooks
- **NEW: Module Files Sync**: Skills with companion files (helpers, configs) are now synced as complete units, enabling multi-file skill sync between agents.
- **NEW: Instructions Sync**: Sync instructions between Claude (`CLAUDE.md`) and Copilot (`*.instructions.md` format).
- **NEW: Codex Agent Support**: Claude agents are now synced to Codex as skills with "agent-" prefix until Codex adds native agent support.
- **Security**: Added git hash validation for rollback commands, config parse error logging, and proper error handling for silent failures.
- **Testing**: Added 37 new frontmatter tests and 6 skill management tests covering edge cases and error handling.
- **Increased max_depth**: Module file collection now traverses up to 10 levels deep (was 5) for deeply nested skill structures.

## 0.5.2 - 2026-01-22
- **NEW: HTTP Transport Support**: MCP servers using HTTP transport (type="http" with url/headers) are now properly synced. Fixes compatibility with servers like context7. (#111)
- **Bug Fix**: Filter whitespace-only descriptions at extraction point, not just empty strings. (#100)
- **Bug Fix**: Skill discovery summary now logs at info level instead of debug. (#101)
- **Testing**: Added tests for multi-word description matching and special characters. (#99)
- **Dependencies**: Pruned unused dependencies and dead code for smaller binary size.
- **CI**: Allow wildcard path deps in cargo-deny config.

## 0.5.1 - 2026-01-21
- **Refactor: Skill Trace Commands**: Split monolithic `/skill-trace` into three focused commands for clearer UX:
  - `/skill-trace-enable` - Enable tracing with instrumentation
  - `/skill-trace-disable` - Disable tracing and cleanup
  - `/skill-trace-status` - Check current tracing state
- **Refactor: Test Utils Crate**: Extracted shared test infrastructure (`TempEnv`, `TempSkillDir`, fixtures) into new `crates/test-utils/` crate for consistent test setup across all crates.
- **Code Quality**: Sorted command registrations alphabetically in plugin.json.
- **Docs**: Cleaned up `docs/audit-logging.md` - removed stale TODO links and parameterized version references.

## 0.5.0 - 2026-01-20
- **NEW: GitHub Copilot CLI Support**: Full bidirectional sync support for GitHub Copilot CLI (`~/.copilot`).
  - Skills: Sync between Copilot and Claude/Codex (SKILL.md format)
  - Agents: Sync from Claude plugins cache to `~/.copilot/agents/` with format transformation
  - MCP Servers: Read from/write to `mcp-config.json` (Copilot's separate MCP config file)
  - Preferences: Sync via `config.json` with security field preservation
  - Commands: Skipped (Copilot does not support slash commands - see FAQ)
- **NEW: Agent Sync**: Sync Claude Code subagents to Copilot with automatic format transformation (removes `model`/`color`, adds `target: github-copilot`).
- **NEW: CLI Flags**: Added `--from copilot` and `--to copilot` for sync-all and sync-* commands.
- **NEW: MCP Tools**: `sync-from-copilot`, `sync-to-copilot`, and enhanced `sync-skills` with all 6 source→target combinations.
- **NEW: Validation Target**: `--target copilot` for skill validation (same rules as Codex).
- **Testing**: Added 20 Copilot sync integration tests covering all sync directions.
- **Docs**: Added FAQ entry explaining why Copilot doesn't have slash commands.

## 0.4.12 - 2026-01-16
- **Testing**: Added 59 new tests across tool_schemas (20), sync report (20), and validation common (19) modules covering schema generation, report formatting, and validation issue handling.

## 0.4.11 - 2026-01-15
- **Refactor**: Improved test infrastructure with RAII guards (`EnvVarGuard`, `TestFixture`) for reliable parallel test execution.
- **Refactor**: Extracted API key error messages to constants in subagent backends.
- **Bug Fix**: Added logging for silent failure path in skill validation invariant checks.
- **Bug Fix**: Changed timestamp duration calculation to use `saturating_mul` to prevent integer overflow.
- **Docs**: Consolidated 14 documentation files from bulleted lists to flowing prose, reducing visual noise while preserving technical content.
- **Docs**: Added missing `skip_existing_commands` parameter to `sync_all_tool` documentation.
- **Testing**: Added 27 new tests covering cluster formation, deviation score boundaries, n-gram edge cases, behavioral patterns, and parse_trace_target case sensitivity.

## 0.4.10 - 2026-01-15
- **NEW: Confidence Type**: Added `Confidence` newtype with clamping constructor (0.0-1.0) for type-safe confidence scores in recommendations. (#73)
- **Improved: Type Safety**: Removed redundant `ClusteredBehavior::size` field in favor of computed method. (#74)
- **Improved: Observability**: Added tracing for silent early returns in behavioral and comparative recommendation modules. (#75)
- **NEW: Analytics Auto-Persistence**: Added `--auto-persist` flag to `recommend-skills-smart` command and `SKRILLS_AUTO_PERSIST` environment variable for automatic analytics caching after operations that build analytics. (#62)

## 0.4.9 - 2026-01-12
- **NEW: MCP Gateway**: Added context-optimized tool loading via lazy schema loading. Tools `list-mcp-tools`, `describe-mcp-tool`, and `get-context-stats` enable on-demand schema retrieval to reduce context window pressure.
- **NEW: Analytics Persistence**: Export and import usage analytics via `export-analytics` and `import-analytics` CLI commands. Analytics cache persists to `~/.skrills/analytics_cache.json`.
- **NEW: HTTP Security Options**: Added `--auth-token`, `--tls-cert`, `--tls-key`, and `--cors-origins` flags to `serve` command for production deployments.
- **Testing**: Added 11 new tests for analytics persistence functions and MCP gateway handlers.

## 0.4.8 - 2026-01-10
- **NEW: Skill Description Caching**: Added optional `description` field to `SkillMeta` for richer fuzzy skill search. Descriptions are extracted from YAML frontmatter during discovery.
- **Improved: Fuzzy Search**: Enhanced `search-skills-fuzzy` MCP tool to match against skill descriptions in addition to names.
- **Testing**: Added 7 new tests for description extraction covering various frontmatter formats and edge cases.
- **Code Quality**: Applied pedantic clippy auto-fixes (raw string hashes, redundant closures).

## 0.4.7 - 2026-01-09
- **Chore**: Version bump to republish after v0.4.6 tag protection issue.
- All changes from 0.4.6 are included in this release.

## 0.4.6 - 2026-01-08
- **Bug Fixes**: Address 14 issues including warnings for skipped optional dependencies, YAML line/column info in parse errors, SAFETY comments for regex patterns, and actionable hints for I/O errors.
- **Testing**: Add 282 new tests across claude_parser, codex_parser, context detector, github_search, dependencies, llm_generator, and scorer modules.
- **Documentation**: Add audit-logging.md covering security events, mTLS audit trails, and SIEM integration.
- **CI**: Revert sync crate name to skrills_sync (crates.io disallows renaming published crates).
- **Build**: Add fmt-check target to catch formatting issues in pre-commit hooks.

## 0.4.5 - 2026-01-03
- **Testing**: Added tests for tool handler functions including `parse_trace_target`, `skill_loading_status_tool`, `skill_loading_selftest_tool`, and `disable_skill_trace_tool`. Tests cover edge cases, dry-run modes, and target validation for Claude, Codex, and Both trace targets.

## 0.4.4 - 2026-01-02
- **NEW: Empirical Skill Creation**: Generate skills from observed session patterns via `--method empirical`. Clusters successful tool sequences and failure patterns from Claude Code/Codex CLI history.
- **NEW: Comparative Recommendations**: Deviation scoring compares actual skill-assisted outcomes against category baselines (Testing, Debugging, Documentation, etc.) to identify underperforming skills.
- **NEW: Behavioral Analytics**: Extract tool call sequences, file access patterns, and session outcomes (success/failure/partial) from session history for richer usage analysis.
- **Improved: Install Script**: Better error capture and user-scoped MCP registration with `--scope user` flag. Fallback logic now shows detailed error messages.

## 0.4.3 - 2025-12-31
- **NEW: HTTP Transport Installation**: Added `--http` flag to install scripts for HTTP transport instead of stdio. Installs systemd user service and configures MCP clients with HTTP URL.
- **Improved: Runtime HTTP Configuration**: The `--http` CLI flag is now always visible and runtime-checked, rather than compile-time gated.
- **Default: HTTP Transport Enabled**: HTTP transport feature is now enabled by default in release builds.

## 0.4.2 - 2025-12-29
- **NEW: Fuzzy Skill Search**: Added `search-skills-fuzzy` MCP tool with trigram-based similarity matching for typo-tolerant skill discovery (e.g., "databas" finds "database").
- **Improved Tests**: Added edge case tests for similarity matching (unicode, empty strings, punctuation, long strings) and integration tests for fuzzy search tool.

## 0.4.1 - 2025-12-27
- **Hashing**: Switched file hash algorithm to blake2b-256 for improved performance and security.
- **CI/CD**: Improved error handling in publish verification workflow.
- **Documentation**: Added README for the intelligence crate.

## 0.4.0 - 2025-12-24
- **NEW: Intelligent Skills**: Added `skrills-intelligence` crate with project context analysis, skill recommendations, and skill creation capabilities.
- **NEW: Project Context Detection**: Automatically detect programming languages, frameworks, project type, and extract keywords from README and git history.
- **NEW: Smart Recommendations**: Context-aware skill recommendations based on project profile, usage analytics, and dependency relationships.
- **NEW: Skill Creation**: Create new skills via GitHub search or LLM generation with project context awareness.
- **NEW: Usage Analytics**: Parse Claude Code and Codex CLI history for skill usage patterns and co-occurrence analysis.
- **Security Fixes**: Added path traversal guard, GitHub query injection sanitization, and CLI binary name validation.
- **Error Handling**: Improved error logging for silent failures and actionable GitHub API error messages.

## 0.3.5 - 2025-12-21
- **NEW: Agent Discovery**: Added `list-agents` MCP tool for discovering available agents with metadata and caching via `AgentRegistry`.
- **NEW: Run Events Polling**: Added `get-run-events` MCP tool for polling-based event retrieval during agent runs.
- **NEW: Codex CLI Adapter**: Added `CodexCliAdapter` for subprocess execution of Codex CLI agents.
- **NEW: Smart Routing**: Agent routing now automatically selects the appropriate backend based on `agent_id` parameter.
- **NEW: Model Mapping**: Added cross-platform model preference sync for consistent agent configuration.
- **Improved Error Handling**: Added structured `CliError` enum for CLI adapter with better error messages.
- **Logging**: Added error logging for silent failures to improve debugging.
- **Publishing**: Added missing crates.io metadata to workspace crates.
- **Testing**: Added integration tests for the subagent workflow.
- **Documentation**: Addressed PR review suggestions and added SAFETY comments for `json!()` expect invariants.

## 0.3.4 - 2025-12-19
- **NEW: Skill Recommendations**: Added `skrills recommend` CLI command and `recommend-skills` MCP tool for suggesting related skills based on dependency graph relationships (dependencies, dependents, and siblings).

## 0.3.3 - 2025-12-18
- **NEW: Metrics Command**: Added `skrills metrics` CLI command and `skill-metrics` MCP tool for aggregate statistics including quality distribution, dependency graphs, and token usage.
- **NEW: Dependency Graph**: Added `DependencyGraph` module for skill relationship tracking with transitive resolution and cycle detection.
- **NEW: Resolve Dependencies Tool**: Added `resolve-dependencies` MCP tool for querying skill dependencies and dependents.
- **NEW: Dependency Validation**: Added `check_dependencies` flag to `validate-skills` for verifying skill dependencies exist.
- **NEW: Makefile Targets**: Added `make status`, `make install`, `make test-coverage`, `make security`, and `make deps-update` for developer workflows.

## 0.3.2 - 2025-12-17
- **NEW: Dependency Resolution System**: Skill dependency tracking and resolution via YAML frontmatter.
- **Dependency Syntax**: Support for simple, structured, and compact syntax forms in frontmatter.
- **Resolution Engine**: Circular dependency detection, semver version constraints, and source pinning (e.g., `codex:base-skill`).
- **Optional Dependencies**: Configurable behavior for optional dependencies.
- **Extended Validation**: Dependency-related validation issues added to the validate crate.
- **NEW: Skill Loading Trace**: Diagnostic tools (`skill-loading-status`, `enable-skill-trace`, `disable-skill-trace`, `skill-loading-selftest`) for debugging skill loading.
- **Documentation**: Added ADR 0002 for dependency resolution architecture.
- **BREAKING**: rmcp updated to 0.10; removed deprecated `info()` method from ServerHandler impl.
- **BREAKING**: `SkillSource` enum requires wildcard pattern matching due to `#[non_exhaustive]`.

## 0.3.1 - 2025-12-13
- **NEW: Validation Crate** (`skrills-validate`): Validate skills for Claude Code (permissive) and Codex CLI (strict frontmatter requirements). Includes auto-fix capability to add missing frontmatter.
- **NEW: Analysis Crate** (`skrills-analyze`): Token counting, dependency analysis, and optimization suggestions for skills.
- **NEW: Validation Integration**: Sync operations now optionally validate skills with `--validate` and `--autofix` flags.
- **Enhanced CLI**: Added `skrills validate` and `skrills analyze` commands for skill quality assurance.
- **MCP Tools**: Added `validate-skills` and `analyze-skills` tools to the MCP server.
- **Tests**: Added 53 tests for bidirectional skill sync covering validation, autofix, edge cases, and negative testing.
- **Architecture Pivot**: Removed redundant skill-serving functionality now that Claude Code and Codex CLI have native skill support. Skrills now focuses on validation, analysis, and sync.
- **REMOVED**: `autoload.rs`, `emit.rs`, and related autoload/emit functionality.
- **REMOVED**: CLI commands: `list`, `list-pinned`, `pin`, `unpin`, `auto-pin`, `history`, `emit-autoload`, `render-preview`.
- **REMOVED**: MCP tools: `list-skills`, `autoload-snippet`, `render-preview`, `runtime-status`, `set-runtime-options`, `pin-skills`, `unpin-skills`, `refresh-cache`.

## 0.3.0 - 2025-12-12
- **NEW: Subagents Module**: Added subagent functionality with MCP server support. Run subagents via `list-subagents`, `run-subagent`, and `get-run-status` tools.
- **NEW: Backend Support**: Implemented dual backend support for both Claude-style and Codex-style subagent execution with configurable adapters.
- **NEW: Sync Infrastructure**: Introduced cross-agent sync orchestration with `SyncOrchestrator`, `ClaudeAdapter`, and `CodexAdapter` for multi-agent coordination.
- **Documentation**: Added AGENTS.md (1500+ lines) with subagent usage examples, configuration options, and best practices.
- **Enhanced CLI**: Added sync commands (`skrills sync import`, `skrills sync export`, `skrills sync report`) for cross-agent skill synchronization.
- **Testing**: Added end-to-end integration tests for subagents functionality ensuring reliable operation across different backends.
- **BREAKING**: Removed the gateway crate and related functionality. The gateway approach has been replaced with a simpler MCP server integration for Codex.
- Codex setup now uses AGENTS.md instructions combined with MCP server registration in `~/.codex/config.toml`.
- Setup no longer generates TLS certificates or wrapper scripts; it now directly registers the MCP server.
- Removed gateway-related MCP tools (`gateway-status`, `gateway-start`, `gateway-restart`, `gateway-stop`).
- The MCP server provides full skill management capabilities including two-tier caching (discovery and content), trigram-based semantic matching, auto-pinning, and usage history tracking.

## 0.2.2 - 2025-12-04
- **Focus on Claude Code**: Simplified integration to focus on Claude Code hook-based skill injection.
- The installer now accepts `--client claude` flag (or `SKRILLS_CLIENT` environment variable) to target specific hook and configuration paths.
- Aligned all workspace crates to version 0.3.0 and updated `book/src/release.md` with the latest release notes.
- A security vulnerability was addressed by updating `rmcp` from 0.9.1 to 0.10.0. This change replaces the unmaintained `paste` dependency (version 1.0.15) with the actively maintained `pastey` (version 0.2.0).
- Autoload now uses `SKRILLS_EMBED_THRESHOLD` as the default embedding threshold if the command-line flag is not set. Manifest previews are now scaled to byte budgets to prevent missed filters and mismatches in gzip previews.
- The skills manifest loader now supports both the legacy array-only JSON format and the new structured object format. This ensures backward compatibility and prevents parsing failures with older configurations.

## 0.2.0 - 2025-11-26
- **Project Restructuring**: The project has been reorganized from a monolithic structure to a modular one, improving maintainability and scalability.
- **Project Renaming**: The project has been renamed from "codex-mcp-skills" to "skrills" across all documentation and source code.
- **Modular Workspace**: A new workspace structure has been implemented, with distinct crates for `discovery`, `state`, and `server` functionalities.
- **CI/CD Enhancements**: The continuous integration pipeline now includes a code coverage workflow and checks for differences in the public API to prevent unintended breaking changes.
- **Documentation Overhaul**: All project documentation has been updated with improved formatting and now includes Mermaid diagrams for better visualization of system architecture.
- **Code Cleanup**: Obsolete binaries and artifacts have been removed to improve the overall maintainability of the codebase.

## 0.2.1 - 2025-11-26
- Added cargo publishing infrastructure with dependency validation and dry-run checks.
- Enhanced release workflow with automated crate publishing to crates.io.
- Improved test isolation in server module to prevent cross-test contamination.
- Updated project documentation with better formatting and clearer explanations.
- Added dependency order validation before packaging to ensure publishing reliability.
- Fixed Python syntax errors in release workflow and improved code formatting.
- Enhanced project icon and improved README documentation clarity.

## 0.1.14 - 2025-11-25
- Added `skrills doctor` to inspect Codex MCP config (`mcp_servers.json`, `config.toml`) and verify `type = "stdio"` and binary paths.
- MCP tool schemas now always include `type = "object"` to prevent Codex from raising `missing field "type"` on startup.
- `serve` command gains `--trace-wire` / `SKRILLS_TRACE_WIRE=1` to log MCP initialize traffic; warm-up now defers until after handshake.
- Installers write `type = "stdio"` to both Codex config files, tolerate permission failures, add a POSIX `--local` build flag, and maintain PowerShell parity.

## 0.1.13 - 2025-11-25
- The installer filters release archives more strictly and uses a source build as a secondary option when no match is found.
- CI jobs are restricted by path changes to avoid unnecessary runs.

## 0.1.12 - 2025-11-25
- Improved installer release-asset lookup for resiliency across different GitHub API responses.

## 0.1.11 - 2025-11-25
- Release workflow skips asset uploads if a release with the same tag already exists, preventing duplicate publishes.

## 0.1.10 - 2025-11-25
- Release workflow creates the GitHub release before uploading assets to eliminate race conditions.

## 0.1.9 - 2025-11-25
- Fixed release upload include patterns to ensure platform archives are attached correctly.

## 0.1.8 - 2025-11-25
- The manifest flag is now respected when hiding agent documentation in autoload outputs.
- Cached cargo artifacts in the audit workflow to speed up security checks.
- Corrected release include paths across workflows.

## 0.1.7 - 2025-11-25
- Added release dry-run builds and cache reuse in CI to validate artifacts before tagging.

## 0.1.6 - 2025-11-24
- Switched to supported archive options in the release workflow for better cross-platform packaging.

## 0.1.5 - 2025-11-24
- Set the `ZIP` flag in the Windows upload step to produce valid Windows artifacts.

## 0.1.4 - 2025-11-24
- Fixed CI upload action inputs for Rust binaries.
- Stabilized core tests by fixing race conditions in environment variable handling.

## 0.1.3 - 2025-11-24
- The installer builds from source in an isolated cargo home as a secondary option when release assets are missing.
- Asset selection prioritizes `jq`, with `awk` as a secondary option, and includes optional `pipefail` for POSIX shells.
- Release workflow asset names use the action template placeholder (`{{ target }}`) to publish platform tarballs reliably.

## 0.1.2 - 2025-11-24
- Added project icon at `assets/icon.png` and linked it in the README.
- Installer docs use branch-agnostic `/HEAD/` URLs.
- Makefile `book` target builds and opens the mdBook.
- Added comparison and FAQ chapters to the book.
- Release workflow assets now interpolate the `target` correctly; the audit workflow runs `cargo audit` directly.

## 0.1.1 - 2025-11-24
- Added a single-command installer that registers the Codex hook/MCP server by default.
- Introduced mdBook documentation and a GitHub Pages deployment workflow.
- Added a CI workflow (fmt, clippy, tests, docs, mdbook) and refined release asset naming.
- Expanded Makefile with docs/book targets and CLI demos.
- Documented child-process safety, cache TTL manifest option, and installer defaults.

## 0.1.0 - 2025-11-24
- Split the project into a Rust workspace: `crates/core` (library/MCP server) and `crates/cli` (binary wrapper).
- Added structured `_meta` outputs across tools with priority ranks and duplicate info.
- Synced `AGENTS.md` generation to include per-skill `priority_rank` and an overall priority list.
- Enhanced README with clearer installation/usage, universal sync, TUI, and structured output examples.
