//! Autoload rendering logic for injecting skills into Claude prompts.
//!
//! It:
//! - Filtering skills based on prompts and embeddings.
//! - Building manifest entries.
//! - Assembling output with optional compression.
//! - Applying size limits and truncation.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use skrills_discovery::{Diagnostics, SkillMeta, SkillSource};
use std::collections::HashSet;
use std::io::Write;

use crate::discovery::{
    read_prefix, read_skill, trigram_similarity_checked, DEFAULT_EMBED_PREVIEW_BYTES,
};

/// Defines how autoloaded content is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum RenderMode {
    /// Emit manifest plus full content (backward compatible).
    #[default]
    Dual,
    /// Emit only manifest (for manifest-capable clients).
    ManifestOnly,
    /// Emit only concatenated content (legacy).
    ContentOnly,
}

/// Options for autoload rendering.
#[derive(Default)]
pub(crate) struct AutoloadOptions<'p, 't, 'm, 'd> {
    pub(crate) include_claude: bool,
    pub(crate) max_bytes: Option<usize>,
    pub(crate) prompt: Option<&'p str>,
    pub(crate) embed_threshold: Option<f32>,
    pub(crate) preload_terms: Option<&'t HashSet<String>>,
    pub(crate) pinned: Option<&'t HashSet<String>>,
    pub(crate) matched: Option<&'m mut HashSet<String>>,
    pub(crate) diagnostics: Option<&'d mut Diagnostics>,
    pub(crate) render_mode: RenderMode,
    pub(crate) log_render_mode: bool,
    pub(crate) gzip_ok: bool,
    pub(crate) minimal_manifest: bool,
}

/// Statistics for preview rendering.
#[derive(Default)]
pub(crate) struct PreviewStats {
    pub(crate) matched: Vec<String>,
    pub(crate) manifest_bytes: usize,
    pub(crate) estimated_tokens: usize,
    pub(crate) truncated: bool,
    pub(crate) truncated_content: bool,
}

/// Gets the environment-defined embedding threshold.
pub(crate) fn env_embed_threshold() -> f32 {
    std::env::var("SKRILLS_EMBED_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.3)
}

/// Determines if a skill is relevant based on prompt, pins, and embedding similarity.
fn is_skill_relevant<G>(
    meta: &SkillMeta,
    term_opt: Option<&HashSet<String>>,
    is_pinned: bool,
    prompt_for_embedding: &str,
    embed_threshold: f32,
    read_prefix: &G,
) -> bool
where
    G: Fn(&SkillMeta, usize) -> Result<String>,
{
    match term_opt {
        None => true,
        Some(_) if is_pinned => true,
        Some(t) => {
            let name = meta.name.to_ascii_lowercase();
            if t.iter().any(|k| name.contains(k)) {
                return true;
            }

            if let Ok(prefix) = read_prefix(meta, DEFAULT_EMBED_PREVIEW_BYTES) {
                let text = prefix.to_ascii_lowercase();
                if t.iter().any(|k| text.contains(k)) {
                    return true;
                }
                let sim = trigram_similarity_checked(prompt_for_embedding, &text);
                sim >= embed_threshold
            } else {
                false
            }
        }
    }
}

/// Builds a manifest entry for a skill.
fn build_manifest_entry<G>(
    meta: &SkillMeta,
    minimal_manifest: bool,
    preview_len: usize,
    read_prefix: &G,
) -> serde_json::Value
where
    G: Fn(&SkillMeta, usize) -> Result<String>,
{
    if minimal_manifest {
        json!({
            "name": meta.name,
            "source": meta.source,
            "hash": meta.hash,
        })
    } else {
        let preview = read_prefix(meta, preview_len).unwrap_or_else(|_| String::new());
        json!({
            "name": meta.name,
            "source": meta.source,
            "root": meta.root,
            "path": meta.path,
            "hash": meta.hash,
            "preview": preview
        })
    }
}

/// Assembles the final output string from manifest, names, and content.
fn assemble_output(
    manifest_json: &str,
    names: &[String],
    content_buf: &str,
    include_manifest: bool,
    include_content: bool,
) -> String {
    let mut output = String::new();
    if include_manifest && !manifest_json.is_empty() {
        if !names.is_empty() {
            output.push_str(&format!(
                "[skills] {}
",
                names.join(", ")
            ));
        }
        output.push_str(manifest_json);
    }
    if include_content && !content_buf.is_empty() {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(content_buf.trim());
    }
    output
}

/// Gzips and base64-encodes the provided data.
pub(crate) fn gzip_base64(data: &str) -> Result<String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data.as_bytes())?;
    let compressed = encoder.finish()?;
    Ok(BASE64.encode(&compressed))
}

