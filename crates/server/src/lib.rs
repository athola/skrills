//! This crate provides the core functionality for the `skrills` application. It includes
//! the MCP server, skill discovery, caching, and command-line interface.
//!
//! The main entry point is the `run` function, which starts the server. The `runtime`
//! module provides tools for managing runtime options. Other parts of the crate are
//! considered internal and may change without notice.
//!
//! For information about stability and versioning, see `docs/semver-policy.md`.
//!
//! The `watch` feature enables filesystem watching for live cache invalidation. To
//! build without this feature, use `--no-default-features`.
//!
//! On Unix-like systems, a `SIGCHLD` handler is installed to prevent zombie processes.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect};
use flate2::{write::GzEncoder, Compression};
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use pathdiff::diff_paths;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ClientInfo, Content, InitializeResult,
    ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParam, RawResource,
    ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    Tool, ToolAnnotations,
};
use rmcp::service::serve_server;
use rmcp::transport;
use rmcp::ServerHandler;
use serde::Deserialize;
use serde_json::{json, Map as JsonMap};
use skrills_discovery::{
    default_priority, discover_skills, extract_refs_from_agents, hash_file, load_priority_override,
    priority_labels as disc_priority_labels,
    priority_labels_and_rank_map as disc_priority_labels_and_rank_map, Diagnostics, DuplicateInfo,
    SkillMeta, SkillRoot, SkillSource,
};
use skrills_state::{
    auto_pin_from_history, cache_ttl, env_auto_pin, env_diag, env_include_claude,
    env_manifest_first, env_manifest_minimal, env_max_bytes, env_render_mode_log,
    extra_dirs_from_env, home_dir, load_auto_pin_flag, load_history, load_manifest_settings,
    load_pinned, print_history, save_auto_pin_flag, save_history, save_pinned, HistoryEntry,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{IsTerminal, Write};
#[cfg(unix)]
use std::mem;
use std::path::{Path, PathBuf};
use std::pin::Pin;
#[cfg(unix)]
use std::ptr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::runtime::Runtime;
use walkdir::WalkDir;

pub mod runtime;
use runtime::{runtime_overrides_cached, RuntimeOverrides};

// Resource IDs and manifest markers
const AGENTS_URI: &str = "doc://agents";
const AGENTS_NAME: &str = "AGENTS.md";
const AGENTS_DESCRIPTION: &str = "AI Agent Development Guidelines";
const AGENTS_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../AGENTS.md"));
const ENV_EXPOSE_AGENTS: &str = "SKRILLS_EXPOSE_AGENTS";
const AGENTS_SECTION_START: &str = "<!-- available_skills:start -->";
const AGENTS_SECTION_END: &str = "<!-- available_skills:end -->";
const DEFAULT_EMBED_PREVIEW_BYTES: usize = 4096;
const DEFAULT_EMBED_THRESHOLD: f32 = 0.18;

/// Convenience re-exports for priority labels and ranks.
fn priority_labels() -> Vec<String> {
    disc_priority_labels()
}

fn priority_labels_and_rank_map() -> (Vec<String>, JsonMap<String, serde_json::Value>) {
    disc_priority_labels_and_rank_map()
}

#[derive(Debug, Parser)]
#[command(
    name = "skrills",
    about = "MCP server exposing local SKILL.md files for Codex"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run as MCP server over stdio
    Serve {
        /// Additional skill directories (repeatable)
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Cache TTL for skill discovery in milliseconds (overrides env SKRILLS_CACHE_TTL_MS)
        #[arg(long = "cache-ttl-ms", value_name = "MILLIS")]
        cache_ttl_ms: Option<u64>,
        /// Dump raw MCP initialize traffic (stdin/stdout) as hex+UTF8 for debugging
        #[arg(long, env = "SKRILLS_TRACE_WIRE", default_value_t = false)]
        trace_wire: bool,
        #[cfg(feature = "watch")]
        /// Watch filesystem for changes and invalidate caches immediately
        #[arg(long, default_value_t = false)]
        watch: bool,
    },
    /// List discovered skills (debug)
    #[command(alias = "list-skills")]
    List,
    /// List pinned skills
    ListPinned,
    /// Pin one or more skills by name (substring match allowed if unique)
    Pin {
        /// Skill names or unique substrings to pin
        #[arg(required = true)]
        skills: Vec<String>,
    },
    /// Unpin specific skills or everything with --all
    Unpin {
        /// Skill names or unique substrings to unpin (ignored when --all is set)
        skills: Vec<String>,
        /// Remove every pinned skill
        #[arg(long)]
        all: bool,
    },
    /// Enable or disable heuristic auto-pinning
    AutoPin {
        /// Set to true to enable, false to disable
        #[arg(long)]
        enable: bool,
    },
    /// Show recent autoload match history
    History {
        /// Limit number of entries shown (most recent first)
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Generate <available_skills> section in AGENTS.md for non-MCP agents
    SyncAgents {
        /// Optional path to AGENTS.md (default: ./AGENTS.md)
        #[arg(long)]
        path: Option<PathBuf>,
        /// Additional skill directories (repeatable)
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
    /// Emit hook JSON for autoload
    EmitAutoload {
        /// Include ~/.claude skills in autoload output
        #[arg(long, default_value_t = env_include_claude())]
        include_claude: bool,
        /// Maximum bytes of additionalContext payload
        #[arg(long)]
        max_bytes: Option<usize>,
        /// Prompt text to filter relevant skills (optional; uses env SKRILLS_PROMPT if not provided)
        #[arg(long)]
        prompt: Option<String>,
        /// Embedding similarity threshold (0-1) for fuzzy prompt matching
        #[arg(long)]
        embed_threshold: Option<f32>,
        /// Enable heuristic auto-pinning based on recent prompt matches
        #[arg(long, default_value_t = env_auto_pin_default())]
        auto_pin: bool,
        /// Additional skill directories (repeatable)
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
        /// Emit diagnostics (included skills + skips)
        #[arg(long, default_value_t = env_diag())]
        diagnose: bool,
    },
    /// Copy skills from ~/.claude into ~/.codex/skills-mirror
    Sync,
    /// Diagnose Codex MCP configuration for this server
    Doctor,
    /// Interactive TUI for sync and pin management
    Tui {
        /// Additional skill directories (repeatable)
        #[arg(long = "skill-dir", value_name = "DIR")]
        skill_dirs: Vec<PathBuf>,
    },
}

/// Reports the outcome of a synchronization operation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SyncReport {
    copied: usize,
    skipped: usize,
}

type StdioReader = Pin<Box<dyn AsyncRead + Unpin + Send + 'static>>;
type StdioWriter = Pin<Box<dyn AsyncWrite + Unpin + Send + 'static>>;

/// Wrap stdio transport with optional wire tracing for debugging Codex MCP handshakes.
fn stdio_with_optional_trace(trace: bool) -> (StdioReader, StdioWriter) {
    let (stdin, stdout) = transport::stdio();
    if !trace {
        return (Box::pin(stdin), Box::pin(stdout));
    }

    (
        Box::pin(LoggingReader {
            inner: stdin,
            label: "in",
        }),
        Box::pin(LoggingWriter {
            inner: stdout,
            label: "out",
        }),
    )
}

/// Defines the directories searched for SKILL.md files, in priority order.
fn skill_roots(extra_dirs: &[PathBuf]) -> Result<Vec<SkillRoot>> {
    let home = home_dir()?;
    let order = {
        if let Some(mut override_list) =
            load_priority_override(&|| Ok(load_manifest_settings()?.priority.clone()))?
        {
            let mut seen: std::collections::HashSet<String> =
                override_list.iter().map(|s| s.label()).collect();
            for src in default_priority() {
                if seen.insert(src.label()) {
                    override_list.push(src);
                }
            }
            override_list
        } else {
            default_priority()
        }
    };
    let mut roots = Vec::new();
    for source in order {
        let root = match source {
            SkillSource::Codex => home.join(".codex/skills"),
            SkillSource::Claude => home.join(".claude/skills"),
            SkillSource::Mirror => home.join(".codex/skills-mirror"),
            SkillSource::Agent => home.join(".agent/skills"),
            SkillSource::Extra(_) => continue,
        };
        roots.push(SkillRoot { root, source });
    }
    for (idx, dir) in extra_dirs.iter().enumerate() {
        roots.push(SkillRoot {
            root: dir.clone(),
            source: SkillSource::Extra(idx as u32),
        });
    }
    Ok(roots)
}

fn env_auto_pin_default() -> bool {
    env_auto_pin(load_auto_pin_flag().unwrap_or(false))
}

fn env_embed_threshold() -> f32 {
    std::env::var("SKRILLS_EMBED_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(DEFAULT_EMBED_THRESHOLD)
}

fn merge_extra_dirs(cli_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = extra_dirs_from_env();
    dirs.extend(cli_dirs.iter().cloned());
    dirs
}

fn agents_manifest() -> Result<Option<PathBuf>> {
    let path = home_dir()?.join(".codex/AGENTS.md");
    if path.exists() {
        return Ok(Some(path));
    }
    let local = std::env::current_dir()?.join("AGENTS.md");
    if local.exists() {
        return Ok(Some(local));
    }
    Ok(None)
}

/// Resolves a user-provided specification (either an exact skill name or a unique substring) to a full skill name.
fn resolve_skill<'a>(spec: &str, skills: &'a [SkillMeta]) -> Result<&'a str> {
    let spec_l = spec.to_ascii_lowercase();
    let mut matches: Vec<&str> = skills
        .iter()
        .map(|s| s.name.as_str())
        .filter(|name| {
            let nl = name.to_ascii_lowercase();
            nl == spec_l || nl.contains(&spec_l)
        })
        .collect();
    matches.sort_unstable();
    matches.dedup();
    match matches.len() {
        0 => Err(anyhow!("skill not found for spec: {spec}")),
        1 => Ok(matches[0]),
        _ => Err(anyhow!(
            "spec '{spec}' is ambiguous (matches: {})",
            matches.join(", ")
        )),
    }
}

fn is_skill_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file() && entry.file_name() == "SKILL.md"
}

/// Collects skills from all configured roots.
fn collect_skills(extra_dirs: &[PathBuf]) -> Result<Vec<SkillMeta>> {
    discover_skills(&skill_roots(extra_dirs)?, None)
}

/// Reads a SKILL.md file to string.
fn read_skill(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)?)
}

fn tokenize_prompt(prompt: &str) -> std::collections::HashSet<String> {
    prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn trigram_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let normalized = text.to_ascii_lowercase();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() < 3 {
        return counts;
    }
    for window in chars.windows(3) {
        let gram: String = window.iter().collect();
        *counts.entry(gram).or_insert(0) += 1;
    }
    counts
}

fn cosine_similarity(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot = 0f32;
    let mut norm_a = 0f32;
    let mut norm_b = 0f32;
    for (gram, &count) in a.iter() {
        norm_a += (count as f32).powi(2);
        if let Some(&b_count) = b.get(gram) {
            dot += (count as f32) * (b_count as f32);
        }
    }
    for &count in b.values() {
        norm_b += (count as f32).powi(2);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

fn trigram_similarity(prompt: &str, text: &str) -> f32 {
    let prompt_vec = trigram_counts(prompt);
    let text_vec = trigram_counts(text);
    cosine_similarity(&prompt_vec, &text_vec)
}

#[cfg(test)]
static EMBED_SIM_OVERRIDE: once_cell::sync::Lazy<Mutex<Option<f32>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

#[cfg(test)]
struct EmbedOverrideGuard;

#[cfg(test)]
impl EmbedOverrideGuard {
    fn set(value: f32) -> Self {
        if let Ok(mut guard) = EMBED_SIM_OVERRIDE.lock() {
            *guard = Some(value);
        }
        Self
    }
}

#[cfg(test)]
impl Drop for EmbedOverrideGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = EMBED_SIM_OVERRIDE.lock() {
            *guard = None;
        }
    }
}

