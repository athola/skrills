//! Empirical skill generation from clustered behavioral patterns.
//!
//! This module creates skills from observed successful patterns in session data,
//! with guardrails derived from observed failures. Rather than generating skills
//! purely from LLM imagination, it mines real usage patterns for grounded guidance.

use crate::usage::behavioral::{extract_common_ngrams, BehavioralEvent, OutcomeStatus, ToolStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[cfg(test)]
use crate::usage::behavioral::{SessionOutcome, ToolCall};

// ============================================================================
// Pattern Structures
// ============================================================================

/// A successful pattern extracted from session analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessPattern {
    /// Sequence of actions that led to success.
    pub action_sequence: Vec<String>,
    /// Keywords that commonly trigger this pattern.
    pub trigger_keywords: Vec<String>,
    /// Confidence score based on frequency (0.0 - 1.0).
    pub confidence: f64,
    /// Number of sessions where this pattern appeared.
    pub occurrence_count: usize,
    /// Example prompt contexts where this pattern succeeded.
    pub example_contexts: Vec<String>,
}

/// A failure pattern to avoid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePattern {
    /// Sequence of actions that led to failure.
    pub action_sequence: Vec<String>,
    /// Error messages commonly associated with this pattern.
    pub error_messages: Vec<String>,
    /// Suggested recovery or alternative actions.
    pub recovery_suggestions: Vec<String>,
    /// How many times this failure pattern was observed.
    pub occurrence_count: usize,
}

/// A cluster of similar behavioral patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteredBehavior {
    /// Unique identifier for this cluster.
    pub cluster_id: String,
    /// Label describing the cluster (auto-generated).
    pub label: String,
    /// Session IDs in this cluster.
    pub session_ids: Vec<String>,
    /// Extracted success patterns.
    pub success_patterns: Vec<SuccessPattern>,
    /// Extracted failure patterns (anti-patterns).
    pub failure_patterns: Vec<FailurePattern>,
    /// Common file types accessed in this cluster.
    pub common_file_types: Vec<String>,
    /// Dominant skill category inferred from patterns.
    pub inferred_category: String,
    /// Size of the cluster.
    pub size: usize,
}

/// Result of clustering analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusteringResult {
    /// Identified behavioral clusters.
    pub clusters: Vec<ClusteredBehavior>,
    /// Sessions that didn't fit any cluster (outliers).
    pub outlier_sessions: Vec<String>,
    /// Total sessions analyzed.
    pub total_sessions: usize,
    /// Clustering quality metric (0.0 - 1.0).
    pub quality_score: f64,
}

/// Generated skill content from empirical patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmpiricalSkillContent {
    /// Skill name derived from patterns.
    pub name: String,
    /// Auto-generated description.
    pub description: String,
    /// Frontmatter YAML content.
    pub frontmatter: String,
    /// Main skill body (markdown).
    pub body: String,
    /// Guardrails section (warnings based on failure patterns).
    pub guardrails: String,
    /// Confidence in this skill (based on pattern strength).
    pub confidence: f64,
    /// Source cluster ID.
    pub source_cluster_id: String,
}

// ============================================================================
// Feature Extraction for Clustering
// ============================================================================

/// Features extracted from a session for clustering.
#[derive(Debug, Clone, Default)]
struct SessionFeatures {
    /// Session identifier.
    session_id: String,
    /// Tool frequency vector (tool_name -> count).
    tool_frequency: HashMap<String, usize>,
    /// File extension frequency.
    file_extensions: HashMap<String, usize>,
    /// Average time between tool calls.
    avg_tool_interval_ms: f64,
    /// Error rate in session.
    error_rate: f64,
    /// Session duration.
    duration_ms: u64,
    /// Outcome status if known.
    outcome: Option<OutcomeStatus>,
}

