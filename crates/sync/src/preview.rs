//! Interactive sync preview: collects pending changes, computes diffs, and lets
//! the user accept or reject each file before writing.

use similar::TextDiff;

/// The kind of artifact being synced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Command,
    Skill,
    McpServer,
    Preference,
    Agent,
    Hook,
    Instruction,
    PluginAsset,
}

impl std::fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Command => write!(f, "command"),
            Self::Skill => write!(f, "skill"),
            Self::McpServer => write!(f, "mcp-server"),
            Self::Preference => write!(f, "preference"),
            Self::Agent => write!(f, "agent"),
            Self::Hook => write!(f, "hook"),
            Self::Instruction => write!(f, "instruction"),
            Self::PluginAsset => write!(f, "plugin-asset"),
        }
    }
}

/// A single pending change that has not yet been written.
#[derive(Debug, Clone)]
pub struct PendingChange {
    /// What type of artifact this is.
    pub kind: ArtifactKind,
    /// Human-readable name (e.g., command name, skill name, server key).
    pub name: String,
    /// Unified-diff text between old and new content.
    pub diff_text: String,
    /// True when the file is entirely new (no previous content).
    pub is_new: bool,
}

/// A collection of pending changes to be reviewed.
#[derive(Debug, Default)]
pub struct ChangeSet {
    pub changes: Vec<PendingChange>,
}

impl ChangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a pending change to the set.
    pub fn push(&mut self, change: PendingChange) {
        self.changes.push(change);
    }

    /// Returns true when there are no pending changes.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Number of pending changes.
    pub fn len(&self) -> usize {
        self.changes.len()
    }
}

/// Computes a unified diff between two UTF-8 byte slices.
///
/// Returns a human-readable unified-diff string. If both slices are
/// identical the diff will be empty.
pub fn compute_diff(old: &[u8], new: &[u8]) -> String {
    let old_str = String::from_utf8_lossy(old);
    let new_str = String::from_utf8_lossy(new);

    let diff = TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());
    diff.unified_diff()
        .context_radius(3)
        .header("old", "new")
        .to_string()
}

/// Displays each pending change and asks the user to accept or reject it.
///
/// Returns a `Vec<bool>` parallel to `changes` where `true` means accepted.
///
/// This function writes to stdout and reads from stdin via `inquire`.
pub fn preview_changes(changes: &[PendingChange]) -> Vec<bool> {
    let total = changes.len();
    let mut decisions = Vec::with_capacity(total);

    for (i, change) in changes.iter().enumerate() {
        println!(
            "\n--- [{}/{}] {} ({}) ---",
            i + 1,
            total,
            change.name,
            change.kind
        );

        if change.is_new {
            println!("  (new file)");
        }

        if change.diff_text.is_empty() {
            println!("  (no content change)");
        } else {
            println!("{}", change.diff_text);
        }

        let accepted = inquire::Confirm::new(&format!("Accept {} '{}'?", change.kind, change.name))
            .with_default(true)
            .prompt()
            .unwrap_or(false);

        decisions.push(accepted);
    }

    decisions
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // compute_diff tests
    // ==========================================

    #[test]
    fn given_identical_content_when_compute_diff_then_empty() {
        let content = b"hello world\n";
        let diff = compute_diff(content, content);
        // identical content produces no hunks
        assert!(
            !diff.contains("@@"),
            "identical content should produce no diff hunks"
        );
    }

    #[test]
    fn given_added_lines_when_compute_diff_then_shows_plus() {
        let old = b"line one\n";
        let new = b"line one\nline two\n";
        let diff = compute_diff(old, new);
        assert!(diff.contains("+line two"), "diff should show added line");
    }

    #[test]
    fn given_removed_lines_when_compute_diff_then_shows_minus() {
        let old = b"line one\nline two\n";
        let new = b"line one\n";
        let diff = compute_diff(old, new);
        assert!(diff.contains("-line two"), "diff should show removed line");
    }

    #[test]
    fn given_changed_lines_when_compute_diff_then_shows_both() {
        let old = b"alpha\n";
        let new = b"beta\n";
        let diff = compute_diff(old, new);
        assert!(diff.contains("-alpha"), "diff should show removed old line");
        assert!(diff.contains("+beta"), "diff should show added new line");
    }

    #[test]
    fn given_empty_old_when_compute_diff_then_shows_all_added() {
        let old = b"";
        let new = b"brand new content\n";
        let diff = compute_diff(old, new);
        assert!(
            diff.contains("+brand new content"),
            "diff should show all lines as added"
        );
    }

    #[test]
    fn given_non_utf8_content_when_compute_diff_then_does_not_panic() {
        let old: &[u8] = &[0xff, 0xfe, 0x0a];
        let new: &[u8] = &[0xfe, 0xff, 0x0a];
        // Should not panic — lossy conversion handles invalid UTF-8
        let _diff = compute_diff(old, new);
    }

    // ==========================================
    // ChangeSet tests
    // ==========================================

    #[test]
    fn given_new_changeset_when_created_then_empty() {
        let cs = ChangeSet::new();
        assert!(cs.is_empty());
        assert_eq!(cs.len(), 0);
    }

    #[test]
    fn given_changeset_when_push_then_len_increases() {
        let mut cs = ChangeSet::new();
        cs.push(PendingChange {
            kind: ArtifactKind::Command,
            name: "hello".into(),
            diff_text: String::new(),
            is_new: true,
        });
        assert_eq!(cs.len(), 1);
        assert!(!cs.is_empty());
    }

    // ==========================================
    // ArtifactKind Display tests
    // ==========================================

    #[test]
    fn artifact_kind_display_covers_all_variants() {
        assert_eq!(format!("{}", ArtifactKind::Command), "command");
        assert_eq!(format!("{}", ArtifactKind::Skill), "skill");
        assert_eq!(format!("{}", ArtifactKind::McpServer), "mcp-server");
        assert_eq!(format!("{}", ArtifactKind::Preference), "preference");
        assert_eq!(format!("{}", ArtifactKind::Agent), "agent");
        assert_eq!(format!("{}", ArtifactKind::Hook), "hook");
        assert_eq!(format!("{}", ArtifactKind::Instruction), "instruction");
        assert_eq!(format!("{}", ArtifactKind::PluginAsset), "plugin-asset");
    }
}
