//! Conflict detection, resolution types, and side-by-side diff formatting.
//!
//! When both the source and target have modified an artifact since the last sync,
//! a conflict is raised. This module provides the types for representing conflicts,
//! detecting them via 3-way hash comparison, formatting diffs for display, and
//! prompting users for resolution.

use crate::common::Command;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::fmt;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of change detected during conflict analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Only the source changed since last sync.
    SourceChanged,
    /// Only the target changed since last sync.
    TargetChanged,
    /// Both source and target changed since last sync (true conflict).
    BothChanged,
}

impl fmt::Display for ConflictKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceChanged => write!(f, "source changed"),
            Self::TargetChanged => write!(f, "target changed"),
            Self::BothChanged => write!(f, "both changed"),
        }
    }
}

/// The type of artifact involved in the conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    /// Slash commands.
    Command,
    /// Skills.
    Skill,
    /// Hooks.
    Hook,
    /// Agents.
    Agent,
    /// Instructions (CLAUDE.md, etc.).
    Instruction,
}

impl fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command => write!(f, "command"),
            Self::Skill => write!(f, "skill"),
            Self::Hook => write!(f, "hook"),
            Self::Agent => write!(f, "agent"),
            Self::Instruction => write!(f, "instruction"),
        }
    }
}

impl ArtifactType {
    /// Returns the string representation used in storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Skill => "skill",
            Self::Hook => "hook",
            Self::Agent => "agent",
            Self::Instruction => "instruction",
        }
    }

    /// Parse from a stored string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "command" => Some(Self::Command),
            "skill" => Some(Self::Skill),
            "hook" => Some(Self::Hook),
            "agent" => Some(Self::Agent),
            "instruction" => Some(Self::Instruction),
            _ => None,
        }
    }
}

/// A detected conflict between source and target versions of an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    /// What type of artifact is conflicting.
    pub artifact_type: ArtifactType,
    /// Name/identifier of the artifact.
    pub name: String,
    /// Content from the source side (UTF-8 lossy).
    pub source_content: String,
    /// Content from the target side (UTF-8 lossy).
    pub target_content: String,
    /// SHA-256 hash of source content.
    pub source_hash: String,
    /// SHA-256 hash of target content.
    pub target_hash: String,
    /// What kind of conflict this is.
    pub kind: ConflictKind,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} '{}' ({})",
            self.artifact_type, self.name, self.kind
        )
    }
}

/// How a conflict should be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Keep the source version (overwrite target).
    KeepSource,
    /// Keep the target version (skip this artifact).
    KeepTarget,
    /// Skip this artifact entirely (no changes).
    Skip,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeepSource => write!(f, "keep source"),
            Self::KeepTarget => write!(f, "keep target"),
            Self::Skip => write!(f, "skip"),
        }
    }
}

impl Resolution {
    /// Returns the string representation used in storage/metrics.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::KeepSource => "keep_source",
            Self::KeepTarget => "keep_target",
            Self::Skip => "skip",
        }
    }
}

/// A conflict paired with its chosen resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedConflict {
    /// The original conflict.
    pub conflict: Conflict,
    /// The chosen resolution.
    pub resolution: Resolution,
}

// ─────────────────────────────────────────────────────────────────────────────
// Detection
// ─────────────────────────────────────────────────────────────────────────────

/// Result of comparing a single item against its baseline.
enum ItemStatus {
    /// No conflict — item can be synced normally.
    NoConflict,
    /// A conflict was detected.
    Conflict(Conflict),
}

/// Detects conflicts between source and target items using baseline hashes.
///
/// For each source item that also exists on the target, compares both against
/// the stored baseline hash to determine if one or both sides changed.
///
/// Returns only items where `ConflictKind::BothChanged` — the true conflicts
/// that require user resolution. `SourceChanged` items can be synced normally,
/// and `TargetChanged` items are skipped.
///
/// # Arguments
/// * `source_items` — items read from the source adapter
/// * `target_items` — items read from the target adapter
/// * `artifact_type` — what kind of artifact these are
/// * `get_baseline_hash` — closure that returns the last-synced hash for a name
pub fn detect_conflicts<F>(
    source_items: &[Command],
    target_items: &[Command],
    artifact_type: ArtifactType,
    get_baseline_hash: F,
) -> Vec<Conflict>
where
    F: Fn(&str) -> Option<String>,
{
    let target_map: std::collections::HashMap<&str, &Command> =
        target_items.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut conflicts = Vec::new();

    for source in source_items {
        let Some(target) = target_map.get(source.name.as_str()) else {
            // New item — no conflict possible.
            continue;
        };

        // Same hash on both sides — no conflict.
        if source.hash == target.hash {
            continue;
        }

        // Check against baseline to classify the change.
        match classify_change(source, target, artifact_type, &get_baseline_hash) {
            ItemStatus::Conflict(c) => conflicts.push(c),
            ItemStatus::NoConflict => {}
        }
    }

    conflicts
}