/// Extract features from behavioral events.
fn extract_session_features(events: &[BehavioralEvent]) -> Vec<SessionFeatures> {
    // Group events by session
    let mut session_map: HashMap<String, Vec<&BehavioralEvent>> = HashMap::new();
    for event in events {
        session_map
            .entry(event.skill_usage.session_id.clone())
            .or_default()
            .push(event);
    }

    session_map
        .into_iter()
        .map(|(session_id, session_events)| {
            let mut features = SessionFeatures {
                session_id,
                ..Default::default()
            };

            // Aggregate tool frequencies
            for event in &session_events {
                for tool_call in &event.tool_sequence {
                    *features
                        .tool_frequency
                        .entry(tool_call.name.clone())
                        .or_insert(0) += 1;
                }

                // Extract file extensions
                for file_access in &event.files_accessed {
                    if let Some(ext) = std::path::Path::new(&file_access.path)
                        .extension()
                        .and_then(|e| e.to_str())
                    {
                        *features.file_extensions.entry(ext.to_string()).or_insert(0) += 1;
                    }
                }
            }

            // Calculate error rate
            let total_tools: usize = session_events.iter().map(|e| e.tool_sequence.len()).sum();
            let error_tools: usize = session_events
                .iter()
                .flat_map(|e| &e.tool_sequence)
                .filter(|t| matches!(t.status, ToolStatus::Error { .. }))
                .count();

            features.error_rate = if total_tools > 0 {
                error_tools as f64 / total_tools as f64
            } else {
                0.0
            };

            // Get outcome if available
            if let Some(event) = session_events.first() {
                if let Some(ref outcome) = event.session_outcome {
                    features.outcome = Some(outcome.status.clone());
                }
            }

            // Calculate duration and intervals
            let timestamps: Vec<u64> = session_events
                .iter()
                .flat_map(|e| e.tool_sequence.iter().map(|t| t.timestamp))
                .collect();

            if timestamps.len() > 1 {
                let min_ts = *timestamps.iter().min().unwrap_or(&0);
                let max_ts = *timestamps.iter().max().unwrap_or(&0);
                features.duration_ms = max_ts.saturating_sub(min_ts) * 1000;

                let intervals: Vec<u64> = timestamps
                    .windows(2)
                    .map(|w| w[1].saturating_sub(w[0]))
                    .collect();
                if !intervals.is_empty() {
                    features.avg_tool_interval_ms =
                        intervals.iter().sum::<u64>() as f64 / intervals.len() as f64 * 1000.0;
                }
            }

            features
        })
        .collect()
}

// ============================================================================
// Clustering Algorithm (Simple K-means-like)
// ============================================================================

/// Compute distance between two session feature vectors.
/// Used for advanced k-means clustering (currently using simpler grouping).
#[allow(dead_code)]
fn compute_feature_distance(a: &SessionFeatures, b: &SessionFeatures) -> f64 {
    let mut distance = 0.0;

    // Tool frequency similarity (Jaccard-like)
    let a_tools: HashSet<_> = a.tool_frequency.keys().collect();
    let b_tools: HashSet<_> = b.tool_frequency.keys().collect();
    let intersection = a_tools.intersection(&b_tools).count();
    let union = a_tools.union(&b_tools).count();
    let tool_similarity = if union > 0 {
        intersection as f64 / union as f64
    } else {
        0.0
    };
    distance += 1.0 - tool_similarity;

    // File extension similarity
    let a_exts: HashSet<_> = a.file_extensions.keys().collect();
    let b_exts: HashSet<_> = b.file_extensions.keys().collect();
    let ext_intersection = a_exts.intersection(&b_exts).count();
    let ext_union = a_exts.union(&b_exts).count();
    let ext_similarity = if ext_union > 0 {
        ext_intersection as f64 / ext_union as f64
    } else {
        0.0
    };
    distance += 1.0 - ext_similarity;

    // Error rate difference
    distance += (a.error_rate - b.error_rate).abs();

    // Outcome match bonus (reduce distance if same outcome)
    if a.outcome.is_some() && a.outcome == b.outcome {
        distance -= 0.5;
    }

    distance.max(0.0)
}

