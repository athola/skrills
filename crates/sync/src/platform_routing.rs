//! Platform-routing helpers for sync operations.

use crate::Platform;

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
/// ```
/// use skrills_sync::platform_routing::skill_delivery;
/// use skrills_sync::Platform;
///
/// let cursor = skill_delivery(Platform::Codex, Platform::Cursor);
/// assert!(cursor.sync_skills);
/// assert!(cursor.full_plugin_mirror);
///
/// let codex = skill_delivery(Platform::Claude, Platform::Codex);
/// assert!(codex.sync_skills);
/// assert!(!codex.full_plugin_mirror);
/// ```
#[must_use]
pub fn skill_delivery(_from: Platform, to: Platform) -> SkillDelivery {
    SkillDelivery {
        sync_skills: true,
        full_plugin_mirror: to == Platform::Cursor,
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
        for from in [Platform::Claude, Platform::Codex, Platform::Copilot] {
            let delivery = skill_delivery(from, Platform::Cursor);
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

    /// The dedicated copier this direction used to rely on wrote
    /// `~/.codex/skills`, which current Codex does not load; only the
    /// orchestrator's skill phase reaches `~/.agents/skills`.
    #[test]
    fn skill_delivery_runs_the_skill_phase_for_claude_to_codex() {
        let delivery = skill_delivery(Platform::Claude, Platform::Codex);
        assert!(delivery.sync_skills);
        // Codex reads no plugin tree, so there is nothing to mirror.
        assert!(!delivery.full_plugin_mirror);
    }

    #[test]
    fn skill_delivery_syncs_skills_for_every_other_direction() {
        for (from, to) in [
            (Platform::Codex, Platform::Claude),
            (Platform::Claude, Platform::Copilot),
            (Platform::Copilot, Platform::Claude),
            (Platform::Cursor, Platform::Claude),
        ] {
            let delivery = skill_delivery(from, to);
            assert!(delivery.sync_skills, "{from} to {to} should sync skills");
            assert!(!delivery.full_plugin_mirror);
        }
    }
}