fn trigram_similarity_checked(prompt: &str, text: &str) -> f32 {
    #[cfg(test)]
    {
        if let Ok(guard) = EMBED_SIM_OVERRIDE.lock() {
            if let Some(v) = *guard {
                return v;
            }
        }
    }
    trigram_similarity(prompt, text)
}

fn read_prefix(path: &Path, max: usize) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; max];
    let n = file.read(&mut buf)?;
    buf.truncate(n);
    Ok(String::from_utf8_lossy(&buf).to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum RenderMode {
    /// Emit manifest plus full content (backward compatible).
    #[default]
    Dual,
    /// Emit only manifest (for manifest-capable clients).
    ManifestOnly,
    /// Emit only concatenated content (legacy).
    ContentOnly,
}

#[derive(Default)]
struct AutoloadOptions<'p, 't, 'm, 'd> {
    include_claude: bool,
    max_bytes: Option<usize>,
    prompt: Option<&'p str>,
    embed_threshold: Option<f32>,
    preload_terms: Option<&'t HashSet<String>>,
    pinned: Option<&'t HashSet<String>>,
    matched: Option<&'m mut HashSet<String>>,
    diagnostics: Option<&'d mut Diagnostics>,
    render_mode: RenderMode,
    log_render_mode: bool,
    gzip_ok: bool,
    minimal_manifest: bool,
}

#[derive(Default)]
struct PreviewStats {
    matched: Vec<String>,
    manifest_bytes: usize,
    estimated_tokens: usize,
    truncated: bool,
    truncated_content: bool,
}

/// Concatenates skills into an autoload payload, with optional prompt-based filtering and truncation.
fn render_autoload_with_reader<F, G>(
    skills: &[SkillMeta],
    opts: AutoloadOptions<'_, '_, '_, '_>,
    read_full: F,
    read_prefix: G,
) -> Result<String>
where
    F: Fn(&SkillMeta) -> Result<String>,
    G: Fn(&SkillMeta, usize) -> Result<String>,
{
    let AutoloadOptions {
        include_claude,
        max_bytes,
        prompt,
        embed_threshold,
        preload_terms,
        pinned,
        mut matched,
        mut diagnostics,
        render_mode,
        log_render_mode,
        gzip_ok,
        minimal_manifest,
    } = opts;

    let mut terms = prompt.map(tokenize_prompt).unwrap_or_default();
    if let Some(extra) = preload_terms {
        terms.extend(extra.iter().cloned());
    }
    let term_opt = if terms.is_empty() { None } else { Some(terms) };
    let prompt_for_embedding = prompt.unwrap_or_default();
    let embed_threshold = embed_threshold.unwrap_or_else(env_embed_threshold);

    let preview_len = max_bytes
        .map(|m| m.saturating_div(4).clamp(64, 512))
        .unwrap_or(512);

    let mut manifest = Vec::new();
    let mut buf = String::new();
    for meta in skills.iter().filter(|s| match s.source {
        SkillSource::Codex => true,
        SkillSource::Mirror => {
            include_claude || pinned.map(|set| set.contains(&s.name)).unwrap_or(false)
        }
        SkillSource::Claude => {
            include_claude || pinned.map(|set| set.contains(&s.name)).unwrap_or(false)
        }
        SkillSource::Agent => true,
        SkillSource::Extra(_) => true,
    }) {
        let is_pinned = pinned.map(|set| set.contains(&meta.name)).unwrap_or(false);
        let mut prefix_cache: Option<String> = None;
        let relevant = match &term_opt {
            None => true,
            Some(_) if is_pinned => true,
            Some(t) => {
                let name = meta.name.to_ascii_lowercase();
                if t.iter().any(|k| name.contains(k)) {
                    true
                } else {
                    let prefix = prefix_cache.get_or_insert_with(|| {
                        read_prefix(meta, DEFAULT_EMBED_PREVIEW_BYTES)
                            .unwrap_or_else(|_| String::new())
                    });
                    let text = prefix.to_ascii_lowercase();
                    if t.iter().any(|k| text.contains(k)) {
                        true
                    } else {
                        let sim = trigram_similarity_checked(prompt_for_embedding, &text);
                        sim >= embed_threshold
                    }
                }
            }
        };
        if !relevant {
            if let Some(d) = diagnostics.as_deref_mut() {
                d.skipped
                    .push((meta.name.clone(), "filtered by prompt".into()));
            }
            continue;
        }

        if let Some(set) = matched.as_deref_mut() {
            set.insert(meta.name.clone());
        }
        if let Some(d) = diagnostics.as_deref_mut() {
            d.included.push((
                meta.name.clone(),
                meta.source.label(),
                meta.root.display().to_string(),
                meta.source.location().to_string(),
            ));
        }

        if minimal_manifest {
            manifest.push(json!({
                "name": meta.name,
                "source": meta.source,
                "hash": meta.hash,
            }));
        } else {
            let preview = read_prefix(meta, preview_len).unwrap_or_else(|_| String::new());
            manifest.push(json!({
                "name": meta.name,
                "source": meta.source,
                "root": meta.root,
                "path": meta.path,
                "hash": meta.hash,
                "preview": preview
            }));
        }

        // Append full content after the manifest to preserve existing behavior.
        if let Ok(text) = read_full(meta) {
            buf.push_str(&format!("\n\n# {}\n\n{}", meta.name, text));
        }
    }

    let include_manifest = !matches!(render_mode, RenderMode::ContentOnly);
    let include_content = !matches!(render_mode, RenderMode::ManifestOnly);

    let manifest_json = if include_manifest {
        serde_json::to_string_pretty(&json!({ "skills_manifest": manifest })).unwrap_or_default()
    } else {
        String::new()
    };
    let names: Vec<String> = manifest
        .iter()
        .filter_map(|v| {
            v.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let mut output = String::new();
    if include_manifest && !manifest_json.is_empty() {
        if !names.is_empty() {
            output.push_str(&format!("[skills] {}\n", names.join(", ")));
        }
        output.push_str(&manifest_json);
    }
    if include_content && !buf.is_empty() {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(buf.trim());
    }
    if let Some(d) = diagnostics.as_deref_mut() {
        d.render_mode = Some(format!("{:?}", render_mode));
    }
    if log_render_mode && diagnostics.is_some() {
        // Avoid per-request log spam; only log when diagnostics are requested.
        tracing::info!(
            render_mode = ?render_mode,
            include_manifest,
            include_content,
            "autoload render mode selected"
        );
    } else if log_render_mode {
        tracing::debug!(
            render_mode = ?render_mode,
            include_manifest,
            include_content,
            "autoload render mode selected"
        );
    }
    if let Some(d) = diagnostics.as_deref_mut() {
        if !d.included.is_empty() {
            let mut footer = String::from("\n\n[activated skills]\n");
            for (name, src, _, loc) in d.included.iter() {
                footer.push_str(&format!("- {} ({}; location: {})\n", name, src, loc));
            }
            output.push_str(&footer);
        }
    }

    if let Some(limit) = max_bytes {
        if output.len() > limit {
            if include_manifest && !manifest_json.is_empty() && manifest_json.len() <= limit {
                // Prefer returning a valid manifest and signal content was dropped.
                output = manifest_json.clone();
                if let Some(d) = diagnostics.as_deref_mut() {
                    d.truncated = true;
                    d.truncated_content = true;
                }
            } else if include_manifest && gzip_ok {
                let gz = gzip_base64(&manifest_json)?;
                let gz_wrapped = format!(r#"{{"skills_manifest_gzip_base64":"{}"}}"#, gz);
                if gz_wrapped.len() <= limit {
                    output = gz_wrapped;
                    if let Some(d) = diagnostics.as_deref_mut() {
                        d.truncated = true;
                        d.truncated_content = true;
                    }
                } else {
                    return Err(anyhow!(
                        "autoload payload exceeds byte limit (even gzipped manifest)"
                    ));
                }
            } else {
                return Err(anyhow!("autoload payload exceeds byte limit"));
            }
        }
    }
    if let Some(d) = diagnostics {
        let mut header = String::from("<!-- diagnostics:\n");
        if !d.included.is_empty() {
            header.push_str("included:\n");
            for (name, src, root, loc) in d.included.iter() {
                header.push_str(&format!(
                    "- {} ({} from {} location={})\n",
                    name, src, root, loc
                ));
            }
        }
        if !d.duplicates.is_empty() {
            header.push_str("duplicates (kept highest priority, skipped others):\n");
            for dup in d.duplicates.iter() {
                header.push_str(&format!(
                    "- {} skipped {} ({}) kept {} ({})\n",
                    dup.name, dup.skipped_source, dup.skipped_root, dup.kept_source, dup.kept_root
                ));
            }
        }
        if !d.skipped.is_empty() {
            header.push_str("skipped:\n");
            for (name, reason) in d.skipped.iter() {
                header.push_str(&format!("- {} [{}]\n", name, reason));
            }
        }
        if d.truncated {
            header.push_str("note: output truncated\n");
        }
        if d.truncated_content {
            header.push_str("note: content omitted to fit manifest size\n");
        }
        header.push_str("-->\n");
        output = format!("{header}{output}");
    }
    Ok(output)
}

fn render_autoload(skills: &[SkillMeta], opts: AutoloadOptions<'_, '_, '_, '_>) -> Result<String> {
    render_autoload_with_reader(
        skills,
        opts,
        |meta| read_skill(&meta.path),
        |meta, max| read_prefix(&meta.path, max),
    )
}

fn render_preview_stats(
    skills: &[SkillMeta],
    opts: AutoloadOptions<'_, '_, '_, '_>,
) -> Result<PreviewStats> {
    let mut matched = HashSet::new();
    let mut diagnostics = Diagnostics::default();
    let content = render_autoload_with_reader(
        skills,
        AutoloadOptions {
            render_mode: RenderMode::ManifestOnly,
            log_render_mode: false,
            gzip_ok: false,
            matched: Some(&mut matched),
            diagnostics: Some(&mut diagnostics),
            ..opts
        },
        |meta| read_skill(&meta.path),
        |meta, max| read_prefix(&meta.path, max),
    )?;
    let manifest_bytes = content.len();
    let estimated_tokens = manifest_bytes.div_ceil(4); // rough UTF-8→token estimate
    let mut matched_vec: Vec<String> = matched.into_iter().collect();
    matched_vec.sort();
    Ok(PreviewStats {
        matched: matched_vec,
        manifest_bytes,
        estimated_tokens,
        truncated: diagnostics.truncated,
        truncated_content: diagnostics.truncated_content,
    })
}

/// Decide render mode based on env + client identity.
fn manifest_render_mode(runtime: &RuntimeOverrides, peer_info: Option<&ClientInfo>) -> RenderMode {
    // If explicitly disabled, stick to legacy content-only.
    if !runtime.manifest_first() {
        return RenderMode::ContentOnly;
    }

    if let Some(info) = peer_info {
        if manifest_allowlist_match(info) {
            return RenderMode::ManifestOnly;
        }
    }

    let manifest_capable = peer_info.map(|info| {
        let name = info.client_info.name.to_ascii_lowercase();
        name.contains("claude") || name.contains("anthropic")
    });

    match manifest_capable {
        Some(true) => RenderMode::ManifestOnly,
        _ => RenderMode::Dual,
    }
}

/// Whether the peer can accept gzipped manifest payloads.
fn peer_accepts_gzip(peer_info: Option<&ClientInfo>) -> bool {
    if std::env::var("SKRILLS_ACCEPT_GZIP")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(info) = peer_info {
        let name = info.client_info.name.to_ascii_lowercase();
        if name.contains("gzip") {
            return true;
        }
    }
    false
}

/// Allowlist-driven manifest capability detection.
/// Optional JSON file: SKRILLS_MANIFEST_ALLOWLIST
/// Format: [{"name_substr": "codex", "min_version": "1.2.3"}]
fn manifest_allowlist_match(info: &ClientInfo) -> bool {
    if let Some(entries) = ALLOWLIST_CACHE.get_or_init() {
        let name_lc = info.client_info.name.to_ascii_lowercase();
        for item in entries {
            if !name_lc.contains(&item.name_substr) {
                continue;
            }
            if let Some(min) = &item.min_version {
                if !version_gte(&info.client_info.version, min) {
                    continue;
                }
            }
            return true;
        }
    }
    false
}

/// Minimal dotted version comparison (numeric segments only; non-numeric treated as 0).
fn version_gte(current: &str, min: &str) -> bool {
    match (semver::Version::parse(current), semver::Version::parse(min)) {
        (Ok(c), Ok(m)) => c >= m,
        _ => false,
    }
}

fn gzip_base64(data: &str) -> Result<String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data.as_bytes())?;
    let compressed = encoder.finish()?;
    Ok(BASE64.encode(compressed))
}

#[derive(Clone)]
struct AllowlistEntry {
    name_substr: String,
    min_version: Option<String>,
}

struct AllowlistCache {
    inner: Mutex<Option<Vec<AllowlistEntry>>>,
}

impl AllowlistCache {
    fn get_or_init(&self) -> Option<Vec<AllowlistEntry>> {
        let mut guard = self.inner.lock().ok()?;
        if guard.is_none() {
            *guard = load_allowlist();
        }
        guard.clone()
    }
}

static ALLOWLIST_CACHE: once_cell::sync::Lazy<AllowlistCache> =
    once_cell::sync::Lazy::new(|| AllowlistCache {
        inner: Mutex::new(None),
    });

#[cfg(test)]
fn reset_allowlist_cache_for_tests() {
    if let Ok(mut guard) = ALLOWLIST_CACHE.inner.lock() {
        *guard = None;
    }
}

fn load_allowlist() -> Option<Vec<AllowlistEntry>> {
    let path = std::env::var("SKRILLS_MANIFEST_ALLOWLIST").ok()?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "failed to read manifest allowlist");
            return None;
        }
    };
    let val: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "failed to parse manifest allowlist JSON");
            return None;
        }
    };
    let arr = match val.as_array() {
        Some(a) => a,
        None => {
            tracing::warn!(path = %path, "manifest allowlist is not an array");
            return None;
        }
    };
    let mut entries = Vec::new();
    for item in arr {
        let Some(sub) = item.get("name_substr").and_then(|v| v.as_str()) else {
            continue;
        };
        let min = item
            .get("min_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        entries.push(AllowlistEntry {
            name_substr: sub.to_ascii_lowercase(),
            min_version: min,
        });
    }
    Some(entries)
}