/// Classifies a change by comparing source/target hashes against baseline.
fn classify_change<F>(
    source: &Command,
    target: &Command,
    artifact_type: ArtifactType,
    get_baseline_hash: &F,
) -> ItemStatus
where
    F: Fn(&str) -> Option<String>,
{
    let baseline_hash = get_baseline_hash(&source.name);

    let source_content = String::from_utf8_lossy(&source.content).into_owned();
    let target_content = String::from_utf8_lossy(&target.content).into_owned();

    let kind = match baseline_hash {
        Some(ref baseline) => {
            let source_changed = source.hash != *baseline;
            let target_changed = target.hash != *baseline;

            match (source_changed, target_changed) {
                (true, true) => ConflictKind::BothChanged,
                (true, false) => ConflictKind::SourceChanged,
                (false, true) => ConflictKind::TargetChanged,
                (false, false) => {
                    // Neither changed from baseline but hashes differ from each
                    // other — this shouldn't happen with consistent hashing, but
                    // treat as BothChanged to be safe.
                    ConflictKind::BothChanged
                }
            }
        }
        None => {
            // No baseline — both sides have the item with different content.
            // Since we can't tell who changed, treat as BothChanged.
            ConflictKind::BothChanged
        }
    };

    // Only return true conflicts (BothChanged) — the others are handled normally.
    match kind {
        ConflictKind::BothChanged => ItemStatus::Conflict(Conflict {
            artifact_type,
            name: source.name.clone(),
            source_content,
            target_content,
            source_hash: source.hash.clone(),
            target_hash: target.hash.clone(),
            kind,
        }),
        ConflictKind::SourceChanged | ConflictKind::TargetChanged => ItemStatus::NoConflict,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Diff formatting
// ─────────────────────────────────────────────────────────────────────────────

/// Formats a unified diff between two strings.
///
/// Uses the `similar` crate to compute the diff and renders it in a
/// familiar unified diff format with `+`/`-` markers.
pub fn format_unified_diff(left: &str, right: &str, left_label: &str, right_label: &str) -> String {
    let diff = TextDiff::from_lines(left, right);
    let mut output = String::new();

    output.push_str(&format!("--- {left_label}\n"));
    output.push_str(&format!("+++ {right_label}\n"));

    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&format!("{hunk}"));
    }

    output
}

/// Formats a side-by-side diff of two strings.
///
/// Each line is shown with source on the left and target on the right,
/// separated by a gutter character that indicates the change type:
/// - `|` means the line differs between sides
/// - `<` means the line only exists on the left (source)
/// - `>` means the line only exists on the right (target)
/// - ` ` means the line is identical on both sides
///
/// Falls back to unified diff if `width` is less than 60 columns.
pub fn format_side_by_side(left: &str, right: &str, width: usize) -> String {
    // Minimum reasonable width for side-by-side
    if width < 60 {
        return format_unified_diff(left, right, "source", "target");
    }

    let diff = TextDiff::from_lines(left, right);

    // Each side gets half the width minus the gutter (3 chars: " | ")
    let side_width = (width - 3) / 2;
    let mut output = String::new();

    // Header
    let src_header = format!("{:width$}", "SOURCE", width = side_width);
    let tgt_header = format!("{:width$}", "TARGET", width = side_width);
    output.push_str(&format!("{src_header} | {tgt_header}\n"));
    output.push_str(&format!(
        "{} + {}\n",
        "-".repeat(side_width),
        "-".repeat(side_width)
    ));

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let line = change.value().trim_end_matches('\n').trim_end_matches('\r');
            match change.tag() {
                ChangeTag::Equal => {
                    let left_text = truncate_or_pad(line, side_width);
                    let right_text = truncate_or_pad(line, side_width);
                    output.push_str(&format!("{left_text}   {right_text}\n"));
                }
                ChangeTag::Delete => {
                    let left_text = truncate_or_pad(line, side_width);
                    let right_text = " ".repeat(side_width);
                    output.push_str(&format!("{left_text} < {right_text}\n"));
                }
                ChangeTag::Insert => {
                    let left_text = " ".repeat(side_width);
                    let right_text = truncate_or_pad(line, side_width);
                    output.push_str(&format!("{left_text} > {right_text}\n"));
                }
            }
        }
    }

    output
}

