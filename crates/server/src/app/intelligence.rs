//! Intelligence tool implementations for SkillService.
//!
//! Extracted from `app/mod.rs` to keep the main module under the 2500 LOC threshold
//! per ADR-0001. These methods provide smart recommendations, project context analysis,
//! skill gap detection, and skill creation capabilities.

use crate::setup;
use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, Content};
use serde_json::{json, Map as JsonMap, Value};
use skrills_state::home_dir;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::SkillService;

impl SkillService {
    // -------------------------------------------------------------------------
    // Intelligence Tools: Smart recommendations, project context, skill creation
    // -------------------------------------------------------------------------

    /// Smart skill recommendations combining dependency graph, usage patterns, and project context.
    pub(crate) fn recommend_skills_smart_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_analyze::analyze_skill;
        use skrills_intelligence::recommend::{RecommendationScorer, Scorer};
        use skrills_intelligence::usage::{
            parse_claude_sessions, parse_codex_sessions, parse_codex_skills_history,
            SkillUsageEvent,
        };
        use skrills_intelligence::{analyze_project, build_analytics, RecommendationSignal};

        let uri = args.get("uri").and_then(|v| v.as_str());
        let prompt = args.get("prompt").and_then(|v| v.as_str());
        let project_dir = args.get("project_dir").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let include_usage = args
            .get("include_usage")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let include_context = args
            .get("include_context")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Build scorer with optional usage and context data
        let mut scorer = RecommendationScorer::new();

        let (skills, _) = self.current_skills_with_dups()?;
        let mut path_to_uri: HashMap<PathBuf, String> = HashMap::new();
        for meta in &skills {
            let uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
            path_to_uri.insert(meta.path.clone(), uri.clone());
            if let Ok(canonical) = meta.path.canonicalize() {
                path_to_uri.insert(canonical, uri.clone());
            }
        }

        let normalize_usage_events =
            |events: Vec<SkillUsageEvent>, mapping: &HashMap<PathBuf, String>| {
                events
                    .into_iter()
                    .map(|mut event| {
                        if !event.skill_path.starts_with("skill://") {
                            let path = PathBuf::from(&event.skill_path);
                            if let Some(uri) = mapping.get(&path) {
                                event.skill_path = uri.clone();
                            } else if let Ok(canonical) = path.canonicalize() {
                                if let Some(uri) = mapping.get(&canonical) {
                                    event.skill_path = uri.clone();
                                }
                            }
                        }
                        event
                    })
                    .collect::<Vec<_>>()
            };

        // Load usage analytics if requested
        if include_usage {
            let mut events = Vec::new();
            let mut load_errors: Vec<String> = Vec::new();

            if let Some(home) = dirs::home_dir() {
                let claude_projects = home.join(".claude/projects");
                if claude_projects.exists() {
                    match parse_claude_sessions(&claude_projects) {
                        Ok(claude_events) => events.extend(claude_events),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                path = %claude_projects.display(),
                                "Failed to parse Claude session data"
                            );
                            load_errors.push(format!("Claude sessions: {}", e));
                        }
                    }
                }

                let codex_skills_history = home.join(".codex/skills-history.json");
                let mut codex_events = Vec::new();

                match parse_codex_skills_history(&codex_skills_history) {
                    Ok(history_events) => codex_events = history_events,
                    Err(e) => {
                        tracing::debug!(
                            error = %e,
                            path = %codex_skills_history.display(),
                            "Could not parse Codex skills history, trying sessions"
                        );
                    }
                }