/// Mirrors SKILL.md files from `~/.claude` into a Codex-owned mirror tree.
fn sync_from_claude(claude_root: &Path, mirror_root: &Path) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    if !claude_root.exists() {
        return Ok(report);
    }
    for entry in WalkDir::new(claude_root)
        .min_depth(1)
        .max_depth(6)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !is_skill_file(&entry) {
            continue;
        }
        let src = entry.into_path();
        let rel = diff_paths(&src, claude_root).unwrap_or_else(|| src.clone());
        let dest = mirror_root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let should_copy = if dest.exists() {
            hash_file(&dest)? != hash_file(&src)?
        } else {
            true
        };
        if should_copy {
            fs::copy(&src, &dest)?;
            report.copied += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}

/// In-memory cache for discovered skills to avoid repeated directory walks.
struct SkillCache {
    roots: Vec<SkillRoot>,
    ttl: Duration,
    last_scan: Option<Instant>,
    skills: Vec<SkillMeta>,
    duplicates: Vec<DuplicateInfo>,
    uri_index: HashMap<String, usize>,
}

impl SkillCache {
    #[allow(dead_code)]
    fn new(roots: Vec<SkillRoot>) -> Self {
        Self::new_with_ttl(roots, cache_ttl(&load_manifest_settings))
    }

    fn new_with_ttl(roots: Vec<SkillRoot>, ttl: Duration) -> Self {
        Self {
            roots,
            ttl,
            last_scan: None,
            skills: Vec::new(),
            duplicates: Vec::new(),
            uri_index: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns the paths of the root directories being watched.
    fn watched_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.root.clone()).collect()
    }

    /// Invalidates the cache, forcing a rescan on the next access.
    fn invalidate(&mut self) {
        self.last_scan = None;
    }

    /// Refreshes cached skills if the TTL has expired or cache is empty.
    fn refresh_if_stale(&mut self) -> Result<()> {
        let now = Instant::now();
        let fresh = self
            .last_scan
            .map(|ts| now.duration_since(ts) < self.ttl)
            .unwrap_or(false);
        if fresh {
            return Ok(());
        }

        let scan_started = Instant::now();
        let mut dup_log = Vec::new();
        let skills = discover_skills(&self.roots, Some(&mut dup_log))?;
        let mut uri_index = HashMap::new();
        for (idx, s) in skills.iter().enumerate() {
            uri_index.insert(format!("skill://{}/{}", s.source.label(), s.name), idx);
        }
        self.skills = skills;
        self.duplicates = dup_log;
        self.uri_index = uri_index;
        self.last_scan = Some(now);
        let elapsed_ms = scan_started.elapsed().as_millis();
        if elapsed_ms > 250 {
            tracing::info!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        } else {
            tracing::debug!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        }
        Ok(())
    }

    /// Returns the current list of skills and any recorded duplicate information.
    fn skills_with_dups(&mut self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        self.refresh_if_stale()?;
        Ok((self.skills.clone(), self.duplicates.clone()))
    }

    /// Retrieves a skill by its URI.
    fn get_by_uri(&mut self, uri: &str) -> Result<SkillMeta> {
        self.refresh_if_stale()?;
        if let Some(idx) = self.uri_index.get(uri).copied() {
            return Ok(self.skills[idx].clone());
        }
        Err(anyhow!("skill not found"))
    }
}

/// In-memory cache for SKILL.md contents keyed by (path, hash).
#[derive(Default)]
struct ContentCache {
    by_path: HashMap<PathBuf, (String, String)>, // path -> (hash, contents)
}

impl ContentCache {
    /// Reads the full content of a skill, utilizing the cache if available.
    fn read_full(&mut self, meta: &SkillMeta) -> Result<String> {
        if let Some((hash, text)) = self.by_path.get(&meta.path) {
            if hash == &meta.hash {
                return Ok(text.clone());
            }
        }
        let text = read_skill(&meta.path)?;
        self.by_path
            .insert(meta.path.clone(), (meta.hash.clone(), text.clone()));
        Ok(text)
    }

    /// Reads a specified prefix of a skill's content, utilizing the cache.
    fn read_prefix(&mut self, meta: &SkillMeta, max: usize) -> Result<String> {
        let text = self.read_full(meta)?;
        if text.len() <= max {
            return Ok(text);
        }
        let mut bytes = text.into_bytes();
        bytes.truncate(max);
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }
}

/// Manages and serves skills via the RMCP (Remote Method Call Protocol) server.
///
/// This service provides an interface for discovering, caching, and interacting with skills.
/// It maintains in-memory caches for skill metadata and content to optimize performance.
struct SkillService {
    cache: Arc<Mutex<SkillCache>>,
    content_cache: Arc<Mutex<ContentCache>>,
    warmup_started: AtomicBool,
    runtime: Arc<Mutex<RuntimeOverrides>>,
}

/// Logs stdin/stdout traffic for debugging MCP handshakes.
struct LoggingReader<R> {
    inner: R,
    label: &'static str,
}

struct LoggingWriter<W> {
    inner: W,
    label: &'static str,
}

impl<R: AsyncRead + Unpin> AsyncRead for LoggingReader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            let after = buf.filled().len();
            let read = after.saturating_sub(before);
            if read > 0 {
                let bytes = &buf.filled()[after - read..after];
                tracing::debug!(
                    target: "skrills::wire",
                    dir = self.label,
                    len = read,
                    hex = %hex::encode(bytes),
                    text = %String::from_utf8_lossy(bytes),
                    "wire read"
                );
            }
        }
        poll
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for LoggingWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(written)) = &poll {
            if *written > 0 {
                let bytes = &buf[..*written];
                tracing::debug!(
                    target: "skrills::wire",
                    dir = self.label,
                    len = *written,
                    hex = %hex::encode(bytes),
                    text = %String::from_utf8_lossy(bytes),
                    "wire write"
                );
            }
        }
        poll
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Starts a filesystem watcher to invalidate caches when skill files change.
#[cfg(feature = "watch")]
fn start_fs_watcher(service: &SkillService) -> Result<RecommendedWatcher> {
    let cache = service.cache.clone();
    let content_cache = service.content_cache.clone();
    let roots = {
        let guard = cache
            .lock()
            .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
        guard.watched_roots()
    };

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                if let Ok(mut cache) = cache.lock() {
                    cache.invalidate();
                }
                if let Ok(mut content) = content_cache.lock() {
                    content.by_path.clear();
                }
            }
        },
        NotifyConfig::default(),
    )?;

    for root in roots {
        if root.exists() {
            watcher.watch(root.as_path(), RecursiveMode::Recursive)?;
        }
    }

    Ok(watcher)
}

/// Placeholder for the filesystem watcher when the 'watch' feature is disabled.
/// Returns an error if called.
#[cfg(not(feature = "watch"))]
fn start_fs_watcher(_service: &SkillService) -> Result<()> {
    Err(anyhow!(
        "watch feature is disabled; rebuild with --features watch"
    ))
}

impl SkillService {
    /// Builds a skill service with the default search roots.
    #[allow(dead_code)]
    fn new(extra_dirs: Vec<PathBuf>) -> Result<Self> {
        Self::new_with_ttl(extra_dirs, cache_ttl(&load_manifest_settings))
    }