/// Cluster sessions by behavioral similarity.
pub fn cluster_sessions(
    events: &[BehavioralEvent],
    num_clusters: usize,
    min_cluster_size: usize,
) -> ClusteringResult {
    if events.is_empty() {
        return ClusteringResult::default();
    }

    let features = extract_session_features(events);
    if features.len() < num_clusters * min_cluster_size {
        // Not enough data for meaningful clustering
        return ClusteringResult {
            total_sessions: features.len(),
            outlier_sessions: features.iter().map(|f| f.session_id.clone()).collect(),
            ..Default::default()
        };
    }

    // Simple clustering: group by dominant tool pattern
    let mut clusters: HashMap<String, Vec<&SessionFeatures>> = HashMap::new();

    for feature in &features {
        // Find dominant tool
        let dominant_tool = feature
            .tool_frequency
            .iter()
            .max_by_key(|(_, &count)| count)
            .map(|(tool, _)| tool.clone())
            .unwrap_or_else(|| "unknown".to_string());

        // Secondary grouping by file type
        let dominant_ext = feature
            .file_extensions
            .iter()
            .max_by_key(|(_, &count)| count)
            .map(|(ext, _)| ext.clone())
            .unwrap_or_else(|| "none".to_string());

        let cluster_key = format!("{}:{}", dominant_tool, dominant_ext);
        clusters.entry(cluster_key).or_default().push(feature);
    }

    // Convert to ClusteredBehavior
    let mut result_clusters = Vec::new();
    let mut outliers = Vec::new();

    for (cluster_key, cluster_features) in clusters {
        if cluster_features.len() < min_cluster_size {
            // Too small, mark as outliers
            outliers.extend(cluster_features.iter().map(|f| f.session_id.clone()));
            continue;
        }

        let session_ids: Vec<String> = cluster_features
            .iter()
            .map(|f| f.session_id.clone())
            .collect();

        // Extract patterns from this cluster
        let cluster_events: Vec<_> = events
            .iter()
            .filter(|e| session_ids.contains(&e.skill_usage.session_id))
            .collect();

        let success_patterns = extract_success_patterns(&cluster_events);
        let failure_patterns = extract_failure_patterns(&cluster_events);

        // Determine common file types
        let mut ext_counts: HashMap<String, usize> = HashMap::new();
        for feature in &cluster_features {
            for (ext, count) in &feature.file_extensions {
                *ext_counts.entry(ext.clone()).or_insert(0) += count;
            }
        }
        let common_file_types: Vec<_> = ext_counts
            .into_iter()
            .filter(|(_, count)| *count > cluster_features.len() / 2)
            .map(|(ext, _)| ext)
            .collect();

        // Infer category from cluster key
        let inferred_category = infer_category_from_patterns(&cluster_key, &success_patterns);

        result_clusters.push(ClusteredBehavior {
            cluster_id: format!("cluster-{}", result_clusters.len()),
            label: cluster_key,
            size: cluster_features.len(),
            session_ids,
            success_patterns,
            failure_patterns,
            common_file_types,
            inferred_category,
        });
    }

    // Limit to requested number of clusters (keep largest)
    result_clusters.sort_by(|a, b| b.size.cmp(&a.size));
    result_clusters.truncate(num_clusters);

    // Calculate quality score
    let clustered_sessions: usize = result_clusters.iter().map(|c| c.size).sum();
    let quality_score = if !features.is_empty() {
        clustered_sessions as f64 / features.len() as f64
    } else {
        0.0
    };

    ClusteringResult {
        clusters: result_clusters,
        outlier_sessions: outliers,
        total_sessions: features.len(),
        quality_score,
    }
}

/// Infer skill category from cluster patterns.
fn infer_category_from_patterns(cluster_key: &str, success_patterns: &[SuccessPattern]) -> String {
    let lower = cluster_key.to_lowercase();

    if lower.contains("bash") || lower.contains("test") {
        return "testing".to_string();
    }
    if lower.contains("read") && lower.contains("rs") {
        return "rust-development".to_string();
    }
    if lower.contains("read") && lower.contains("py") {
        return "python-development".to_string();
    }
    if lower.contains("read") && (lower.contains("ts") || lower.contains("js")) {
        return "typescript-development".to_string();
    }
    if lower.contains("write") || lower.contains("edit") {
        return "code-modification".to_string();
    }

    // Check success patterns for additional hints
    for pattern in success_patterns {
        for keyword in &pattern.trigger_keywords {
            let kw_lower = keyword.to_lowercase();
            if kw_lower.contains("test") {
                return "testing".to_string();
            }
            if kw_lower.contains("fix") || kw_lower.contains("debug") {
                return "debugging".to_string();
            }
            if kw_lower.contains("doc") {
                return "documentation".to_string();
            }
        }
    }

    "general".to_string()
}

// ============================================================================
// Pattern Extraction
// ============================================================================

