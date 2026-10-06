//! Platform-routing helpers for sync operations.

/// Returns the default target platform for a given source.
///
/// Used when a sync tool needs to infer the target from only the source name.
#[must_use]
pub fn default_target_for(from: &str) -> &'static str {
    match from {
        "claude" => "codex",
        "codex" => "claude",
        "copilot" => "claude",
        "cursor" => "claude",
        _ => "codex",
    }
}

/// How one sync run delivers skills to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillDelivery {
    /// Run the orchestrator's skill sync.
    pub sync_skills: bool,
    /// Ask the source for its complete plugin tree (manifests and skill bodies
    /// included) instead of only the files no other artifact type covers.
    pub full_plugin_mirror: bool,
}

/// Decides how skills reach `to` from `from`.
///
/// Cursor discovers installed plugins under `plugins/local/<plugin>/`, so a
/// Cursor target takes a full mirror. The orchestrator then drops plugin skills
/// from the flat copy on its own, because only it knows which skills carry a
/// `plugin_origin` and whether the source can read the plugin cache at all.
/// Skills with no plugin origin are absent from the mirror, so the flat sync
/// stays on for every target.
///
/// Claude to Codex is the one direction that skips the orchestrator's skill
/// sync: `sync_skills_only_from_claude` has already copied those skills into
/// Codex's own discovery root.
///
/// ```
/// use skrills_sync::platform_routing::skill_delivery;
///
/// let cursor = skill_delivery("codex", "cursor");
/// assert!(cursor.sync_skills);
/// assert!(cursor.full_plugin_mirror);
///
/// let codex = skill_delivery("claude", "codex");
/// assert!(!codex.sync_skills);
/// assert!(!codex.full_plugin_mirror);
/// ```
#[must_use]
pub fn skill_delivery(from: &str, to: &str) -> SkillDelivery {
    SkillDelivery {
        sync_skills: !(from == "claude" && to == "codex"),
        full_plugin_mirror: to == "cursor",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The duplicate-write bug: a Cursor target that also ran the flat skill
    /// sync wrote every plugin skill twice, and turning the flat sync off
    /// entirely dropped the skills of a source with no plugin cache.
    #[test]
    fn skill_delivery_keeps_flat_sync_on_for_cursor_and_asks_for_the_full_mirror() {
        for from in ["claude", "codex", "copilot"] {
            let delivery = skill_delivery(from, "cursor");
            assert!(
                delivery.sync_skills,
                "{from} to cursor must still sync skills with no plugin origin"
            );
            assert!(
                delivery.full_plugin_mirror,
                "{from} to cursor needs the complete plugin tree"
            );
        }
    }

    #[test]
    fn skill_delivery_leaves_claude_to_codex_skills_to_the_dedicated_copier() {
        let delivery = skill_delivery("claude", "codex");
        assert!(!delivery.sync_skills);
        // Codex reads no plugin tree, so there is nothing to mirror.
        assert!(!delivery.full_plugin_mirror);
    }

    #[test]
    fn skill_delivery_syncs_skills_for_every_other_direction() {
        for (from, to) in [
            ("codex", "claude"),
            ("claude", "copilot"),
            ("copilot", "claude"),
            ("cursor", "claude"),
        ] {
            let delivery = skill_delivery(from, to);
            assert!(delivery.sync_skills, "{from} to {to} should sync skills");
            assert!(!delivery.full_plugin_mirror);
        }
    }

    #[test]
    fn default_target_for_all_platforms() {
        assert_eq!(default_target_for("claude"), "codex");
        assert_eq!(default_target_for("codex"), "claude");
        assert_eq!(default_target_for("copilot"), "claude");
        assert_eq!(default_target_for("cursor"), "claude");
        assert_eq!(default_target_for("unknown"), "codex");
    }
}