/// Applies size limits to the output, truncating or compressing if necessary.
fn apply_size_limit(
    mut output: String,
    max_bytes: usize,
    manifest_json: &str,
    include_manifest: bool,
    gzip_ok: bool,
    diagnostics: &mut Option<&mut Diagnostics>,
) -> Result<String> {
    if output.len() <= max_bytes {
        return Ok(output);
    }

    if include_manifest && !manifest_json.is_empty() && manifest_json.len() <= max_bytes {
        output = manifest_json.to_string();
        if let Some(d) = diagnostics.as_deref_mut() {
            d.truncated = true;
            d.truncated_content = true;
        }
        return Ok(output);
    }

    if include_manifest && gzip_ok {
        let gz = gzip_base64(manifest_json)?;
        let gz_wrapped = format!(r#"{{"skills_manifest_gzip_base64":"{}"}}"#, gz);
        if gz_wrapped.len() <= max_bytes {
            if let Some(d) = diagnostics.as_deref_mut() {
                d.truncated = true;
                d.truncated_content = true;
            }
            return Ok(gz_wrapped);
        }
        return Err(anyhow!(
            "autoload payload exceeds byte limit (even gzipped manifest)"
        ));
    }

    Err(anyhow!("autoload payload exceeds byte limit"))
}

/// Renders autoload content using custom skill readers (useful for testing).
pub(crate) fn render_autoload_with_reader<R, P>(
    skills: &[SkillMeta],
    mut opts: AutoloadOptions<'_, '_, '_, '_>,
    read_skill: R,
    read_prefix: P,
) -> Result<String>
where
    R: Fn(&SkillMeta) -> Result<String>,
    P: Fn(&SkillMeta, usize) -> Result<String>,
{
    if let Some(diag) = opts.diagnostics.as_deref_mut() {
        if opts.log_render_mode {
            diag.render_mode = Some(format!("{:?}", opts.render_mode));
        }
    }

    let include_manifest = opts.render_mode != RenderMode::ContentOnly;
    let include_content = opts.render_mode != RenderMode::ManifestOnly;

    let mut names = Vec::new();
    let mut manifest_entries = Vec::new();
    let mut content_buf = String::new();

    let embed_threshold = opts.embed_threshold.unwrap_or_else(env_embed_threshold);
    let prompt_for_embedding = opts.prompt.unwrap_or("");
    let mut prompt_terms = None;
    if opts.preload_terms.is_none() {
        if let Some(prompt) = opts.prompt {
            prompt_terms = Some(crate::discovery::tokenize_prompt(prompt));
        }
    }
    let term_opt = opts.preload_terms.or(prompt_terms.as_ref());
    let preview_len = opts
        .max_bytes
        .map(|max| max.saturating_div(4).clamp(64, DEFAULT_EMBED_PREVIEW_BYTES))
        .unwrap_or(DEFAULT_EMBED_PREVIEW_BYTES);

    for meta in skills {
        if !opts.include_claude
            && (meta.source == SkillSource::Claude
                || meta.source == SkillSource::Marketplace
                || meta.source == SkillSource::Cache)
        {
            if let Some(diag) = opts.diagnostics.as_deref_mut() {
                diag.skipped
                    .push((meta.name.clone(), "claude skills disabled".to_string()));
            }
            continue;
        }

        let is_pinned = opts
            .pinned
            .map(|pins| pins.contains(&meta.name))
            .unwrap_or(false);

        if !is_skill_relevant(
            meta,
            term_opt,
            is_pinned,
            prompt_for_embedding,
            embed_threshold,
            &read_prefix,
        ) {
            if let Some(diag) = opts.diagnostics.as_deref_mut() {
                diag.skipped
                    .push((meta.name.clone(), "not relevant to prompt".to_string()));
            }
            continue;
        }

        if let Some(m) = opts.matched.as_deref_mut() {
            m.insert(meta.name.clone());
        }

        names.push(meta.name.clone());
        if let Some(diag) = opts.diagnostics.as_deref_mut() {
            diag.included.push((
                meta.name.clone(),
                meta.source.label(),
                meta.root.to_string_lossy().into_owned(),
                meta.path.to_string_lossy().into_owned(),
            ));
        }

        if include_manifest {
            manifest_entries.push(build_manifest_entry(
                meta,
                opts.minimal_manifest,
                preview_len,
                &read_prefix,
            ));
        }

        if include_content {
            let text = read_skill(meta)?;
            if !content_buf.is_empty() {
                content_buf.push_str("\n\n");
            }
            content_buf.push_str(&text);
        }
    }

    let manifest_json = if include_manifest {
        serde_json::to_string(&json!({ "skills_manifest": manifest_entries }))?
    } else {
        String::new()
    };

    let output = assemble_output(
        &manifest_json,
        &names,
        &content_buf,
        include_manifest,
        include_content,
    );

    if let Some(max) = opts.max_bytes {
        apply_size_limit(
            output,
            max,
            &manifest_json,
            include_manifest,
            opts.gzip_ok,
            &mut opts.diagnostics,
        )
    } else {
        Ok(output)
    }
}

/// Renders autoload content using filesystem readers.
pub(crate) fn render_autoload(
    skills: &[SkillMeta],
    opts: AutoloadOptions<'_, '_, '_, '_>,
) -> Result<String> {
    render_autoload_with_reader(
        skills,
        opts,
        |meta| read_skill(&meta.path),
        |meta, max| read_prefix(&meta.path, max),
    )
}

/// Computes preview statistics without emitting full autoload content.
pub(crate) fn render_preview_stats(
    skills: &[SkillMeta],
    minimal_manifest: bool,
) -> Result<PreviewStats> {
    let mut stats = PreviewStats::default();
    if skills.is_empty() {
        return Ok(stats);
    }

    let mut manifest_entries = Vec::new();
    for meta in skills {
        stats.matched.push(meta.name.clone());
        manifest_entries.push(build_manifest_entry(
            meta,
            minimal_manifest,
            DEFAULT_EMBED_PREVIEW_BYTES,
            &|m, max| read_prefix(&m.path, max),
        ));
    }

    let manifest_json = serde_json::to_string(&json!({ "skills_manifest": manifest_entries }))?;
    stats.manifest_bytes = manifest_json.len();
    stats.estimated_tokens = (stats.manifest_bytes / 4).max(1);
    Ok(stats)
}