/// Extract success patterns from clustered events.
fn extract_success_patterns(events: &[&BehavioralEvent]) -> Vec<SuccessPattern> {
    let mut patterns = Vec::new();

    // Get successful sessions
    let successful_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.session_outcome
                .as_ref()
                .map(|o| o.status == OutcomeStatus::Success)
                .unwrap_or(false)
        })
        .collect();

    if successful_events.is_empty() {
        return patterns;
    }

    // Extract tool sequences from successful sessions
    let sequences: Vec<Vec<String>> = successful_events
        .iter()
        .map(|e| e.tool_sequence.iter().map(|t| t.name.clone()).collect())
        .collect();

    // Find common n-grams (bigrams and trigrams)
    let common_bigrams = extract_common_ngrams(&sequences, 2, 2);
    let common_trigrams = extract_common_ngrams(&sequences, 3, 2);

    // Convert to success patterns
    for (sequence, count) in common_bigrams.into_iter().take(5) {
        let keywords = extract_trigger_keywords(&successful_events, &sequence);
        let confidence = count as f64 / successful_events.len() as f64;

        patterns.push(SuccessPattern {
            action_sequence: sequence,
            trigger_keywords: keywords,
            confidence: confidence.min(1.0),
            occurrence_count: count,
            example_contexts: successful_events
                .iter()
                .filter_map(|e| e.skill_usage.prompt_context.clone())
                .take(3)
                .collect(),
        });
    }

    for (sequence, count) in common_trigrams.into_iter().take(3) {
        let keywords = extract_trigger_keywords(&successful_events, &sequence);
        let confidence = count as f64 / successful_events.len() as f64;

        patterns.push(SuccessPattern {
            action_sequence: sequence,
            trigger_keywords: keywords,
            confidence: confidence.min(1.0),
            occurrence_count: count,
            example_contexts: successful_events
                .iter()
                .filter_map(|e| e.skill_usage.prompt_context.clone())
                .take(3)
                .collect(),
        });
    }

    // Deduplicate and sort by confidence
    patterns.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    patterns.dedup_by(|a, b| a.action_sequence == b.action_sequence);

    patterns
}

/// Extract failure patterns from clustered events.
fn extract_failure_patterns(events: &[&BehavioralEvent]) -> Vec<FailurePattern> {
    let mut patterns = Vec::new();

    // Get failed sessions
    let failed_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.session_outcome
                .as_ref()
                .map(|o| o.status == OutcomeStatus::Failure)
                .unwrap_or(false)
        })
        .collect();

    if failed_events.is_empty() {
        return patterns;
    }

    // Extract error messages
    let mut error_sequences: HashMap<Vec<String>, Vec<String>> = HashMap::new();

    for event in &failed_events {
        let error_tools: Vec<_> = event
            .tool_sequence
            .iter()
            .filter(|t| matches!(t.status, ToolStatus::Error { .. }))
            .collect();

        if !error_tools.is_empty() {
            let sequence: Vec<String> = error_tools.iter().map(|t| t.name.clone()).collect();
            let errors: Vec<String> = error_tools
                .iter()
                .filter_map(|t| {
                    if let ToolStatus::Error { message } = &t.status {
                        Some(message.clone())
                    } else {
                        None
                    }
                })
                .collect();

            error_sequences.entry(sequence).or_default().extend(errors);
        }
    }

    // Convert to failure patterns
    for (sequence, errors) in error_sequences {
        if sequence.is_empty() {
            continue;
        }

        // Deduplicate and limit errors
        let unique_errors: Vec<_> = errors
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .take(5)
            .collect();

        // Generate recovery suggestions based on error types
        let recovery_suggestions = generate_recovery_suggestions(&unique_errors);

        patterns.push(FailurePattern {
            action_sequence: sequence,
            error_messages: unique_errors,
            recovery_suggestions,
            occurrence_count: failed_events.len(),
        });
    }

    patterns
}

/// Extract trigger keywords from events that match a pattern.
fn extract_trigger_keywords(events: &[&&BehavioralEvent], _pattern: &[String]) -> Vec<String> {
    let mut keywords: HashMap<String, usize> = HashMap::new();

    for event in events {
        if let Some(ref context) = event.skill_usage.prompt_context {
            // Extract words from context
            for word in context
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| s.len() >= 3)
            {
                *keywords.entry(word.to_string()).or_insert(0) += 1;
            }
        }
    }

    // Return top keywords
    let mut sorted: Vec<_> = keywords.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted.into_iter().take(10).map(|(k, _)| k).collect()
}