    /// Builds a skill service with a custom cache TTL.
    fn new_with_ttl(extra_dirs: Vec<PathBuf>, ttl: Duration) -> Result<Self> {
        let build_started = Instant::now();
        let roots = skill_roots(&extra_dirs)?;
        let elapsed_ms = build_started.elapsed().as_millis();
        tracing::info!(
            target: "skrills::startup",
            elapsed_ms,
            roots = roots.len(),
            "constructed SkillService (discovery deferred until after initialize)"
        );
        Ok(Self {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(roots, ttl))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::load()?)),
        })
    }

    /// Clears metadata and content caches; next access will rescan.
    fn invalidate_cache(&self) -> Result<()> {
        if let Ok(mut cache) = self.cache.lock() {
            cache.invalidate();
        }
        if let Ok(mut content) = self.content_cache.lock() {
            content.by_path.clear();
        }
        Ok(())
    }

    /// Returns current skills plus duplicate log (winner/loser by priority).
    fn current_skills_with_dups(&self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
        cache.skills_with_dups()
    }

    /// Builds the MCP `listResources` payload.
    fn list_resources_payload(&self) -> Result<Vec<Resource>> {
        let (skills, dup_log) = self.current_skills_with_dups()?;
        let mut resources: Vec<Resource> = skills
            .into_iter()
            .map(|s| {
                let uri = format!("skill://{}/{}", s.source.label(), s.name);
                let mut raw = RawResource::new(uri, s.name.clone());
                raw.description = Some(format!(
                    "Skill from {} [location: {}]",
                    s.source.label(),
                    s.source.location()
                ));
                raw.mime_type = Some("text/markdown".to_string());
                Resource::new(raw, None)
            })
            .collect();
        // Expose AGENTS.md guidelines as a first-class resource for clients, unless disabled.
        if self.expose_agents_doc()? {
            let mut agents = RawResource::new(AGENTS_URI, AGENTS_NAME);
            agents.description = Some(AGENTS_DESCRIPTION.to_string());
            agents.mime_type = Some("text/markdown".to_string());
            resources.insert(0, Resource::new(agents, None));
        }
        if !dup_log.is_empty() {
            for dup in dup_log {
                tracing::warn!(
                    "duplicate skill {} skipped from {} (winner: {})",
                    dup.name,
                    dup.skipped_source,
                    dup.kept_source
                );
            }
        }
        Ok(resources)
    }

    /// Reads a resource by URI, returning its contents.
    fn read_resource_sync(&self, uri: &str) -> Result<ReadResourceResult> {
        if uri == AGENTS_URI {
            if !self.expose_agents_doc()? {
                return Err(anyhow!("resource not found"));
            }
            return Ok(ReadResourceResult {
                contents: vec![text_with_location(AGENTS_TEXT, uri, None, "global")],
            });
        }
        if !uri.starts_with("skill://") {
            return Err(anyhow!("unsupported uri"));
        }
        let parts: Vec<&str> = uri.trim_start_matches("skill://").splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err(anyhow!("invalid uri"));
        }
        let uri = format!("skill://{}/{}", parts[0], parts[1]);
        let meta = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
            cache.get_by_uri(&uri)?
        };
        let text = self.read_skill_cached(&meta)?;
        Ok(ReadResourceResult {
            contents: vec![text_with_location(
                text,
                &uri,
                Some(&meta.source.label()),
                meta.source.location(),
            )],
        })
    }

    /// Reads the content of a skill from the cache.
    fn read_skill_cached(&self, meta: &SkillMeta) -> Result<String> {
        let mut cache = self
            .content_cache
            .lock()
            .map_err(|e| anyhow!("content cache poisoned: {e}"))?;
        cache.read_full(meta)
    }

    /// Reads a prefix of a skill's content from the cache.
    fn read_prefix_cached(&self, meta: &SkillMeta, max: usize) -> Result<String> {
        let mut cache = self
            .content_cache
            .lock()
            .map_err(|e| anyhow!("content cache poisoned: {e}"))?;
        cache.read_prefix(meta, max)
    }

    /// Renders an autoload snippet, using cached skill content.
    fn render_autoload_cached(
        &self,
        skills: &[SkillMeta],
        opts: AutoloadOptions<'_, '_, '_, '_>,
    ) -> Result<String> {
        render_autoload_with_reader(
            skills,
            opts,
            |meta| self.read_skill_cached(meta),
            |meta, max| self.read_prefix_cached(meta, max),
        )
    }

    fn runtime_overrides(&self) -> RuntimeOverrides {
        self.runtime
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Determines whether the AGENTS.md document should be exposed as a resource.
    fn expose_agents_doc(&self) -> Result<bool> {
        let manifest = load_manifest_settings()?;
        if let Some(flag) = manifest.expose_agents {
            return Ok(flag);
        }
        if let Ok(val) = std::env::var(ENV_EXPOSE_AGENTS) {
            if let Ok(parsed) = val.parse::<bool>() {
                return Ok(parsed);
            }
        }
        // Legacy/edge: explicit manifest JSON without manifest schema parsing.
        if let Ok(custom) = std::env::var("SKRILLS_MANIFEST") {
            if let Ok(text) = fs::read_to_string(&custom) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(flag) = val.get("expose_agents").and_then(|v| v.as_bool()) {
                        return Ok(flag);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Kicks off a background cache warm-up after `initialize` returns, so startup
    /// handshake stays fast even with large skill trees. The warm-up is best-effort
    /// and logs its duration for diagnostics.
    fn spawn_warmup_if_needed(&self) {
        if self.warmup_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let cache = self.cache.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = cache
                .lock()
                .map_err(|e| anyhow!("skill cache poisoned: {e}"))
                .and_then(|mut cache| cache.refresh_if_stale());

            match result {
                Ok(()) => tracing::info!(
                    target: "skrills::warmup",
                    elapsed_ms = started.elapsed().as_millis(),
                    "background cache warm-up finished"
                ),
                Err(e) => tracing::warn!(
                    target: "skrills::warmup",
                    error = %e,
                    "background cache warm-up failed"
                ),
            }
        });
    }
}

/// Inserts location and optional priority rank into readResource responses.
fn text_with_location(
    text: impl Into<String>,
    uri: &str,
    source_label: Option<&str>,
    location: &str,
) -> ResourceContents {
    let mut meta = Meta::new();
    meta.insert("location".into(), json!(location));
    if let Some(label) = source_label {
        if let Some(rank) = priority_labels()
            .iter()
            .position(|p| p == label)
            .map(|i| i + 1)
        {
            meta.insert("priority_rank".into(), json!(rank));
        }
    }
    ResourceContents::TextResourceContents {
        uri: uri.into(),
        mime_type: Some("text".into()),
        text: text.into(),
        meta: Some(meta),
    }
}

/// Prints diagnostics about Codex MCP config entries for this server, helping pinpoint
/// the common "missing field `type`" startup error on the client side.
fn doctor_report() -> Result<()> {
    let home = home_dir()?;
    let mcp_path = home.join(".codex/mcp_servers.json");
    let cfg_path = home.join(".codex/config.toml");
    let expected_cmd = home.join(".codex/bin/skrills");

    println!("== skrills doctor ==");

    // Inspect ~/.codex/mcp_servers.json
    if mcp_path.exists() {
        let raw = fs::read_to_string(&mcp_path)?;
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(json) => {
                if let Some(entry) = json.get("mcpServers").and_then(|m| m.get("skrills")) {
                    let typ = entry
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<missing>");
                    let cmd = entry
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<missing>");
                    println!(
                        "mcp_servers.json: type={typ} command={cmd} args={:?} ({})",
                        entry.get("args").and_then(|v| v.as_array()),
                        mcp_path.display()
                    );
                    if typ != "stdio" {
                        println!("  ! expected type=\"stdio\"");
                    }
                    if Path::new(cmd) != expected_cmd {
                        println!(
                            "  i command differs; ensure binary path is correct and executable"
                        );
                    }
                    if !Path::new(cmd).exists() {
                        println!("  ! command path does not exist on disk");
                    }
                } else {
                    println!(
                        "mcp_servers.json: missing skrills entry ({})",
                        mcp_path.display()
                    );
                }
            }
            Err(e) => println!(
                "mcp_servers.json: failed to parse ({:?}): {}",
                mcp_path.display(),
                e
            ),
        }
    } else {
        println!("mcp_servers.json: not found at {}", mcp_path.display());
    }

    // Inspect ~/.codex/config.toml
    if cfg_path.exists() {
        let raw = fs::read_to_string(&cfg_path)?;
        match toml::from_str::<toml::Value>(&raw) {
            Ok(toml_val) => {
                let entry = toml_val.get("mcp_servers").and_then(|m| m.get("skrills"));
                if let Some(e) = entry {
                    let typ = e
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<missing>");
                    let cmd = e
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<missing>");
                    println!(
                        "config.toml:    type={typ} command={cmd} args={:?} ({})",
                        e.get("args"),
                        cfg_path.display()
                    );
                    if typ != "stdio" {
                        println!("  ! expected type=\"stdio\"");
                    }
                    if !Path::new(cmd).exists() {
                        println!("  ! command path does not exist on disk");
                    }
                } else {
                    println!(
                        "config.toml:    missing [mcp_servers.skrills] ({})",
                        cfg_path.display()
                    );
                }
            }
            Err(e) => println!(
                "config.toml:    failed to parse ({:?}): {}",
                cfg_path.display(),
                e
            ),
        }
    } else {
        println!("config.toml:    not found at {}", cfg_path.display());
    }

    println!("Hint: Codex CLI raises 'missing field `type`' when either file lacks type=\"stdio\" for skrills.");
    Ok(())
}

/// Simple interactive TUI for sync + pin management.
fn tui_flow(extra_dirs: &[PathBuf]) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(anyhow!("TUI requires a TTY"));
    }
    let theme = ColorfulTheme::default();
    // Optional sync
    if Confirm::with_theme(&theme)
        .with_prompt("Run claude → codex mirror sync first?")
        .default(false)
        .interact()?
    {
        let home = home_dir()?;
        let report = sync_from_claude(&home.join(".claude"), &home.join(".codex/skills-mirror"))?;
        println!(
            "Mirror sync complete (copied: {}, skipped: {})",
            report.copied, report.skipped
        );
    }

    let skills = collect_skills(extra_dirs)?;
    if skills.is_empty() {
        println!("No skills found.");
        return Ok(());
    }
    let pinned = load_pinned().unwrap_or_default();
    let mut items = Vec::new();
    let mut defaults = Vec::new();
    for s in skills.iter() {
        let display = format!(
            "[{} | {}] {}",
            s.source.label(),
            s.source.location(),
            s.name
        );
        items.push(display);
        defaults.push(pinned.contains(&s.name));
    }
    let selected = MultiSelect::with_theme(&theme)
        .with_prompt("Select skills to pin (space to toggle, enter to save)")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    let mut new_pins = HashSet::new();
    for idx in selected {
        new_pins.insert(skills[idx].name.clone());
    }
    save_pinned(&new_pins)?;
    println!("Pinned {} skills.", new_pins.len());
    Ok(())
}

/// Build a compact XML summary for AGENTS.md consumers (with priority + per-skill rank).
fn render_available_skills_xml(skills: &[SkillMeta]) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut out = String::from("<available_skills");
    out.push_str(&format!(" generated_at_utc=\"{}\"", ts));
    out.push_str(&format!(" priority=\"{}\"", priority_labels().join(",")));
    out.push_str(">\n");
    let priority_order = priority_labels();
    for s in skills {
        let rank = priority_order
            .iter()
            .position(|p| p == &s.source.label())
            .map(|i| i + 1)
            .unwrap_or(priority_order.len() + 1);
        out.push_str(&format!(
            "  <skill name=\"{}\" source=\"{}\" location=\"{}\" path=\"{}\" priority_rank=\"{}\" />\n",
            s.name,
            s.source.label(),
            s.source.location(),
            s.path.display(),
            rank
        ));
    }
    out.push_str("</available_skills>");
    out
}

/// Writes/updates the AGENTS.md available_skills section with current skills.
fn sync_agents(path: &Path, extra_dirs: &[PathBuf]) -> Result<()> {
    let skills = collect_skills(extra_dirs)?;
    sync_agents_with_skills(path, &skills)
}

fn sync_agents_with_skills(path: &Path, skills: &[SkillMeta]) -> Result<()> {
    let xml = render_available_skills_xml(skills);
    let section = format!(
        "{start}\n{xml}\n{end}\n",
        start = AGENTS_SECTION_START,
        xml = xml,
        end = AGENTS_SECTION_END
    );

    let content = if path.exists() {
        let mut existing = fs::read_to_string(path)?;
        if let (Some(start), Some(end)) = (
            existing.find(AGENTS_SECTION_START),
            existing.find(AGENTS_SECTION_END),
        ) {
            let end_idx = end + AGENTS_SECTION_END.len();
            existing.replace_range(start..end_idx, &section);
            existing
        } else {
            format!("{existing}\n\n{section}")
        }
    } else {
        // Seed with the shipped AGENTS.md text plus section.
        format!("{AGENTS_TEXT}\n\n{section}")
    };

    fs::write(path, content)?;
    Ok(())
}

impl ServerHandler for SkillService {
    /// Lists all available resources, including skills and the AGENTS.md document.
    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, rmcp::ErrorData>> + Send + '_
    {
        let result = self
            .list_resources_payload()
            .map(|resources| ListResourcesResult {
                resources,
                next_cursor: None,
            })
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
        std::future::ready(result)
    }

    /// Reads the content of a specific resource identified by its URI.
    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, rmcp::ErrorData>> + Send + '_
    {
        let result = self
            .read_resource_sync(&request.uri)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
        std::future::ready(result)
    }

