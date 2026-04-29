"""BDD tests for scripts/update_plugin_registrations.py.

The night-market original imports Phase 2-4 modules
(performance, meta-eval, knowledge queue) that are not relevant
in this Rust crate. The skrills port is Phase 1 only:
disk-vs-manifest registration audit with optional --fix.
"""

from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest

import update_plugin_registrations as upr  # type: ignore[import-not-found]  # pyright: ignore[reportMissingImports]


def _make_plugin(
    plugins_root: Path,
    name: str,
    manifest: dict,
    files: dict[str, str] | None = None,
) -> Path:
    plugin_dir = plugins_root / name
    (plugin_dir / ".claude-plugin").mkdir(parents=True)
    (plugin_dir / ".claude-plugin" / "plugin.json").write_text(
        json.dumps(manifest, indent=2)
    )
    for rel, content in (files or {}).items():
        target = plugin_dir / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    return plugin_dir


@pytest.fixture()
def plugins_root(tmp_path: Path) -> Path:
    root = tmp_path / "plugins"
    root.mkdir()
    return root


class TestCleanPluginNoIssues:
    """
    Feature: clean plugin produces zero issues

    As a maintainer
    I want the auditor to be silent when manifest matches disk
    So that running it on healthy plugins is a no-op.
    """

    @pytest.mark.unit
    def test_matching_manifest_returns_zero(self, plugins_root: Path):
        """
        Scenario: manifest lists exactly the on-disk commands
        Given a plugin with one command on disk and in manifest
        When I run audit_all
        Then issues_found is 0
        """
        _make_plugin(
            plugins_root,
            "skrills",
            manifest={
                "name": "skrills",
                "version": "0.0.0",
                "commands": ["./commands/foo.md"],
            },
            files={"commands/foo.md": "# foo"},
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=True)
        issues = auditor.audit_all()
        assert issues == 0


class TestMissingRegistration:
    """
    Feature: detect commands present on disk but absent from plugin.json

    As a maintainer
    I want missing registrations flagged
    So that newly-added commands cannot ship unregistered.
    """

    @pytest.mark.unit
    def test_missing_command_is_reported(self, plugins_root: Path):
        """
        Scenario: command file exists on disk but not in manifest
        Given a plugin with two .md files but only one in manifest
        When I run the audit
        Then 'missing' contains the unlisted command
        """
        _make_plugin(
            plugins_root,
            "skrills",
            manifest={
                "name": "skrills",
                "version": "0.0.0",
                "commands": ["./commands/foo.md"],
            },
            files={
                "commands/foo.md": "# foo",
                "commands/bar.md": "# bar",
            },
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=True)
        auditor.audit_plugin("skrills")
        missing = auditor.discrepancies["skrills"]["missing"].get("commands", [])
        assert "./commands/bar.md" in missing


class TestStaleRegistration:
    """
    Feature: detect plugin.json entries with no on-disk file

    As a maintainer
    I want stale registrations flagged
    So that deleted commands do not linger in the manifest.
    """

    @pytest.mark.unit
    def test_stale_command_is_reported(self, plugins_root: Path):
        """
        Scenario: manifest references a command file that was deleted
        Given a manifest with two commands but only one file on disk
        When I run the audit
        Then 'stale' contains the absent command
        """
        _make_plugin(
            plugins_root,
            "skrills",
            manifest={
                "name": "skrills",
                "version": "0.0.0",
                "commands": ["./commands/foo.md", "./commands/gone.md"],
            },
            files={"commands/foo.md": "# foo"},
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=True)
        auditor.audit_plugin("skrills")
        stale = auditor.discrepancies["skrills"]["stale"].get("commands", [])
        assert "./commands/gone.md" in stale


class TestFixMode:
    """
    Feature: --fix updates plugin.json in place

    As a release operator
    I want one command to bring the manifest into sync with disk
    So that I do not hand-edit JSON for trivial drift.
    """

    @pytest.mark.unit
    def test_fix_adds_missing_and_drops_stale(self, plugins_root: Path):
        """
        Scenario: drift in both directions
        Given a manifest missing a real file and listing a deleted one
        When I run the audit with dry_run=False
        Then plugin.json on disk lists exactly the on-disk commands, sorted
        """
        manifest_path = (
            _make_plugin(
                plugins_root,
                "skrills",
                manifest={
                    "name": "skrills",
                    "version": "0.0.0",
                    "commands": ["./commands/foo.md", "./commands/gone.md"],
                },
                files={
                    "commands/foo.md": "# foo",
                    "commands/bar.md": "# bar",
                },
            )
            / ".claude-plugin"
            / "plugin.json"
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=False)
        auditor.audit_all()
        rewritten = json.loads(manifest_path.read_text())
        assert rewritten["commands"] == [
            "./commands/bar.md",
            "./commands/foo.md",
        ]


class TestSkillsAndAgents:
    """
    Feature: detect missing skills/agents registrations

    As a plugin maintainer
    I want skills and agents drift surfaced too
    So that the audit covers all registration categories.
    """

    @pytest.mark.unit
    def test_skill_dir_with_skill_md_is_detected(self, plugins_root: Path):
        """
        Scenario: a skill directory exists with SKILL.md but is unregistered
        Given an on-disk skills/<name>/SKILL.md and an empty manifest skills field
        When I run the audit
        Then the skill path appears in 'missing'
        """
        skill_md = textwrap.dedent(
            """\
            ---
            name: my-skill
            description: x
            ---

            body
            """
        )
        _make_plugin(
            plugins_root,
            "skrills",
            manifest={"name": "skrills", "version": "0.0.0"},
            files={"skills/my-skill/SKILL.md": skill_md},
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=True)
        auditor.audit_plugin("skrills")
        missing = auditor.discrepancies["skrills"]["missing"].get("skills", [])
        assert "./skills/my-skill" in missing

    @pytest.mark.unit
    def test_agent_md_unregistered_is_detected(self, plugins_root: Path):
        """
        Scenario: agents/<name>.md exists but manifest has no agents entry
        Given an on-disk agent file and an empty manifest agents field
        When I run the audit
        Then the agent path appears in 'missing'
        """
        _make_plugin(
            plugins_root,
            "skrills",
            manifest={"name": "skrills", "version": "0.0.0"},
            files={"agents/helper.md": "# helper"},
        )
        auditor = upr.PluginAuditor(plugins_root, dry_run=True)
        auditor.audit_plugin("skrills")
        missing = auditor.discrepancies["skrills"]["missing"].get("agents", [])
        assert "./agents/helper.md" in missing