/// Generate recovery suggestions from error messages.
fn generate_recovery_suggestions(errors: &[String]) -> Vec<String> {
    let mut suggestions = Vec::new();

    for error in errors {
        let lower = error.to_lowercase();

        if lower.contains("not found") || lower.contains("no such file") {
            suggestions.push("Verify the file path exists before accessing".to_string());
        }
        if lower.contains("permission denied") {
            suggestions.push("Check file permissions or use appropriate privileges".to_string());
        }
        if lower.contains("syntax error") || lower.contains("parse error") {
            suggestions.push("Validate code syntax before execution".to_string());
        }
        if lower.contains("timeout") {
            suggestions.push("Consider breaking the operation into smaller steps".to_string());
        }
        if lower.contains("connection") || lower.contains("network") {
            suggestions.push("Verify network connectivity and API endpoints".to_string());
        }
    }

    suggestions.dedup();
    suggestions
}

// ============================================================================
// Skill Generation from Patterns
// ============================================================================

/// Generate a skill from a behavioral cluster.
pub fn generate_skill_from_cluster(
    cluster: &ClusteredBehavior,
    base_name: Option<&str>,
) -> EmpiricalSkillContent {
    let name = base_name
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("{}-workflow", cluster.inferred_category));

    let description = format!(
        "Empirically-derived skill for {} workflows, based on {} analyzed sessions.",
        cluster.inferred_category, cluster.size
    );

    // Build frontmatter
    let frontmatter = format!(
        r"---
name: {}
description: {}
triggers:
{}
category: {}
confidence: {:.2}
source: empirical-analysis
cluster_id: {}
---",
        name,
        description,
        cluster
            .success_patterns
            .iter()
            .flat_map(|p| &p.trigger_keywords)
            .take(5)
            .map(|k| format!("  - {}", k))
            .collect::<Vec<_>>()
            .join("\n"),
        cluster.inferred_category,
        cluster
            .success_patterns
            .first()
            .map(|p| p.confidence)
            .unwrap_or(0.5),
        cluster.cluster_id,
    );

    // Build body from success patterns
    let mut body = String::new();
    body.push_str(&format!("# {}\n\n", name));
    body.push_str(&format!("{}\n\n", description));

    body.push_str("## Recommended Workflow\n\n");
    body.push_str("Based on observed successful sessions, follow this sequence:\n\n");

    for (i, pattern) in cluster.success_patterns.iter().enumerate() {
        body.push_str(&format!(
            "### Step {}: {}\n",
            i + 1,
            pattern.action_sequence.join(" → ")
        ));
        body.push_str(&format!(
            "**Confidence:** {:.0}% ({} occurrences)\n\n",
            pattern.confidence * 100.0,
            pattern.occurrence_count
        ));

        if !pattern.example_contexts.is_empty() {
            body.push_str("**Example triggers:**\n");
            for ctx in &pattern.example_contexts {
                body.push_str(&format!(
                    "- \"{}\"\n",
                    ctx.chars().take(80).collect::<String>()
                ));
            }
            body.push('\n');
        }
    }

    if !cluster.common_file_types.is_empty() {
        body.push_str("## Common File Types\n\n");
        body.push_str("This workflow typically involves:\n");
        for ext in &cluster.common_file_types {
            body.push_str(&format!("- `.{}`\n", ext));
        }
        body.push('\n');
    }

    // Build guardrails from failure patterns
    let mut guardrails = String::new();
    if !cluster.failure_patterns.is_empty() {
        guardrails.push_str("## Guardrails (Avoid These Patterns)\n\n");
        guardrails.push_str("The following patterns have been observed to lead to failures:\n\n");

        for pattern in &cluster.failure_patterns {
            guardrails.push_str(&format!("### ⚠️ {}\n", pattern.action_sequence.join(" → ")));

            if !pattern.error_messages.is_empty() {
                guardrails.push_str("**Common errors:**\n");
                for err in &pattern.error_messages {
                    guardrails.push_str(&format!(
                        "- `{}`\n",
                        err.chars().take(100).collect::<String>()
                    ));
                }
            }

            if !pattern.recovery_suggestions.is_empty() {
                guardrails.push_str("\n**Recovery suggestions:**\n");
                for suggestion in &pattern.recovery_suggestions {
                    guardrails.push_str(&format!("- {}\n", suggestion));
                }
            }
            guardrails.push('\n');
        }
    }

    let confidence = cluster
        .success_patterns
        .iter()
        .map(|p| p.confidence)
        .sum::<f64>()
        / cluster.success_patterns.len().max(1) as f64;

    EmpiricalSkillContent {
        name,
        description,
        frontmatter,
        body,
        guardrails,
        confidence,
        source_cluster_id: cluster.cluster_id.clone(),
    }
}