    /// Lists the tools provided by this service.
    ///
    /// This implementation defines several tools for interacting with skills, including
    /// listing available skills, generating autoload snippets, synchronizing skills
    /// from external sources (e.g., Claude), and refreshing internal caches.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        // Codex CLI expects every tool input_schema to include a JSON Schema "type".
        // An empty map triggers "missing field `type`" during MCP → OpenAI conversion,
        // so we explicitly mark parameterless tools as taking an empty object.
        let mut schema_map = JsonMap::new();
        schema_map.insert("type".into(), json!("object"));
        schema_map.insert("properties".into(), json!({}));
        schema_map.insert("additionalProperties".into(), json!(false));
        let schema = std::sync::Arc::new(schema_map);
        let mut options_schema = JsonMap::new();
        options_schema.insert("type".into(), json!("object"));
        options_schema.insert(
            "properties".into(),
            json!({
                "manifest_first": { "type": "boolean" },
                "render_mode_log": { "type": "boolean" },
                "manifest_minimal": { "type": "boolean" }
            }),
        );
        options_schema.insert("additionalProperties".into(), json!(false));
        let options_schema = std::sync::Arc::new(options_schema);
        let tools = vec![
            Tool {
                name: "list-skills".into(),
                title: Some("List skills".into()),
                description: Some("List discovered SKILL.md files".into()),
                input_schema: schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "autoload-snippet".into(),
                title: Some("Autoload SKILL.md content".into()),
                description: Some("Concatenate SKILL.md markdown for prompt injection".into()),
                input_schema: schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "runtime-status".into(),
                title: Some("Runtime status".into()),
                description: Some("Show effective runtime overrides and sources".into()),
                input_schema: schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "render-preview".into(),
                title: Some("Preview selected skills with size estimates".into()),
                description: Some(std::borrow::Cow::Borrowed(
                    "Return matched skill names plus manifest/preview size and estimated tokens.",
                )),
                input_schema: schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "set-runtime-options".into(),
                title: Some("Set runtime options".into()),
                description: Some(
                    "Adjust manifest/logging overrides for autoload rendering".into(),
                ),
                input_schema: options_schema,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-from-claude".into(),
                title: Some("Copy ~/.claude skills into ~/.codex".into()),
                description: Some(
                    "Mirror SKILL.md files from ~/.claude into ~/.codex/skills-mirror".into(),
                ),
                input_schema: schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "refresh-cache".into(),
                title: Some("Refresh caches".into()),
                description: Some("Invalidate in-memory skill and content caches".into()),
                input_schema: schema,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
        ];
        std::future::ready(Ok(ListToolsResult {
            tools,
            next_cursor: None,
        }))
    }