                if codex_events.is_empty() {
                    let codex_sessions = home.join(".codex/sessions");
                    match parse_codex_sessions(&codex_sessions) {
                        Ok(session_events) => codex_events = session_events,
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                path = %codex_sessions.display(),
                                "Failed to parse Codex session data"
                            );
                            load_errors.push(format!("Codex sessions: {}", e));
                        }
                    }
                }
                events.extend(codex_events);
            } else {
                tracing::warn!("Could not determine home directory for usage analytics");
            }

            if !load_errors.is_empty() {
                tracing::info!(
                    errors = ?load_errors,
                    events_loaded = events.len(),
                    "Usage analytics loaded with some errors"
                );
            }

            if !events.is_empty() {
                let analytics = build_analytics(normalize_usage_events(events, &path_to_uri));
                scorer = scorer.with_usage(analytics);
            }
        }

        // Load project context if requested
        if include_context {
            if let Some(project_path) =
                resolve_project_dir(project_dir, "recommend_skills_smart_tool")
            {
                match analyze_project(&project_path) {
                    Ok(profile) => {
                        scorer = scorer.with_context(profile);
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "Could not analyze project context");
                    }
                }
            }
        }

        // Build quality scores for all skills
        let mut quality_scores: HashMap<String, f64> = HashMap::new();
        for meta in &skills {
            if let Ok(content) = fs::read_to_string(&meta.path) {
                let analysis = analyze_skill(&meta.path, &content);
                let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
                quality_scores.insert(skill_uri, analysis.quality_score);
            }
        }
        scorer = scorer.with_quality_scores(quality_scores);

        // Collect recommendations from multiple sources
        let mut all_recommendations = Vec::new();
        let mut seen_uris: HashSet<String> = HashSet::new();

        // If a URI is provided, use dependency-based recommendations
        if let Some(source_uri) = uri {
            let mut cache = self.cache.lock();
            cache.ensure_fresh()?;

            let dependencies: Vec<String> = cache.dependencies_raw(source_uri);
            let dependents: Vec<String> = cache.dependents_raw(source_uri);

            // Find siblings (skills sharing dependencies)
            let source_deps: HashSet<_> = dependencies.iter().cloned().collect();
            let all_uris = cache.skill_uris()?;
            let mut siblings: Vec<String> = Vec::new();

            for other_uri in &all_uris {
                if other_uri == source_uri {
                    continue;
                }
                if dependencies.contains(other_uri) || dependents.contains(other_uri) {
                    continue;
                }
                let other_deps: HashSet<_> =
                    cache.dependencies_raw(other_uri).into_iter().collect();
                if !source_deps.is_disjoint(&other_deps) {
                    siblings.push(other_uri.clone());
                }
            }

            // Score dependencies
            for dep_uri in &dependencies {
                if seen_uris.insert(dep_uri.clone()) {
                    let signals =
                        scorer.enhance_signals(dep_uri, vec![RecommendationSignal::Dependency]);
                    let rec = scorer.score(dep_uri, signals);
                    all_recommendations.push(rec);
                }
            }

            // Score dependents
            for dep_uri in &dependents {
                if seen_uris.insert(dep_uri.clone()) {
                    let signals =
                        scorer.enhance_signals(dep_uri, vec![RecommendationSignal::Dependent]);
                    let rec = scorer.score(dep_uri, signals);
                    all_recommendations.push(rec);
                }
            }

            // Score siblings
            for sib_uri in &siblings {
                if seen_uris.insert(sib_uri.clone()) {
                    let signals =
                        scorer.enhance_signals(sib_uri, vec![RecommendationSignal::Sibling]);
                    let rec = scorer.score(sib_uri, signals);
                    all_recommendations.push(rec);
                }
            }
        }

        // If a prompt is provided, add prompt-matching recommendations
        if let Some(query) = prompt {
            let keywords: Vec<String> =
                query.split_whitespace().map(|s| s.to_lowercase()).collect();

            for meta in &skills {
                let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
                if seen_uris.contains(&skill_uri) {
                    continue;
                }

                let name_lower = meta.name.to_lowercase();
                let matched_keywords: Vec<String> = keywords
                    .iter()
                    .filter(|kw| name_lower.contains(kw.as_str()))
                    .cloned()
                    .collect();

                if !matched_keywords.is_empty() {
                    seen_uris.insert(skill_uri.clone());
                    let signals = scorer.enhance_signals(
                        &skill_uri,
                        vec![RecommendationSignal::PromptMatch {
                            keywords: matched_keywords,
                        }],
                    );
                    let rec = scorer.score(&skill_uri, signals);
                    all_recommendations.push(rec);
                }
            }
        }

        // If no URI or prompt, recommend based on project context and usage
        if uri.is_none() && prompt.is_none() {
            for meta in &skills {
                let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
                if seen_uris.contains(&skill_uri) {
                    continue;
                }

                let signals = scorer.enhance_signals(&skill_uri, vec![]);
                if !signals.is_empty() {
                    seen_uris.insert(skill_uri.clone());
                    let rec = scorer.score(&skill_uri, signals);
                    all_recommendations.push(rec);
                }
            }
        }

        // Sort by score descending
        all_recommendations.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_found = all_recommendations.len();
        all_recommendations.truncate(limit);

        let text = format!(
            "Found {} smart recommendations (showing top {})",
            total_found,
            all_recommendations.len()
        );

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(json!({
                "total_found": total_found,
                "recommendations": all_recommendations,
                "include_usage": include_usage,
                "include_context": include_context,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Analyze project context for skill recommendations.
    pub(crate) fn analyze_project_context_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_intelligence::{analyze_project_with_options, AnalyzeProjectOptions};

        let project_dir = resolve_project_dir(
            args.get("project_dir").and_then(|v| v.as_str()),
            "analyze_project_context_tool",
        )
        .ok_or_else(|| anyhow!("Could not determine current directory; provide project_dir"))?;

        let include_git = args
            .get("include_git")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let commit_limit = args
            .get("commit_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;
        let max_languages = args
            .get("max_languages")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;
        let options = AnalyzeProjectOptions {
            include_git,
            commit_limit,
            max_languages,
        };

        let profile = analyze_project_with_options(&project_dir, options)?;

        // Build summary text
        let primary_langs: Vec<&str> = profile
            .languages
            .iter()
            .filter(|(_, info)| info.primary)
            .map(|(name, _)| name.as_str())
            .collect();
        let lang_summary = if primary_langs.is_empty() {
            "No primary languages detected".to_string()
        } else {
            format!("Primary: {}", primary_langs.join(", "))
        };

        let text = format!(
            "Project: {:?}\nLanguages: {} ({} total)\nFrameworks: {}\nDependencies: {} packages",
            profile.project_type,
            lang_summary,
            profile.languages.len(),
            if profile.frameworks.is_empty() {
                "none detected".to_string()
            } else {
                profile.frameworks.join(", ")
            },
            profile
                .dependencies
                .values()
                .map(|v| v.len())
                .sum::<usize>()
        );

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(serde_json::to_value(&profile)?),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Suggest new skills to create based on project needs.
    pub(crate) fn suggest_new_skills_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_intelligence::{analyze_project, SkillGap, SkillGapAnalysis};

        let project_dir = resolve_project_dir(
            args.get("project_dir").and_then(|v| v.as_str()),
            "suggest_new_skills_tool",
        )
        .ok_or_else(|| anyhow!("Could not determine current directory; provide project_dir"))?;
        let focus_areas: Vec<String> = args
            .get("focus_areas")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let profile = analyze_project(&project_dir)?;

        // Get current skills to compare against
        let (skills, _) = self.current_skills_with_dups()?;
        let skill_names: HashSet<String> = skills.iter().map(|s| s.name.to_lowercase()).collect();

        // Identify gaps based on project context
        let mut gaps = Vec::new();
        let mut suggestions = Vec::new();

        // Check for language-specific skill gaps
        for (lang, info) in &profile.languages {
            if info.primary {
                let lang_lower = lang.to_lowercase();
                let has_lang_skill = skill_names.iter().any(|n| n.contains(&lang_lower));
                if !has_lang_skill {
                    gaps.push(SkillGap {
                        area: format!("{} development", lang),
                        evidence: vec![format!(
                            "{} files detected as primary language",
                            info.file_count
                        )],
                        priority: "high".to_string(),
                    });
                    suggestions.push(format!("{}-best-practices", lang_lower));
                }
            }
        }

        // Check for framework-specific skill gaps
        for framework in &profile.frameworks {
            let fw_lower = framework.to_lowercase();
            let has_fw_skill = skill_names.iter().any(|n| n.contains(&fw_lower));
            if !has_fw_skill {
                gaps.push(SkillGap {
                    area: format!("{} framework", framework),
                    evidence: vec![format!("{} detected in project", framework)],
                    priority: "medium".to_string(),
                });
                suggestions.push(format!("{}-patterns", fw_lower));
            }
        }

        // Check for focus area gaps
        for area in &focus_areas {
            let area_lower = area.to_lowercase();
            let has_area_skill = skill_names.iter().any(|n| n.contains(&area_lower));
            if !has_area_skill {
                gaps.push(SkillGap {
                    area: area.clone(),
                    evidence: vec!["Requested focus area".to_string()],
                    priority: "high".to_string(),
                });
                suggestions.push(format!("{}-workflow", area_lower));
            }
        }

        // Check for common patterns without skills
        let common_patterns = [
            ("testing", vec!["test", "spec", "mock"]),
            ("deployment", vec!["deploy", "ci", "cd", "docker"]),
            ("documentation", vec!["doc", "readme", "wiki"]),
            ("security", vec!["auth", "security", "encryption"]),
        ];

        for (area, patterns) in common_patterns {
            if !focus_areas.is_empty() && !focus_areas.iter().any(|f| f.to_lowercase() == area) {
                continue;
            }
            let has_area_skill = skill_names
                .iter()
                .any(|n| patterns.iter().any(|p| n.contains(p)));
            if !has_area_skill {
                // Check if this area is relevant to the project
                let relevant = profile
                    .git_keywords
                    .iter()
                    .any(|kw| patterns.iter().any(|p| kw.contains(p)))
                    || profile
                        .keywords
                        .iter()
                        .any(|kw| patterns.iter().any(|p| kw.contains(p)));
                if relevant || focus_areas.iter().any(|f| f.to_lowercase() == area) {
                    gaps.push(SkillGap {
                        area: area.to_string(),
                        evidence: vec!["Common development pattern".to_string()],
                        priority: "low".to_string(),
                    });
                }
            }
        }

        let analysis = SkillGapAnalysis { gaps, suggestions };

        let text = format!(
            "Found {} skill gaps with {} suggestions",
            analysis.gaps.len(),
            analysis.suggestions.len()
        );

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(serde_json::to_value(&analysis)?),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Create a new skill via GitHub search, LLM generation, or both.
    pub(crate) async fn create_skill_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_intelligence::{
            analyze_project, generate_skill_with_llm, search_github_skills, CreateSkillRequest,
            CreationMethod,
        };

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: name"))?;

        // Security: Reject path traversal attempts and hidden files
        if name.contains("..") {
            return Err(anyhow!(
                "Invalid name: path traversal sequences ('..') are not allowed"
            ));
        }
        if name.starts_with('.') {
            return Err(anyhow!(
                "Invalid name: hidden files (starting with '.') are not allowed"
            ));
        }

        let mut name_components = std::path::Path::new(name).components();
        let is_simple_name = matches!(
            name_components.next(),
            Some(std::path::Component::Normal(_))
        ) && name_components.next().is_none();
        if !is_simple_name {
            return Err(anyhow!(
                "Invalid name: use a simple directory name without path separators"
            ));
        }
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: description"))?;
        let method_str = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("both");
        let target_dir = args.get("target_dir").and_then(|v| v.as_str());
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let project_dir = args.get("project_dir").and_then(|v| v.as_str());

        let method: CreationMethod = method_str
            .parse()
            .map_err(|err| anyhow!("Invalid creation method: {}", err))?;

        // Build request with optional project context
        let mut request = CreateSkillRequest::new(name, description);
        request.method = method.clone();
        request.dry_run = dry_run;

        if let Some(dir) = target_dir {
            request = request.with_target_dir(dir);
        }

        // Add project context if available
        if let Some(dir) = project_dir {
            if let Ok(profile) = analyze_project(&PathBuf::from(dir)) {
                request = request.with_context(profile);
            }
        }

        let mut github_results = Vec::new();
        let mut llm_content: Option<String> = None;
        let mut errors = Vec::new();

        // GitHub search
        if method == CreationMethod::GitHubSearch || method == CreationMethod::Both {
            let query = format!("{} {}", name, description);
            match search_github_skills(&query, 5).await {
                Ok(results) => {
                    github_results = results;
                }
                Err(e) => {
                    errors.push(format!("GitHub search failed: {}", e));
                }
            }
        }

        // LLM generation
        if method == CreationMethod::LLMGenerate || method == CreationMethod::Both {
            match generate_skill_with_llm(&request).await {
                Ok(result) => {
                    if result.success {
                        llm_content = result.content;
                    } else if let Some(err) = result.error {
                        errors.push(format!("LLM generation failed: {}", err));
                    }
                }
                Err(e) => {
                    errors.push(format!("LLM generation error: {}", e));
                }
            }
        }

        // Empirical generation from session patterns
        if method == CreationMethod::Empirical {
            use skrills_intelligence::parse_claude_sessions;

            // Try to load session data from Claude projects directory
            let claude_projects = match skrills_state::home_dir() {
                Ok(home) => home.join(".claude").join("projects"),
                Err(e) => {
                    let error_msg = format!(
                        "Could not determine home directory: {}. \
                         Use --method llm or --method both instead.",
                        e
                    );
                    errors.push(error_msg.clone());
                    return Ok(CallToolResult {
                        content: vec![Content::text(error_msg)],
                        structured_content: Some(json!({
                            "success": false,
                            "method": method_str,
                            "name": name,
                            "errors": errors,
                        })),
                        is_error: Some(true),
                        meta: None,
                    });
                }
            };

            if claude_projects.exists() {
                match parse_claude_sessions(&claude_projects) {
                    Ok(events) if events.len() >= 10 => {
                        // Empirical generation requires BehavioralEvent data with tool
                        // sequences and file accesses. Currently we only have basic
                        // SkillUsageEvent data. This feature is in preview.
                        let preview_msg = format!(
                            "Found {} session events. Empirical skill generation from \
                             behavioral patterns is in preview. Use --method llm or \
                             --method both for production use.",
                            events.len()
                        );
                        return Ok(CallToolResult {
                            content: vec![Content::text(&preview_msg)],
                            structured_content: Some(json!({
                                "success": true,
                                "method": method_str,
                                "name": name,
                                "dry_run": dry_run,
                                "preview": true,
                                "session_events": events.len(),
                                "message": preview_msg,
                            })),
                            is_error: Some(false),
                            meta: None,
                        });
                    }
                    Ok(events) => {
                        errors.push(format!(
                            "Found {} session events, but empirical generation requires \
                             at least 10 sessions with detailed behavioral data. \
                             Use Claude Code more to build up patterns, or use \
                             --method llm or --method both.",
                            events.len()
                        ));
                    }
                    Err(e) => {
                        errors.push(format!("Failed to parse session history: {}", e));
                    }
                }
            } else {
                errors.push(format!(
                    "Session history directory not found: {}. \
                     Use Claude Code to build up behavioral patterns, or use \
                     --method llm or --method both.",
                    claude_projects.display()
                ));
            }
        }

        // Determine success
        let has_github = !github_results.is_empty();
        let has_llm = llm_content.is_some();
        let mut success = has_github || has_llm;

        // Write skill file if not dry run and we have content
        let mut written_path: Option<String> = None;
        let write_attempted = !dry_run && has_llm;
        if write_attempted {
            if let Some(ref content) = llm_content {
                let target = target_dir
                    .map(|s| shellexpand::tilde(s).to_string())
                    .unwrap_or_else(|| default_skill_target_dir().display().to_string());
                let skill_dir = PathBuf::from(&target).join(name);
                if let Err(e) = fs::create_dir_all(&skill_dir) {
                    errors.push(format!("Failed to create directory: {}", e));
                } else {
                    let skill_path = skill_dir.join("SKILL.md");
                    match fs::write(&skill_path, content) {
                        Ok(()) => {
                            written_path = Some(skill_path.display().to_string());
                        }
                        Err(e) => {
                            errors.push(format!("Failed to write skill file: {}", e));
                        }
                    }
                }
            }
        }
        if write_attempted && written_path.is_none() {
            success = false;
        }

        let text = if success {
            let mut parts = Vec::new();
            if has_github {
                parts.push(format!("Found {} GitHub results", github_results.len()));
            }
            if has_llm {
                parts.push("Generated skill via LLM".to_string());
            }
            if let Some(ref path) = written_path {
                parts.push(format!("Written to {}", path));
            }
            if dry_run {
                parts.push("(dry run)".to_string());
            }
            parts.join(". ")
        } else {
            format!("Failed to create skill: {}", errors.join("; "))
        };

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(json!({
                "success": success,
                "method": method_str,
                "name": name,
                "dry_run": dry_run,
                "github_results": github_results,
                "llm_content": llm_content,
                "written_path": written_path,
                "errors": errors,
            })),
            is_error: Some(!success),
            meta: None,
        })
    }

    /// Search GitHub for existing SKILL.md files.
    pub(crate) async fn search_skills_github_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_intelligence::search_github_skills;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: query"))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let results = search_github_skills(query, limit).await?;

        let text = if results.is_empty() {
            "No SKILL.md files found on GitHub matching the query".to_string()
        } else {
            format!(
                "Found {} SKILL.md files on GitHub:\n{}",
                results.len(),
                results
                    .iter()
                    .take(5)
                    .map(|r| format!("  - {} ({}★)", r.skill_path, r.stars))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(json!({
                "query": query,
                "total_found": results.len(),
                "results": results,
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Sync wrapper for create-skill in CLI contexts.
    pub(crate) fn create_skill_tool_sync(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(self.create_skill_tool(args))
    }

    /// Sync wrapper for search-skills-github in CLI contexts.
    pub(crate) fn search_skills_github_tool_sync(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(self.search_skills_github_tool(args))
    }

    /// Fuzzy search for installed skills using trigram matching.
    ///
    /// Tolerates typos and finds similar skill names.
    pub(crate) fn search_skills_fuzzy_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use anyhow::Context;
        use skrills_intelligence::{find_similar_skills, SkillInfo, DEFAULT_THRESHOLD};

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing required parameter: query"))?;

        // Validate and clamp threshold to valid range [0.0, 1.0]
        let threshold = args
            .get("threshold")
            .and_then(|v| v.as_f64())
            .map(|t| t.clamp(0.0, 1.0))
            .unwrap_or(DEFAULT_THRESHOLD);

        // Validate and clamp limit to reasonable bounds [1, 1000]
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|l| l.clamp(1, 1000) as usize)
            .unwrap_or(10);

        // Whether to include description matching (default: true for richer results)
        let include_description = args
            .get("include_description")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        tracing::debug!(
            query = %query,
            threshold = %threshold,
            limit = %limit,
            include_description = %include_description,
            "Starting fuzzy skill search"
        );

        // Get all skills
        let (skills, _) = self
            .current_skills_with_dups()
            .context("Failed to load skills for fuzzy search")?;

        // Build skill info for matching
        // Only include descriptions if include_description is true
        let skill_infos: Vec<(String, String, Option<String>)> = skills
            .iter()
            .map(|meta| {
                let uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
                let desc = if include_description {
                    meta.description.clone()
                } else {
                    None
                };
                (uri, meta.name.clone(), desc)
            })
            .collect();

        let skill_info_refs: Vec<SkillInfo<'_>> = skill_infos
            .iter()
            .map(|(uri, name, desc)| SkillInfo {
                uri: uri.as_str(),
                name: name.as_str(),
                description: desc.as_deref(),
            })
            .collect();

        // Perform fuzzy matching
        let matches = find_similar_skills(query, skill_info_refs, threshold);

        // Limit results
        let results: Vec<_> = matches.into_iter().take(limit).collect();

        tracing::debug!(
            query = %query,
            total_skills = skills.len(),
            matches_found = results.len(),
            "Fuzzy skill search completed"
        );

        let text = if results.is_empty() {
            format!(
                "No skills found matching '{}' (threshold: {:.1}%)",
                query,
                threshold * 100.0
            )
        } else {
            let mut lines = vec![format!(
                "Found {} skills matching '{}' (threshold: {:.1}%):",
                results.len(),
                query,
                threshold * 100.0
            )];
            for m in &results {
                lines.push(format!(
                    "  - {} ({:.0}% match via {:?})",
                    m.name,
                    m.similarity * 100.0,
                    m.matched_field
                ));
            }
            lines.join("\n")
        };

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(json!({
                "query": query,
                "threshold": threshold,
                "total_found": results.len(),
                "results": results.iter().map(|m| json!({
                    "uri": m.uri,
                    "name": m.name,
                    "description": m.description,
                    "similarity": m.similarity,
                    "matched_field": format!("{:?}", m.matched_field),
                })).collect::<Vec<_>>(),
            })),
            is_error: Some(false),
            meta: None,
        })
    }
}

// -------------------------------------------------------------------------
// Helper Functions
// -------------------------------------------------------------------------

pub(crate) fn select_default_skill_root(
    home: &Path,
    claude_setup: bool,
    codex_setup: bool,
) -> PathBuf {
    if claude_setup || !codex_setup {
        home.join(".claude/skills")
    } else {
        home.join(".codex/skills")
    }
}

fn default_skill_target_dir() -> PathBuf {
    let Ok(home) = home_dir() else {
        return PathBuf::from("skills");
    };

    let claude_setup = setup::is_setup(setup::Client::Claude).unwrap_or(false);
    let codex_setup = setup::is_setup(setup::Client::Codex).unwrap_or(false);
    select_default_skill_root(&home, claude_setup, codex_setup)
}

pub(crate) fn resolve_project_dir(project_dir: Option<&str>, context: &str) -> Option<PathBuf> {
    if let Some(dir) = project_dir {
        return Some(PathBuf::from(dir));
    }

    match std::env::current_dir() {
        Ok(dir) => Some(dir),
        Err(err) => {
            tracing::warn!(
                error = %err,
                context,
                "Could not determine current directory"
            );
            None
        }
    }
}