/// Format empirical skill as SKILL.md content.
pub fn format_as_skill_md(skill: &EmpiricalSkillContent) -> String {
    let mut content = String::new();
    content.push_str(&skill.frontmatter);
    content.push_str("\n\n");
    content.push_str(&skill.body);
    if !skill.guardrails.is_empty() {
        content.push('\n');
        content.push_str(&skill.guardrails);
    }
    content
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::behavioral::{SkillUsageEventData, ToolStatus};

    fn make_test_event(
        session_id: &str,
        tools: Vec<&str>,
        outcome: OutcomeStatus,
    ) -> BehavioralEvent {
        BehavioralEvent {
            skill_usage: SkillUsageEventData {
                timestamp: 1000,
                skill_path: "test-skill".to_string(),
                session_id: session_id.to_string(),
                prompt_context: Some("test context".to_string()),
            },
            tool_sequence: tools
                .into_iter()
                .enumerate()
                .map(|(i, name)| ToolCall {
                    name: name.to_string(),
                    timestamp: 1000 + i as u64 * 100,
                    input_summary: "{}".to_string(),
                    status: ToolStatus::Success,
                })
                .collect(),
            files_accessed: vec![],
            session_outcome: Some(SessionOutcome {
                session_id: session_id.to_string(),
                status: outcome,
                confidence: 0.8,
                evidence: vec![],
                duration_seconds: 60,
            }),
        }
    }

    #[test]
    fn test_cluster_sessions_empty() {
        let result = cluster_sessions(&[], 5, 2);
        assert!(result.clusters.is_empty());
        assert_eq!(result.total_sessions, 0);
    }

    #[test]
    fn test_cluster_sessions_insufficient_data() {
        let events = vec![make_test_event(
            "s1",
            vec!["Read", "Write"],
            OutcomeStatus::Success,
        )];

        let result = cluster_sessions(&events, 5, 3);
        assert!(result.clusters.is_empty());
        assert_eq!(result.outlier_sessions.len(), 1);
    }

    #[test]
    fn test_infer_category_from_patterns() {
        assert_eq!(
            infer_category_from_patterns("Bash:py", &[]),
            "testing".to_string()
        );
        assert_eq!(
            infer_category_from_patterns("Read:rs", &[]),
            "rust-development".to_string()
        );
        assert_eq!(
            infer_category_from_patterns("Write:ts", &[]),
            "code-modification".to_string()
        );
    }

    #[test]
    fn test_generate_recovery_suggestions() {
        let errors = vec![
            "file not found".to_string(),
            "permission denied".to_string(),
        ];
        let suggestions = generate_recovery_suggestions(&errors);

        assert!(suggestions.iter().any(|s| s.contains("file path")));
        assert!(suggestions.iter().any(|s| s.contains("permission")));
    }

    #[test]
    fn test_generate_skill_from_cluster() {
        let cluster = ClusteredBehavior {
            cluster_id: "test-cluster".to_string(),
            label: "Read:rs".to_string(),
            session_ids: vec!["s1".to_string(), "s2".to_string()],
            success_patterns: vec![SuccessPattern {
                action_sequence: vec!["Read".to_string(), "Edit".to_string()],
                trigger_keywords: vec!["fix".to_string(), "update".to_string()],
                confidence: 0.8,
                occurrence_count: 5,
                example_contexts: vec!["Fix the bug".to_string()],
            }],
            failure_patterns: vec![],
            common_file_types: vec!["rs".to_string()],
            inferred_category: "rust-development".to_string(),
            size: 2,
        };

        let skill = generate_skill_from_cluster(&cluster, None);

        assert!(skill.name.contains("rust-development"));
        assert!(skill.body.contains("Recommended Workflow"));
        assert!(skill.body.contains("Read → Edit"));
        assert!(skill.confidence > 0.0);
    }

    #[test]
    fn test_format_as_skill_md() {
        let skill = EmpiricalSkillContent {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            frontmatter: "---\nname: test-skill\n---".to_string(),
            body: "# Test Skill\n\nContent here.".to_string(),
            guardrails: "## Guardrails\n\nWarning here.".to_string(),
            confidence: 0.9,
            source_cluster_id: "cluster-1".to_string(),
        };

        let md = format_as_skill_md(&skill);

        assert!(md.contains("---\nname: test-skill"));
        assert!(md.contains("# Test Skill"));
        assert!(md.contains("## Guardrails"));
    }
}