    /// Executes a specific tool identified by `request.name`.
    ///
    /// This method dispatches to internal functions based on the tool name,
    /// such as listing skills, generating autoload snippets, synchronizing
    /// from Claude, or refreshing caches. It returns the result of the tool
    /// execution.
    fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        let result = || -> Result<CallToolResult> {
            match request.name.as_ref() {
                "list-skills" => {
                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    let skills_raw_with_rank: Vec<serde_json::Value> = skills
                        .iter()
                        .map(|s| {
                            let rank = rank_map
                                .get(&s.source.label())
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            json!({
                                "name": s.name,
                                "path": s.path,
                                "source": s.source,
                                "root": s.root,
                                "hash": s.hash,
                                "priority_rank": rank
                            })
                        })
                        .collect();
                    let mut skills_ranked = skills_raw_with_rank.clone();
                    skills_ranked
                        .sort_by_key(|v| v.get("priority_rank").and_then(|n| n.as_u64()).unwrap_or(u64::MAX));
                    let payload = json!({
                        "skills": skills_raw_with_rank,
                        "skills_ranked": skills_ranked,
                        "_meta": {
                            "duplicates": dup_log,
                            "priority": priority,
                            "priority_rank_by_source": rank_map
                        }
                    });
                    if !dup_log.is_empty() {
                        for dup in dup_log.iter() {
                            tracing::warn!(
                                "duplicate skill {} skipped from {} (winner: {})",
                                dup.name,
                                dup.skipped_source,
                                dup.kept_source
                            );
                        }
                    }
                    Ok(CallToolResult {
                        content: vec![Content::text(format!(
                            "listed skills{}",
                            if dup_log.is_empty() {
                                "".into()
                            } else {
                                format!(" ({} duplicates skipped)", dup_log.len())
                            }
                        ))],
                        structured_content: Some(payload),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "autoload-snippet" => {
                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    let skills_with_rank: Vec<serde_json::Value> = skills
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "path": s.path,
                                "source": s.source,
                                "root": s.root,
                                "hash": s.hash,
                                "priority_rank": rank_map.get(&s.source.label()).and_then(|v| v.as_u64()).unwrap_or(0)
                            })
                        })
                        .collect();
                    let args: AutoloadArgs = request
                        .arguments
                        .as_ref()
                        .map(|obj| {
                            serde_json::from_value(json!(obj.clone())).map_err(anyhow::Error::from)
                        })
                        .transpose()?
                        .unwrap_or_default();
                    let manual_pins = load_pinned().unwrap_or_default();
                    let history = load_history().unwrap_or_default();
                    let auto_pins = if args.auto_pin.unwrap_or(env_auto_pin_default()) {
                        auto_pin_from_history(&history)
                    } else {
                        HashSet::new()
                    };
                    let mut effective_pins = manual_pins.clone();
                    effective_pins.extend(auto_pins.iter().cloned());
                    let mut matched = HashSet::new();
                    let preload_terms = if let Some(path) = agents_manifest()? {
                        if let Ok(text) = fs::read_to_string(&path) {
                            Some(extract_refs_from_agents(&text))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let preload_terms_ref = preload_terms.as_ref();
                    let mut diag = if args.diagnose.unwrap_or(env_diag()) {
                        Some(Diagnostics::default())
                    } else {
                        None
                    };
                    if let Some(d) = diag.as_mut() {
                        d.duplicates.extend(dup_log.iter().cloned());
                    }
                    let runtime = self.runtime_overrides();
                    let render_mode = manifest_render_mode(&runtime, context.peer.peer_info());
                    let content = self.render_autoload_cached(
                        &skills,
                        AutoloadOptions {
                            include_claude: args.include_claude.unwrap_or(env_include_claude()),
                            max_bytes: args.max_bytes.or(env_max_bytes()),
                            prompt: args
                                .prompt
                                .or_else(|| std::env::var("SKRILLS_PROMPT").ok())
                                .as_deref(),
                            embed_threshold: Some(
                                args.embed_threshold
                                    .unwrap_or_else(env_embed_threshold)
                            ),
                            preload_terms: preload_terms_ref,
                            pinned: Some(&effective_pins),
                            matched: Some(&mut matched),
                            diagnostics: diag.as_mut(),
                            render_mode,
                            log_render_mode: runtime.render_mode_log(),
                            gzip_ok: peer_accepts_gzip(context.peer.peer_info()),
                            minimal_manifest: runtime.manifest_minimal(),
                        },
                    )?;
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let mut history = history;
                    let mut matched_vec: Vec<String> = matched.into_iter().collect();
                    matched_vec.sort();
                    history.push(HistoryEntry {
                        ts,
                        skills: matched_vec.clone(),
                    });
                    let _ = save_history(history);
                    Ok(CallToolResult {
                        content: vec![Content::text(content.clone())],
                        structured_content: Some(json!({
                            "content": content,
                            "matched": matched_vec.clone(),
                            "truncated": diag.as_ref().map(|d| d.truncated).unwrap_or(false),
                            "skills": skills_with_rank,
                            "_meta": {
                                "duplicates": dup_log,
                                "priority": priority,
                                "priority_rank_by_source": rank_map
                            }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "render-preview" => {
                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let args: AutoloadArgs = request
                        .arguments
                        .as_ref()
                        .map(|obj| {
                            serde_json::from_value(json!(obj.clone())).map_err(anyhow::Error::from)
                        })
                        .transpose()?
                        .unwrap_or_default();
                    let manual_pins = load_pinned().unwrap_or_default();
                    let history = load_history().unwrap_or_default();
                    let auto_pins = if args.auto_pin.unwrap_or(env_auto_pin_default()) {
                        auto_pin_from_history(&history)
                    } else {
                        HashSet::new()
                    };
                    let mut effective_pins = manual_pins.clone();
                    effective_pins.extend(auto_pins.iter().cloned());
                    let preload_terms = if let Some(path) = agents_manifest()? {
                        if let Ok(text) = fs::read_to_string(&path) {
                            Some(extract_refs_from_agents(&text))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let preload_terms_ref = preload_terms.as_ref();
                    let runtime = self.runtime_overrides();
                    let stats = render_preview_stats(
                        &skills,
                        AutoloadOptions {
                            include_claude: args.include_claude.unwrap_or(env_include_claude()),
                            max_bytes: args.max_bytes.or(env_max_bytes()),
                            prompt: args
                                .prompt
                                .or_else(|| std::env::var("SKRILLS_PROMPT").ok())
                                .as_deref(),
                            embed_threshold: Some(
                                args.embed_threshold
                                    .unwrap_or_else(env_embed_threshold)
                            ),
                            preload_terms: preload_terms_ref,
                            pinned: Some(&effective_pins),
                            matched: None,
                            diagnostics: None,
                            render_mode: RenderMode::ManifestOnly,
                            log_render_mode: runtime.render_mode_log(),
                            gzip_ok: false,
                            minimal_manifest: runtime.manifest_minimal(),
                        },
                    )?;
                    let text = format!(
                        "preview matched {} skills, ~{} tokens (~{} bytes)",
                        stats.matched.len(),
                        stats.estimated_tokens,
                        stats.manifest_bytes
                    );
                    Ok(CallToolResult {
                        content: vec![Content::text(text)],
                        structured_content: Some(json!({
                            "matched": stats.matched,
                            "manifest_bytes": stats.manifest_bytes,
                            "estimated_tokens": stats.estimated_tokens,
                            "truncated": stats.truncated,
                            "truncated_content": stats.truncated_content,
                            "_meta": { "duplicates": dup_log }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "runtime-status" => {
                    let runtime = self.runtime_overrides();
            let status = json!({
                "manifest_first": runtime.manifest_first(),
                "render_mode_log": runtime.render_mode_log(),
                "manifest_minimal": runtime.manifest_minimal(),
                "overrides": {
                    "manifest_first": runtime.manifest_first,
                    "render_mode_log": runtime.render_mode_log,
                    "manifest_minimal": runtime.manifest_minimal,
                },
                "env": {
                    "manifest_first": env_manifest_first(),
                    "render_mode_log": env_render_mode_log(),
                    "manifest_minimal": env_manifest_minimal(),
                }
            });
                    Ok(CallToolResult {
                        content: vec![Content::text("runtime status")],
                        structured_content: Some(status),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "set-runtime-options" => {
                    let mut runtime = self.runtime_overrides();
                    if let Some(args) = request.arguments.as_ref() {
                        if let Some(val) = args.get("manifest_first").and_then(|v| v.as_bool()) {
                            runtime.manifest_first = Some(val);
                        }
                        if let Some(val) = args.get("render_mode_log").and_then(|v| v.as_bool()) {
                            runtime.render_mode_log = Some(val);
                        }
                        if let Some(val) = args.get("manifest_minimal").and_then(|v| v.as_bool()) {
                            runtime.manifest_minimal = Some(val);
                        }
                        if let Err(e) = runtime.save() {
                            tracing::warn!(error = %e, "failed to save runtime overrides");
                        }
                        if let Ok(mut guard) = self.runtime.lock() {
                            *guard = runtime.clone();
                        }
                    }
                    let status = json!({
                        "manifest_first": runtime.manifest_first(),
                        "render_mode_log": runtime.render_mode_log(),
                        "manifest_minimal": runtime.manifest_minimal(),
                        "overrides": {
                            "manifest_first": runtime.manifest_first,
                            "render_mode_log": runtime.render_mode_log,
                            "manifest_minimal": runtime.manifest_minimal.unwrap_or(runtime.manifest_minimal()),
                        }
                    });
                    Ok(CallToolResult {
                        content: vec![Content::text("runtime options updated")],
                        structured_content: Some(status),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "sync-from-claude" => {
                    let home = home_dir()?;
                    let claude_root = home.join(".claude");
                    let mirror_root = home.join(".codex/skills-mirror");
                    let report = sync_from_claude(&claude_root, &mirror_root)?;
                    let text = format!("copied: {}, skipped: {}", report.copied, report.skipped);
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    Ok(CallToolResult {
                        content: vec![Content::text(text)],
                        structured_content: Some(json!({
                            "report": { "copied": report.copied, "skipped": report.skipped },
                            "_meta": {
                                "priority": priority,
                                "priority_rank_by_source": rank_map
                            }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "refresh-cache" => {
                    self.invalidate_cache()?;
                    Ok(CallToolResult {
                        content: vec![Content::text("cache invalidated")],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    })
                }
                other => Err(anyhow!("unknown tool {other}")),
            }
        }()
        .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
        std::future::ready(result)
    }

    /// Returns initialization information for the RMCP server.
    ///
    /// This includes server capabilities and a brief instruction message,
    /// indicating that this service acts as a bridge for `SKILL.md` files.
    fn get_info(&self) -> InitializeResult {
        // Start background warm-up only after the handshake path is hit to
        // keep the initialize response fast.
        self.spawn_warmup_if_needed();
        InitializeResult {
            capabilities: ServerCapabilities {
                resources: Some(Default::default()),
                tools: Some(Default::default()),
                ..Default::default()
            },
            instructions: Some("Codex SKILL.md bridge".into()),
            ..Default::default()
        }
    }
}

/// Arguments for controlling the `autoload-snippet` tool's behavior.
///
/// These fields determine how skills are filtered, truncated, and whether
/// diagnostic information or automatic pinning is applied during autoloading.
#[derive(Default, Deserialize)]
struct AutoloadArgs {
    /// If true, includes skills from the `~/.claude` directory.
    include_claude: Option<bool>,
    /// The maximum number of bytes for the autoloaded content.
    max_bytes: Option<usize>,
    /// A prompt string used to filter relevant skills.
    prompt: Option<String>,
    /// Embedding similarity threshold (0-1) for fuzzy prompt matching.
    embed_threshold: Option<f32>,
    /// If true, enables heuristic auto-pinning based on recent prompt matches.
    auto_pin: Option<bool>,
    /// If true, diagnostic information (included/skipped skills) is emitted.
    diagnose: Option<bool>,
}

/// Emits a JSON payload to stdout, typically for shell hook installations.
///
/// This payload includes a `hookSpecificOutput` field with `additionalContext`
/// containing the rendered autoload snippet. This is used to dynamically
/// provide relevant skill content for prompt injection or similar mechanisms.
fn emit_autoload(
    include_claude: bool,
    max_bytes: Option<usize>,
    prompt: Option<String>,
    embed_threshold: Option<f32>,
    auto_pin: bool,
    extra_dirs: &[PathBuf],
    diagnose: bool,
) -> Result<()> {
    let mut diag_opt = if diagnose {
        Some(Diagnostics::default())
    } else {
        None
    };
    let skills = if let Some(d) = &mut diag_opt {
        discover_skills(&skill_roots(extra_dirs)?, Some(&mut d.duplicates))?
    } else {
        collect_skills(extra_dirs)?
    };
    let manual_pins = load_pinned().unwrap_or_default();
    let history = load_history().unwrap_or_default();
    let auto_pins = if auto_pin {
        auto_pin_from_history(&history)
    } else {
        HashSet::new()
    };
    let mut effective_pins = manual_pins.clone();
    effective_pins.extend(auto_pins.iter().cloned());
    let mut matched = HashSet::new();
    let mut diag = diag_opt;
    let preload_terms = if let Some(path) = agents_manifest()? {
        if let Ok(text) = fs::read_to_string(&path) {
            Some(extract_refs_from_agents(&text))
        } else {
            None
        }
    } else {
        None
    };
    let preload_terms_ref = preload_terms.as_ref();
    let prompt = prompt.or_else(|| std::env::var("SKRILLS_PROMPT").ok());
    let runtime = runtime_overrides_cached();
    let render_mode = manifest_render_mode(&runtime, None);
    let content = render_autoload(
        &skills,
        AutoloadOptions {
            include_claude,
            max_bytes: max_bytes.or(env_max_bytes()),
            prompt: prompt.as_deref(),
            embed_threshold: Some(embed_threshold.unwrap_or_else(env_embed_threshold)),
            preload_terms: preload_terms_ref,
            pinned: Some(&effective_pins),
            matched: Some(&mut matched),
            diagnostics: diag.as_mut(),
            render_mode,
            log_render_mode: runtime.render_mode_log(),
            gzip_ok: false,
            minimal_manifest: runtime.manifest_minimal(),
        },
    )?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut history = history;
    let mut matched_vec: Vec<String> = matched.into_iter().collect();
    matched_vec.sort();
    history.push(HistoryEntry {
        ts,
        skills: matched_vec,
    });
    let _ = save_history(history);
    let payload = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": content
        }
    });
    println!("{}", serde_json::to_string(&payload)?);
    Ok(())
}

/// Prints a JSON list of discovered skills (debug helper).
fn list_skills(extra_dirs: &[PathBuf]) -> Result<()> {
    let skills = collect_skills(extra_dirs)?;
    println!("{}", serde_json::to_string_pretty(&skills)?);
    Ok(())
}

#[cfg(unix)]
/// Installs a `SIGCHLD` handler to reap child processes, preventing zombies.
///
/// # Safety
/// - This alters process-wide signal disposition (sets `SIGCHLD` to `SIG_IGN` with `SA_NOCLDWAIT`);
///   only call during single-threaded startup before any other code configures `SIGCHLD` or relies
///   on default child-exit behavior.
/// - All threads share the handler state; callers must guarantee no other thread concurrently
///   mutates the `SIGCHLD` handler.
/// - This assumes POSIX semantics for `libc::sigaction`; on non-Unix targets the function is
///   stubbed out. Do not invoke from platforms where `SIGCHLD`/`SA_NOCLDWAIT` are unavailable.
/// - Because the handler discards child exit status, downstream code that expects to `wait`
///   on children must not be used alongside this helper.
fn ignore_sigchld() -> Result<()> {
    unsafe {
        let mut sa: libc::sigaction = mem::zeroed();
        sa.sa_flags = libc::SA_NOCLDWAIT | libc::SA_RESTART;
        sa.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut sa.sa_mask);
        let rc = libc::sigaction(libc::SIGCHLD, &sa, ptr::null_mut());
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
/// Stub for `ignore_sigchld` on non-Unix platforms.
fn ignore_sigchld() -> Result<()> {
    Ok(())
}

pub fn run() -> Result<()> {
    ignore_sigchld()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Serve {
        skill_dirs: Vec::new(),
        cache_ttl_ms: None,
        trace_wire: false,
        #[cfg(feature = "watch")]
        watch: false,
    }) {
        Commands::Serve {
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
        } => {
            let ttl = cache_ttl_ms
                .map(Duration::from_millis)
                .unwrap_or_else(|| cache_ttl(&load_manifest_settings));
            let service = SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs), ttl)?;
            #[cfg(feature = "watch")]
            let _watcher = if watch {
                Some(start_fs_watcher(&service)?)
            } else {
                None
            };

            let transport = stdio_with_optional_trace(trace_wire);
            let rt = Runtime::new()?;
            let running = rt.block_on(async {
                serve_server(service, transport)
                    .await
                    .map_err(|e| anyhow!("failed to start server: {e}"))
            })?;
            rt.block_on(async {
                running
                    .waiting()
                    .await
                    .map_err(|e| anyhow!("server task ended: {e}"))
            })?;
            #[cfg(feature = "watch")]
            drop(_watcher);
            Ok(())
        }
        Commands::List => list_skills(&merge_extra_dirs(&[])),
        Commands::ListPinned => {
            let pinned = load_pinned()?;
            if pinned.is_empty() {
                println!("(no pinned skills)");
            } else {
                let mut list: Vec<_> = pinned.into_iter().collect();
                list.sort();
                for name in list {
                    println!("{name}");
                }
            }
            Ok(())
        }
        Commands::Pin { skills } => {
            let mut pinned = load_pinned()?;
            let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
            for spec in skills {
                let name = resolve_skill(&spec, &all_skills)?;
                pinned.insert(name.to_string());
            }
            save_pinned(&pinned)?;
            println!("Pinned {} skills.", pinned.len());
            Ok(())
        }
        Commands::Unpin { skills, all } => {
            if all {
                save_pinned(&HashSet::new())?;
                println!("Cleared all pinned skills.");
                return Ok(());
            }
            if skills.is_empty() {
                return Err(anyhow!("provide skill names or use --all"));
            }
            let mut pinned = load_pinned()?;
            let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
            for spec in skills {
                let name = resolve_skill(&spec, &all_skills)?;
                pinned.remove(name);
            }
            save_pinned(&pinned)?;
            println!("Pinned skills remaining: {}", pinned.len());
            Ok(())
        }
        Commands::AutoPin { enable } => {
            save_auto_pin_flag(enable)?;
            println!("Auto-pin {}", if enable { "enabled" } else { "disabled" });
            Ok(())
        }
        Commands::History { limit } => {
            print_history(limit)?;
            Ok(())
        }
        Commands::SyncAgents { path, skill_dirs } => {
            let path = path.unwrap_or_else(|| PathBuf::from("AGENTS.md"));
            sync_agents(&path, &merge_extra_dirs(&skill_dirs))?;
            println!("Updated {}", path.display());
            Ok(())
        }
        Commands::EmitAutoload {
            include_claude,
            max_bytes,
            prompt,
            embed_threshold,
            auto_pin,
            skill_dirs,
            diagnose,
        } => emit_autoload(
            include_claude,
            max_bytes,
            prompt,
            embed_threshold,
            auto_pin,
            &merge_extra_dirs(&skill_dirs),
            diagnose,
        ),
        Commands::Sync => {
            let home = home_dir()?;
            let report =
                sync_from_claude(&home.join(".claude"), &home.join(".codex/skills-mirror"))?;
            println!("copied: {}, skipped: {}", report.copied, report.skipped);
            Ok(())
        }
        Commands::Doctor => doctor_report(),
        Commands::Tui { skill_dirs } => tui_flow(&merge_extra_dirs(&skill_dirs)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::reset_runtime_cache_for_tests;
    use flate2::read::GzDecoder;
    use once_cell::sync::Lazy;
    use skrills_state::runtime_overrides_path;
    use std::collections::HashSet;
    use std::fs;
    use std::io::Read;
    use std::process::Command;
    use std::time::Duration;
    use tempfile::tempdir;

    // Tests in this module mutate HOME and on-disk runtime cache; serialize to avoid cross-test contamination.
    static TEST_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_SERIAL.lock().unwrap()
    }

    /// Lightweight Given/When/Then helpers to keep autoload tests readable.
    mod gwt_autoload {
        use super::*;

        pub struct SkillFixture {
            pub skills: Vec<SkillMeta>,
        }

        pub fn given_skill(
            root: &Path,
            name: &str,
            content: &str,
            source: SkillSource,
        ) -> Result<SkillFixture> {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, content)?;
            let skills = vec![SkillMeta {
                name: name.into(),
                path: path.clone(),
                source,
                root: root.to_path_buf(),
                hash: hash_file(&path)?,
            }];
            Ok(SkillFixture { skills })
        }

        #[allow(clippy::too_many_arguments)]
        pub fn given_two_skills(
            one_root: &Path,
            one_name: &str,
            one_content: &str,
            one_source: SkillSource,
            two_root: &Path,
            two_name: &str,
            two_content: &str,
            two_source: SkillSource,
        ) -> Result<SkillFixture> {
            let mut first = given_skill(one_root, one_name, one_content, one_source)?;
            let second = given_skill(two_root, two_name, two_content, two_source)?;
            first.skills.extend(second.skills);
            Ok(first)
        }

        pub fn when_render_autoload(
            fixture: &SkillFixture,
            options: AutoloadOptions<'_, '_, '_, '_>,
        ) -> Result<String> {
            render_autoload(&fixture.skills, options)
        }

        pub fn with_embed_similarity(value: f32) -> EmbedOverrideGuard {
            EmbedOverrideGuard::set(value)
        }

        pub fn then_contains(content: &str, needle: &str) {
            assert!(
                content.contains(needle),
                "expected content to contain `{needle}`, but it did not"
            );
        }

        pub fn then_not_contains(content: &str, needle: &str) {
            assert!(
                !content.contains(needle),
                "expected content to not contain `{needle}`, but it did"
            );
        }
    }

    #[test]
    fn list_resources_includes_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("SKRILLS_MANIFEST");
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };
        let resources = svc.list_resources_payload()?;
        assert!(resources
            .iter()
            .any(|r| r.uri == AGENTS_URI && r.name == AGENTS_NAME));

        // Restore original HOME
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn read_resource_returns_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };
        let result = svc.read_resource_sync(AGENTS_URI)?;
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text,
            _ => anyhow::bail!("expected text content"),
        };
        assert!(text.contains("AI Agent Development Guidelines"));

        // Restore original HOME
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn render_available_skills_xml_contains_location() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("codex/skills");
        fs::create_dir_all(&path).unwrap();
        let skill_path = path.join("alpha/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "hello").unwrap();
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: path.clone(),
            hash: hash_file(&skill_path).unwrap(),
        }];
        let xml = render_available_skills_xml(&skills);
        assert!(xml.contains("location=\"global\""));
        assert!(xml.contains("alpha/SKILL.md"));
    }

    #[test]
    fn sync_agents_inserts_section() -> Result<()> {
        let tmp = tempdir()?;
        let agents = tmp.path().join("AGENTS.md");
        fs::write(&agents, "# Title")?;
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: tmp.path().join("alpha/SKILL.md"),
            source: SkillSource::Codex,
            root: tmp.path().join("codex/skills"),
            hash: "abc".into(),
        }];
        sync_agents_with_skills(&agents, &skills)?;
        let text = fs::read_to_string(&agents)?;
        assert!(text.contains(AGENTS_SECTION_START));
        assert!(text.contains("available_skills"));
        assert!(text.contains("location=\"global\""));
        assert!(text.contains(AGENTS_SECTION_END));
        assert!(text.contains("# Title"));
        Ok(())
    }

    #[test]
    fn read_resource_includes_priority_rank() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "hello")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };
        let result = svc.read_resource_sync("skill://codex/alpha/SKILL.md")?;
        match &result.contents[0] {
            ResourceContents::TextResourceContents { meta: Some(m), .. } => {
                let rank = m.get("priority_rank").and_then(|v| v.as_u64()).unwrap();
                assert_eq!(rank, 1);
                assert_eq!(
                    m.get("location").and_then(|v| v.as_str()).unwrap(),
                    "global"
                );
            }
            _ => anyhow::bail!("expected text content with meta"),
        };
        Ok(())
    }

    #[test]
    fn sync_agents_sets_priority_rank_in_xml() -> Result<()> {
        let tmp = tempdir()?;
        let _agents = tmp.path().join("AGENTS.md");
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: tmp.path().join("alpha/SKILL.md"),
            source: SkillSource::Codex,
            root: tmp.path().join("codex/skills"),
            hash: "abc".into(),
        }];
        let xml = render_available_skills_xml(&skills);
        assert!(xml.contains("priority_rank=\"1\""));
        Ok(())
    }

    #[test]
    fn priority_rank_map_matches_labels() {
        let (labels, map) = priority_labels_and_rank_map();
        assert_eq!(labels, vec!["codex", "mirror", "claude", "agent"]);
        assert_eq!(map.get("codex").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(map.get("agent").and_then(|v| v.as_u64()), Some(4));
    }

    #[test]
    fn duplicates_are_logged_and_can_be_reported() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        let claude_root = tmp.path().join("claude/skills");
        fs::create_dir_all(&codex_root)?;
        fs::create_dir_all(&claude_root)?;
        let path1 = codex_root.join("dup/SKILL.md");
        let path2 = claude_root.join("dup/SKILL.md");
        fs::create_dir_all(path1.parent().unwrap())?;
        fs::create_dir_all(path2.parent().unwrap())?;
        fs::write(&path1, "one")?;
        fs::write(&path2, "two")?;

        let roots = vec![
            SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: claude_root.clone(),
                source: SkillSource::Claude,
            },
        ];
        let mut dup_log = Vec::new();
        let skills = discover_skills(&roots, Some(&mut dup_log))?;
        assert_eq!(skills.len(), 1);
        assert_eq!(dup_log.len(), 1);
        let dup = &dup_log[0];
        assert_eq!(dup.name, "dup/SKILL.md");
        assert_eq!(dup.kept_source, "codex");
        assert_eq!(dup.skipped_source, "claude");
        Ok(())
    }

    #[test]
    fn skill_cache_refreshes_after_ttl() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("one"))?;
        let skill_one = codex_root.join("one/SKILL.md");
        fs::write(&skill_one, "one")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(
                vec![SkillRoot {
                    root: codex_root.clone(),
                    source: SkillSource::Codex,
                }],
                Duration::from_millis(5),
            ))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };

        let (skills_first, _) = svc.current_skills_with_dups()?;
        assert_eq!(skills_first.len(), 1);

        std::thread::sleep(Duration::from_millis(10));
        let skill_two = codex_root.join("two/SKILL.md");
        fs::create_dir_all(skill_two.parent().unwrap())?;
        fs::write(&skill_two, "two")?;

        let (skills_second, _) = svc.current_skills_with_dups()?;
        assert_eq!(skills_second.len(), 2);
        Ok(())
    }

    #[test]
    fn runtime_overrides_cached_memoizes() -> Result<()> {
        let _guard = TEST_SERIAL.lock().unwrap();
        reset_runtime_cache_for_tests();
        let tmp = tempdir()?;
        std::env::set_var("HOME", tmp.path());
        let path = runtime_overrides_path().unwrap();
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, r#"{"manifest_first":false,"render_mode_log":true}"#)?;

        let first = runtime_overrides_cached();
        assert!(!first.manifest_first());
        assert!(first.render_mode_log());

        // Modify file; cached value should remain until reset
        fs::write(&path, r#"{"manifest_first":true,"render_mode_log":false}"#)?;
        let still_cached = runtime_overrides_cached();
        assert!(!still_cached.manifest_first());
        assert!(still_cached.render_mode_log());

        reset_runtime_cache_for_tests();
        let refreshed = runtime_overrides_cached();
        assert!(refreshed.manifest_first());
        assert!(!refreshed.render_mode_log());
        Ok(())
    }

    #[test]
    fn manifest_render_mode_respects_runtime_and_allowlist() {
        reset_runtime_cache_for_tests();
        reset_allowlist_cache_for_tests();
        let mut rt = RuntimeOverrides {
            manifest_first: Some(false),
            ..Default::default()
        };
        assert_eq!(manifest_render_mode(&rt, None), RenderMode::ContentOnly);

        rt.manifest_first = Some(true);
        let mut client = ClientInfo::default();
        client.client_info.name = "claude-desktop".into();
        assert_eq!(
            manifest_render_mode(&rt, Some(&client)),
            RenderMode::ManifestOnly
        );

        let tmp = tempdir().unwrap();
        let allow = tmp.path().join("allow.json");
        fs::write(&allow, r#"[{"name_substr":"codex","min_version":"2.0.0"}]"#).unwrap();
        std::env::set_var("SKRILLS_MANIFEST_ALLOWLIST", &allow);
        reset_allowlist_cache_for_tests();
        let mut codex = ClientInfo::default();
        codex.client_info.name = "codex-cli".into();
        codex.client_info.version = "2.1.0".into();
        assert_eq!(
            manifest_render_mode(&rt, Some(&codex)),
            RenderMode::ManifestOnly
        );

        codex.client_info.version = "1.0.0".into();
        reset_allowlist_cache_for_tests();
        assert_eq!(manifest_render_mode(&rt, Some(&codex)), RenderMode::Dual);
    }

    #[test]
    fn content_cache_updates_when_hash_changes() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "v1")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(
                vec![SkillRoot {
                    root: codex_root.clone(),
                    source: SkillSource::Codex,
                }],
                Duration::from_millis(1),
            ))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };

        let uri = "skill://codex/alpha/SKILL.md";
        let first = svc.read_resource_sync(uri)?;
        let first_text = match &first.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => anyhow::bail!("expected text content"),
        };
        assert!(first_text.contains("v1"));

        fs::write(&skill_path, "v2")?;
        std::thread::sleep(Duration::from_millis(5));

        let second = svc.read_resource_sync(uri)?;
        let second_text = match &second.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => anyhow::bail!("expected text content"),
        };
        assert!(second_text.contains("v2"));
        Ok(())
    }