/// Truncates or pads a string to exactly `width` display characters.
fn truncate_or_pad(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= width {
        format!("{s:width$}", width = width)
    } else {
        // Truncate and add ellipsis
        let truncated: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{truncated}~")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactive resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Strategy for how to handle conflicts during sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    /// Prompt the user interactively for each conflict.
    Prompt,
    /// Automatically keep the source version for all conflicts.
    ForceSource,
    /// Automatically keep the target version for all conflicts.
    ForceTarget,
    /// Skip all conflicting artifacts.
    SkipAll,
}

impl Default for ConflictStrategy {
    fn default() -> Self {
        Self::Prompt
    }
}

/// Resolves a list of conflicts according to the given strategy.
///
/// For `ConflictStrategy::Prompt`, calls `prompt_fn` for each conflict.
/// For automatic strategies, applies the same resolution to all.
///
/// Returns the list of resolved conflicts.
pub fn resolve_conflicts<F>(
    conflicts: Vec<Conflict>,
    strategy: ConflictStrategy,
    mut prompt_fn: F,
) -> Vec<ResolvedConflict>
where
    F: FnMut(&Conflict) -> Resolution,
{
    conflicts
        .into_iter()
        .map(|conflict| {
            let resolution = match strategy {
                ConflictStrategy::Prompt => prompt_fn(&conflict),
                ConflictStrategy::ForceSource => Resolution::KeepSource,
                ConflictStrategy::ForceTarget => Resolution::KeepTarget,
                ConflictStrategy::SkipAll => Resolution::Skip,
            };
            ResolvedConflict {
                conflict,
                resolution,
            }
        })
        .collect()
}

/// Default interactive prompt for conflict resolution.
///
/// Shows a side-by-side diff and asks the user to choose a resolution.
/// Returns `Resolution::Skip` if the prompt fails (e.g., non-interactive terminal).
pub fn prompt_conflict_resolution(conflict: &Conflict) -> Resolution {
    // Try to get terminal width, default to 80
    let width = terminal_width().unwrap_or(80);

    eprintln!();
    eprintln!(
        "=== CONFLICT: {} '{}' ===",
        conflict.artifact_type, conflict.name
    );
    eprintln!("Both source and target have been modified since last sync.");
    eprintln!();
    eprintln!("{}", format_side_by_side(&conflict.source_content, &conflict.target_content, width));

    let options = vec![
        "Keep source (overwrite target)",
        "Keep target (skip this artifact)",
        "Skip (no changes)",
    ];

    match inquire::Select::new("How should this conflict be resolved?", options).prompt() {
        Ok(choice) => {
            if choice.starts_with("Keep source") {
                Resolution::KeepSource
            } else if choice.starts_with("Keep target") {
                Resolution::KeepTarget
            } else {
                Resolution::Skip
            }
        }
        Err(_) => {
            eprintln!("  (non-interactive terminal, skipping conflict)");
            Resolution::Skip
        }
    }
}

/// Attempts to detect the terminal width.
///
/// Returns `None` if detection fails.
fn terminal_width() -> Option<usize> {
    // Use a simple ioctl-based approach on Unix
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        unsafe {
            let mut winsize = MaybeUninit::<libc::winsize>::uninit();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, winsize.as_mut_ptr()) == 0 {
                let ws = winsize.assume_init();
                if ws.ws_col > 0 {
                    return Some(ws.ws_col as usize);
                }
            }
        }
        None
    }

    #[cfg(not(unix))]
    {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::utils::test_helpers::make_command;

    // ==========================================
    // ConflictKind tests
    // ==========================================

    #[test]
    fn conflict_kind_display() {
        assert_eq!(ConflictKind::SourceChanged.to_string(), "source changed");
        assert_eq!(ConflictKind::TargetChanged.to_string(), "target changed");
        assert_eq!(ConflictKind::BothChanged.to_string(), "both changed");
    }

    // ==========================================
    // ArtifactType tests
    // ==========================================

    #[test]
    fn artifact_type_roundtrip() {
        for at in [
            ArtifactType::Command,
            ArtifactType::Skill,
            ArtifactType::Hook,
            ArtifactType::Agent,
            ArtifactType::Instruction,
        ] {
            let s = at.as_str();
            let parsed = ArtifactType::from_str(s).unwrap();
            assert_eq!(parsed, at);
        }
    }

    #[test]
    fn artifact_type_from_str_unknown() {
        assert!(ArtifactType::from_str("unknown").is_none());
    }

    // ==========================================
    // Resolution tests
    // ==========================================

    #[test]
    fn resolution_display() {
        assert_eq!(Resolution::KeepSource.to_string(), "keep source");
        assert_eq!(Resolution::KeepTarget.to_string(), "keep target");
        assert_eq!(Resolution::Skip.to_string(), "skip");
    }

    #[test]
    fn resolution_as_str() {
        assert_eq!(Resolution::KeepSource.as_str(), "keep_source");
        assert_eq!(Resolution::KeepTarget.as_str(), "keep_target");
        assert_eq!(Resolution::Skip.as_str(), "skip");
    }

    // ==========================================
    // Conflict detection tests
    // ==========================================

    #[test]
    fn detect_no_conflicts_when_hashes_match() {
        let source = vec![make_command("skill-a", "content A")];
        let target = vec![make_command("skill-a", "content A")];

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |_| None);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn detect_no_conflicts_for_new_items() {
        let source = vec![make_command("skill-a", "content A")];
        let target: Vec<Command> = vec![];

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |_| None);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn detect_both_changed_with_no_baseline() {
        let source = vec![make_command("skill-a", "source version")];
        let target = vec![make_command("skill-a", "target version")];

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |_| None);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::BothChanged);
        assert_eq!(conflicts[0].name, "skill-a");
    }

    #[test]
    fn detect_both_changed_with_baseline() {
        let source = vec![make_command("skill-a", "new source")];
        let target = vec![make_command("skill-a", "new target")];

        // Baseline hash is different from both
        let baseline_hash =
            crate::adapters::utils::hash_content(b"original content");

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |name| {
            if name == "skill-a" {
                Some(baseline_hash.clone())
            } else {
                None
            }
        });

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::BothChanged);
    }

    #[test]
    fn detect_source_only_changed_not_conflict() {
        let source = vec![make_command("skill-a", "new source")];
        let target = vec![make_command("skill-a", "original content")];

        // Baseline matches target — only source changed
        let baseline_hash =
            crate::adapters::utils::hash_content(b"original content");

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |name| {
            if name == "skill-a" {
                Some(baseline_hash.clone())
            } else {
                None
            }
        });

        // SourceChanged is not a true conflict — it should be synced normally
        assert!(conflicts.is_empty());
    }

    #[test]
    fn detect_target_only_changed_not_conflict() {
        let source = vec![make_command("skill-a", "original content")];
        let target = vec![make_command("skill-a", "new target")];

        // Baseline matches source — only target changed
        let baseline_hash =
            crate::adapters::utils::hash_content(b"original content");

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Skill, |name| {
            if name == "skill-a" {
                Some(baseline_hash.clone())
            } else {
                None
            }
        });

        // TargetChanged is not a true conflict — target version should be preserved
        assert!(conflicts.is_empty());
    }

    #[test]
    fn detect_multiple_conflicts() {
        let source = vec![
            make_command("a", "source-a"),
            make_command("b", "source-b"),
            make_command("c", "same-content"),
        ];
        let target = vec![
            make_command("a", "target-a"),
            make_command("b", "target-b"),
            make_command("c", "same-content"),
        ];

        let conflicts = detect_conflicts(&source, &target, ArtifactType::Command, |_| None);
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts.iter().any(|c| c.name == "a"));
        assert!(conflicts.iter().any(|c| c.name == "b"));
    }

    // ==========================================
    // Diff formatting tests
    // ==========================================

    #[test]
    fn unified_diff_shows_changes() {
        let left = "line 1\nline 2\nline 3\n";
        let right = "line 1\nline 2 modified\nline 3\n";

        let diff = format_unified_diff(left, right, "source", "target");
        assert!(diff.contains("--- source"));
        assert!(diff.contains("+++ target"));
        assert!(diff.contains("-line 2"));
        assert!(diff.contains("+line 2 modified"));
    }

    #[test]
    fn unified_diff_identical_content() {
        let content = "line 1\nline 2\n";
        let diff = format_unified_diff(content, content, "a", "b");
        assert!(diff.contains("--- a"));
        assert!(diff.contains("+++ b"));
        // No change hunks for identical content
        assert!(!diff.contains('-'));
    }

    #[test]
    fn side_by_side_falls_back_to_unified_for_narrow_terminal() {
        let left = "line 1\n";
        let right = "line 2\n";

        let result = format_side_by_side(left, right, 50);
        // Should fall back to unified format
        assert!(result.contains("---"));
        assert!(result.contains("+++"));
    }

    #[test]
    fn side_by_side_shows_both_columns() {
        let left = "line 1\nline 2\n";
        let right = "line 1\nline 2 mod\n";

        let result = format_side_by_side(left, right, 80);
        assert!(result.contains("SOURCE"));
        assert!(result.contains("TARGET"));
    }

    #[test]
    fn side_by_side_equal_content() {
        let content = "same line\n";
        let result = format_side_by_side(content, content, 80);
        assert!(result.contains("SOURCE"));
        assert!(result.contains("TARGET"));
        // Should show the line on both sides
        assert!(result.contains("same line"));
    }

    #[test]
    fn truncate_or_pad_short_string() {
        let result = truncate_or_pad("hi", 10);
        assert_eq!(result.len(), 10);
        assert!(result.starts_with("hi"));
    }

    #[test]
    fn truncate_or_pad_long_string() {
        let result = truncate_or_pad("a very long string here", 10);
        assert_eq!(result.chars().count(), 10);
        assert!(result.ends_with('~'));
    }

    #[test]
    fn truncate_or_pad_exact_length() {
        let result = truncate_or_pad("exact", 5);
        assert_eq!(result, "exact");
    }

    // ==========================================
    // Resolution strategy tests
    // ==========================================

    #[test]
    fn resolve_conflicts_force_source() {
        let conflicts = vec![Conflict {
            artifact_type: ArtifactType::Skill,
            name: "test".to_string(),
            source_content: "src".to_string(),
            target_content: "tgt".to_string(),
            source_hash: "aaa".to_string(),
            target_hash: "bbb".to_string(),
            kind: ConflictKind::BothChanged,
        }];

        let resolved = resolve_conflicts(conflicts, ConflictStrategy::ForceSource, |_| {
            panic!("prompt_fn should not be called for ForceSource")
        });

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].resolution, Resolution::KeepSource);
    }

    #[test]
    fn resolve_conflicts_force_target() {
        let conflicts = vec![Conflict {
            artifact_type: ArtifactType::Skill,
            name: "test".to_string(),
            source_content: "src".to_string(),
            target_content: "tgt".to_string(),
            source_hash: "aaa".to_string(),
            target_hash: "bbb".to_string(),
            kind: ConflictKind::BothChanged,
        }];

        let resolved = resolve_conflicts(conflicts, ConflictStrategy::ForceTarget, |_| {
            panic!("prompt_fn should not be called for ForceTarget")
        });

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].resolution, Resolution::KeepTarget);
    }

    #[test]
    fn resolve_conflicts_skip_all() {
        let conflicts = vec![
            Conflict {
                artifact_type: ArtifactType::Command,
                name: "a".to_string(),
                source_content: String::new(),
                target_content: String::new(),
                source_hash: "aaa".to_string(),
                target_hash: "bbb".to_string(),
                kind: ConflictKind::BothChanged,
            },
            Conflict {
                artifact_type: ArtifactType::Command,
                name: "b".to_string(),
                source_content: String::new(),
                target_content: String::new(),
                source_hash: "ccc".to_string(),
                target_hash: "ddd".to_string(),
                kind: ConflictKind::BothChanged,
            },
        ];

        let resolved = resolve_conflicts(conflicts, ConflictStrategy::SkipAll, |_| {
            panic!("prompt_fn should not be called for SkipAll")
        });

        assert_eq!(resolved.len(), 2);
        assert!(resolved.iter().all(|r| r.resolution == Resolution::Skip));
    }

    #[test]
    fn resolve_conflicts_prompt_calls_fn() {
        let conflicts = vec![Conflict {
            artifact_type: ArtifactType::Skill,
            name: "my-skill".to_string(),
            source_content: "src".to_string(),
            target_content: "tgt".to_string(),
            source_hash: "aaa".to_string(),
            target_hash: "bbb".to_string(),
            kind: ConflictKind::BothChanged,
        }];

        let resolved = resolve_conflicts(conflicts, ConflictStrategy::Prompt, |c| {
            assert_eq!(c.name, "my-skill");
            Resolution::KeepSource
        });

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].resolution, Resolution::KeepSource);
    }

    // ==========================================
    // Serialization tests
    // ==========================================

    #[test]
    fn conflict_kind_serialization_roundtrip() {
        for kind in [
            ConflictKind::SourceChanged,
            ConflictKind::TargetChanged,
            ConflictKind::BothChanged,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let restored: ConflictKind = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, kind);
        }
    }

    #[test]
    fn resolution_serialization_roundtrip() {
        for res in [
            Resolution::KeepSource,
            Resolution::KeepTarget,
            Resolution::Skip,
        ] {
            let json = serde_json::to_string(&res).unwrap();
            let restored: Resolution = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, res);
        }
    }

    #[test]
    fn conflict_strategy_default_is_prompt() {
        assert_eq!(ConflictStrategy::default(), ConflictStrategy::Prompt);
    }
}