    #[test]
    fn manifest_can_disable_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(
            &manifest,
            r#"{ "priority": ["codex","claude"], "expose_agents": false }"#,
        )?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
        };
        assert!(!svc.expose_agents_doc()?);
        let resources = svc.list_resources_payload()?;
        assert!(!resources.iter().any(|r| r.uri == AGENTS_URI));
        let err = svc.read_resource_sync(AGENTS_URI).unwrap_err();
        assert!(err.to_string().contains("not found"));
        std::env::remove_var("SKRILLS_MANIFEST");

        // Restore original HOME
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn manifest_can_set_cache_ttl() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(&manifest, r#"{ "cache_ttl_ms": 2500 }"#)?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let svc = SkillService::new(vec![])?;
        let ttl = svc
            .cache
            .lock()
            .map_err(|e| anyhow!("poisoned: {e}"))?
            .ttl();
        assert_eq!(ttl, Duration::from_millis(2500));
        std::env::remove_var("SKRILLS_MANIFEST");

        // Restore original HOME
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn collect_skills_uses_relative_paths_and_hashes() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "hello")?;

        let roots = vec![SkillRoot {
            root: codex_root.clone(),
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None)?;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "alpha/SKILL.md");
        assert_eq!(skills[0].hash, hash_file(&skill_path)?);
        assert!(matches!(skills[0].source, SkillSource::Codex));
        Ok(())
    }

    #[test]
    fn sync_from_claude_copies_and_updates() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");
        fs::create_dir_all(claude_root.join("nested"))?;
        let skill_src = claude_root.join("nested/SKILL.md");
        fs::write(&skill_src, "v1")?;

        let report1 = sync_from_claude(&claude_root, &mirror_root)?;
        assert_eq!(report1.copied, 1);
        let dest = mirror_root.join("nested/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "v1");

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&skill_src, "v2")?;
        let report2 = sync_from_claude(&claude_root, &mirror_root)?;
        assert_eq!(report2.copied, 1);
        assert_eq!(fs::read_to_string(&dest)?, "v2");
        Ok(())
    }

    #[test]
    fn given_manifest_limit_when_render_autoload_then_manifest_valid_json() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN codex + claude skills with limited byte budget
        let tmp = tempdir()?;
        let fixture = given_two_skills(
            &tmp.path().join("codex/skills"),
            "codex/SKILL.md",
            &"C token_repeat".repeat(20),
            SkillSource::Codex,
            &tmp.path().join("claude"),
            "claude/SKILL.md",
            &"irrelevant content".repeat(5),
            SkillSource::Claude,
        )?;

        let manifest_only = when_render_autoload(
            &fixture,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let full_dual = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("token efficiency test prompt"),
                ..Default::default()
            },
        )?;
        let limit = manifest_only.len() + 16;
        assert!(limit < full_dual.len());

        // WHEN rendering with a tight limit
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                max_bytes: Some(limit),
                prompt: Some("token efficiency test prompt"),
                ..Default::default()
            },
        )?;

        // THEN output stays under limit and manifest JSON is still parseable
        assert!(content.len() <= limit);
        let json_part = content
            .lines()
            .skip_while(|l| l.starts_with("[skills]"))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str::<serde_json::Value>(&json_part)?;
        Ok(())
    }

    #[test]
    fn autoload_includes_pinned_even_when_filtered_out() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        let mirror_dir = tmp.path().join("mirror");
        fs::create_dir_all(&codex_dir)?;
        fs::create_dir_all(&mirror_dir)?;

        let codex_skill = codex_dir.join("codex/SKILL.md");
        let mirror_skill = mirror_dir.join("mirror/SKILL.md");
        fs::create_dir_all(codex_skill.parent().unwrap())?;
        fs::create_dir_all(mirror_skill.parent().unwrap())?;
        fs::write(&codex_skill, "codex content")?;
        fs::write(&mirror_skill, "mirror content with no prompt hits")?;

        let skills = vec![
            SkillMeta {
                name: "codex/SKILL.md".into(),
                path: codex_skill.clone(),
                source: SkillSource::Codex,
                root: codex_dir.clone(),
                hash: hash_file(&codex_skill)?,
            },
            SkillMeta {
                name: "mirror/SKILL.md".into(),
                path: mirror_skill.clone(),
                source: SkillSource::Mirror,
                root: mirror_dir.clone(),
                hash: hash_file(&mirror_skill)?,
            },
        ];

        let mut pinned = HashSet::new();
        pinned.insert("mirror/SKILL.md".to_string());

        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: true,
                prompt: Some("tokenless prompt"),
                pinned: Some(&pinned),
                ..Default::default()
            },
        )?;
        assert!(content.contains("mirror/SKILL.md"));
        // Codex skill is not pinned and the prompt does not match, so it may be filtered out.
        Ok(())
    }

    #[test]
    fn given_fuzzy_prompt_when_similarity_above_threshold_then_skill_included() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN a skill whose name matches the prompt fuzzily
        let tmp = tempdir()?;
        let fixture = given_skill(
            &tmp.path().join("codex/skills"),
            "analysis/SKILL.md",
            "Guide to analyse pipeline performance and resilience.",
            SkillSource::Codex,
        )?;

        // WHEN rendering with a permissive embed threshold
        let _override = with_embed_similarity(0.8);
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                embed_threshold: Some(0.10),
                ..Default::default()
            },
        )?;

        // THEN the fuzzy-matched skill is included
        then_contains(&content, "analysis/SKILL.md");
        Ok(())
    }

    #[test]
    fn given_fuzzy_prompt_when_threshold_strict_then_skill_excluded() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN a skill and a fuzzy prompt
        let tmp = tempdir()?;
        let fixture = given_skill(
            &tmp.path().join("codex/skills"),
            "analysis/SKILL.md",
            "Guide to analyse pipeline performance and resilience.",
            SkillSource::Codex,
        )?;

        // WHEN the embed threshold is strict
        let _override = with_embed_similarity(0.2);
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                embed_threshold: Some(0.95),
                ..Default::default()
            },
        )?;

        // THEN the fuzzy match is rejected
        then_not_contains(&content, "analysis/SKILL.md");
        Ok(())
    }

    #[test]
    fn autoload_respects_env_embed_threshold_default() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("analysis/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(
            &skill_path,
            "Guide to analyse pipeline performance and resilience.",
        )?;

        let skills = vec![SkillMeta {
            name: "analysis/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        std::env::set_var("SKRILLS_EMBED_THRESHOLD", "0.9");
        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                ..Default::default()
            },
        )?;
        std::env::remove_var("SKRILLS_EMBED_THRESHOLD");

        assert!(
            !content.contains("analysis/SKILL.md"),
            "env-set high threshold should apply when option not provided"
        );
        Ok(())
    }

    #[test]
    fn autoload_keyword_match_still_wins_when_threshold_high() -> Result<()> {
        // GIVEN a prompt that directly names the skill (keyword path)
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("observability/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(&skill_path, "How to add tracing and metrics.")?;

        let skills = vec![SkillMeta {
            name: "observability/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        // WHEN embed threshold is high but keyword hits
        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("need observability best practices"),
                embed_threshold: Some(0.99),
                ..Default::default()
            },
        )?;

        // THEN the skill is still included
        assert!(
            content.contains("observability/SKILL.md"),
            "direct keyword match should not be blocked by high embed threshold"
        );
        Ok(())
    }

    #[test]
    fn autoload_args_parses_embed_threshold() -> Result<()> {
        let json = r#"{
            "include_claude": false,
            "max_bytes": 1024,
            "prompt": "typo prompt",
            "embed_threshold": 0.42,
            "auto_pin": false,
            "diagnose": true
        }"#;
        let args: AutoloadArgs = serde_json::from_str(json)?;
        assert_eq!(args.embed_threshold, Some(0.42));
        assert_eq!(args.prompt, Some("typo prompt".into()));
        assert_eq!(args.max_bytes, Some(1024));
        Ok(())
    }

    #[test]
    fn render_preview_stats_returns_token_estimate() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("analysis/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(
            &skill_path,
            "Guide to analyse pipeline performance and resilience.",
        )?;

        let skills = vec![SkillMeta {
            name: "analysis/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        let stats = render_preview_stats(
            &skills,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("pipeline review"),
                embed_threshold: Some(0.05),
                ..Default::default()
            },
        )?;

        assert_eq!(stats.matched, vec!["analysis/SKILL.md"]);
        assert!(stats.manifest_bytes > 0);
        assert!(stats.estimated_tokens > 0);
        Ok(())
    }

    #[test]
    fn manifest_only_small_limit_stays_valid_json() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let codex_skill = codex_dir.join("SKILL.md");
        fs::write(&codex_skill, "C token_repeat".repeat(20))?;

        let skills = vec![SkillMeta {
            name: "codex/SKILL.md".into(),
            path: codex_skill.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&codex_skill)?,
        }];

        let manifest_only = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let limit = manifest_only.len() + 8;

        let content = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                max_bytes: Some(limit),
                ..Default::default()
            },
        )?;

        // Should parse as JSON and not exceed limit.
        assert!(content.len() <= limit);
        let json_part = content
            .lines()
            .skip_while(|l| l.starts_with("[skills]"))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str::<serde_json::Value>(&json_part)?;
        Ok(())
    }

    #[test]
    fn gzipped_manifest_fallback_when_limit_hit_and_supported() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let codex_skill = codex_dir.join("SKILL.md");
        fs::write(&codex_skill, "C token_repeat".repeat(50))?;

        let skills = vec![SkillMeta {
            name: "codex/SKILL.md".into(),
            path: codex_skill.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&codex_skill)?,
        }];

        let manifest_only_full = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let manifest_json_full = manifest_only_full
            .lines()
            .skip_while(|l| l.starts_with("[skills]"))
            .collect::<Vec<_>>()
            .join("\n");
        let gz_wrapped_full = format!(
            r#"{{"skills_manifest_gzip_base64":"{}"}}"#,
            gzip_base64(&manifest_json_full)?
        );
        let limit = gz_wrapped_full.len(); // smaller than raw manifest to force gzip path
        let preview_len = limit.saturating_div(4).clamp(64, 512);
        let preview = read_prefix(&codex_skill, preview_len)?;
        let expected_manifest = json!({
            "skills_manifest": [{
                "name": "codex/SKILL.md",
                "source": SkillSource::Codex,
                "root": codex_dir,
                "path": codex_skill,
                "hash": hash_file(&codex_skill)?,
                "preview": preview
            }]
        });

        let content = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                max_bytes: Some(limit),
                gzip_ok: true,
                ..Default::default()
            },
        )?;

        let v_compressed: serde_json::Value = serde_json::from_str(&content)?;
        let b64 = v_compressed
            .get("skills_manifest_gzip_base64")
            .and_then(|s| s.as_str())
            .expect("compressed manifest present");
        let bytes = BASE64.decode(b64)?;
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded)?;
        let manifest_val: serde_json::Value = serde_json::from_str(&decoded)?;
        assert_eq!(manifest_val, expected_manifest);
        Ok(())
    }

    #[test]
    fn minimal_manifest_drops_heavy_fields() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let codex_skill = codex_dir.join("SKILL.md");
        fs::write(&codex_skill, "short content")?;

        let skills = vec![SkillMeta {
            name: "codex/SKILL.md".into(),
            path: codex_skill.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&codex_skill)?,
        }];

        let full = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let minimal = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                minimal_manifest: true,
                ..Default::default()
            },
        )?;

        assert!(minimal.len() < full.len());
        let parse_manifest = |s: &str| -> Result<serde_json::Value> {
            let body = s
                .lines()
                .skip_while(|l| l.starts_with("[skills]"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(serde_json::from_str(&body)?)
        };

        let full_json = parse_manifest(&full)?;
        let minimal_json = parse_manifest(&minimal)?;
        let minimal_skill = minimal_json["skills_manifest"][0].as_object().unwrap();
        assert!(!minimal_skill.contains_key("path"));
        assert!(!minimal_skill.contains_key("preview"));
        assert!(minimal_skill.contains_key("name"));
        assert!(minimal_skill.contains_key("hash"));

        let full_skill = full_json["skills_manifest"][0].as_object().unwrap();
        assert!(full_skill.contains_key("path"));
        assert!(full_skill.contains_key("preview"));
        Ok(())
    }

    #[test]
    fn peer_accepts_gzip_prefers_env_then_name() {
        assert!(!peer_accepts_gzip(None));

        std::env::set_var("SKRILLS_ACCEPT_GZIP", "1");
        assert!(peer_accepts_gzip(None));
        std::env::remove_var("SKRILLS_ACCEPT_GZIP");

        let mut client = ClientInfo::default();
        client.client_info.name = "gzip-capable-client".into();
        assert!(peer_accepts_gzip(Some(&client)));
    }

    #[test]
    fn auto_pin_from_recent_history() {
        let history = vec![
            HistoryEntry {
                ts: 1,
                skills: vec!["a".into(), "b".into()],
            },
            HistoryEntry {
                ts: 2,
                skills: vec!["a".into()],
            },
            HistoryEntry {
                ts: 3,
                skills: vec!["c".into()],
            },
        ];
        let pins = auto_pin_from_history(&history);
        assert!(pins.contains("a")); // appears twice in window
        assert!(!pins.contains("b")); // only once
        assert!(!pins.contains("c")); // only once
    }

    #[test]
    fn manifest_priority_overrides_default() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(&manifest, r#"["agent","codex"]"#)?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let roots = skill_roots(&[])?;
        let order: Vec<_> = roots.into_iter().map(|r| r.source.label()).collect();
        assert_eq!(order, vec!["agent", "codex", "mirror", "claude"]);
        std::env::remove_var("SKRILLS_MANIFEST");

        // Restore original HOME
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn ignores_sigchld_to_avoid_zombies() -> Result<()> {
        ignore_sigchld()?;

        let child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawn child");
        let pid = child.id() as libc::pid_t;

        drop(child);
        std::thread::sleep(Duration::from_millis(50));

        let res = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(res, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn collect_skills_errors_on_unreadable_skill() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex");
        fs::create_dir_all(&codex_root)?;
        let skill_path = codex_root.join("SKILL.md");
        fs::write(&skill_path, "secret")?;
        let mut perms = fs::metadata(&skill_path)?.permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&skill_path, perms)?;

        let roots = vec![SkillRoot {
            root: codex_root,
            source: SkillSource::Codex,
        }];

        let result = discover_skills(&roots, None);
        assert!(result.is_err());
        Ok(())
    }
}
