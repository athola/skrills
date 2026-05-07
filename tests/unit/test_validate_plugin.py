"""BDD characterization tests for scripts/validate_plugin.py.

These tests lock in the contract of the port from
claude-night-market plugins/abstract/scripts/validate_plugin.py so future
edits cannot regress validation behaviour.
"""

from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest

from validate_plugin import PluginValidator  # type: ignore[import-not-found]  # pyright: ignore[reportMissingImports]


def _make_plugin(tmp_path: Path, manifest: dict, files: dict[str, str] | None = None) -> Path:
    """Materialize a plugin tree under tmp_path with a given manifest."""
    plugin_dir = tmp_path / "plugin"
    (plugin_dir / ".claude-plugin").mkdir(parents=True)
    (plugin_dir / ".claude-plugin" / "plugin.json").write_text(
        json.dumps(manifest, indent=2)
    )
    for rel, content in (files or {}).items():
        target = plugin_dir / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    return plugin_dir


class TestPluginManifestLocation:
    """
    Feature: plugin.json location enforcement

    As a plugin author
    I want validation to flag misplaced plugin.json
    So that the harness can locate my manifest.
    """

    @pytest.mark.unit
    def test_manifest_at_root_is_critical(self, tmp_path):
        """
        Scenario: manifest at plugin root instead of .claude-plugin/
        Given a plugin with plugin.json at the root
        When I validate the plugin
        Then the report contains a critical issue about location
        """
        plugin_dir = tmp_path / "plugin"
        plugin_dir.mkdir()
        (plugin_dir / "plugin.json").write_text("{}")
        validator = PluginValidator(plugin_dir)
        validator.validate()
        critical = " ".join(validator.issues["critical"])
        assert "found at root" in critical

    @pytest.mark.unit
    def test_missing_manifest_is_critical(self, tmp_path):
        """
        Scenario: no manifest anywhere
        Given a plugin directory with no plugin.json
        When I validate the plugin
        Then the report flags a missing manifest
        """
        plugin_dir = tmp_path / "plugin"
        plugin_dir.mkdir()
        validator = PluginValidator(plugin_dir)
        validator.validate()
        critical = " ".join(validator.issues["critical"])
        assert ".claude-plugin/plugin.json not found" in critical


class TestPluginNameRules:
    """
    Feature: kebab-case plugin name enforcement

    As a marketplace consumer
    I want plugin names to follow kebab-case
    So that discovery is consistent.
    """

    @pytest.mark.unit
    def test_kebab_case_passes(self, tmp_path):
        """
        Scenario: valid kebab-case name
        Given a manifest with name 'my-good-plugin'
        When I validate
        Then the info section confirms the convention is followed
        """
        plugin = _make_plugin(tmp_path, {"name": "my-good-plugin", "version": "1.0.0"})
        validator = PluginValidator(plugin)
        validator.validate()
        info_blob = " ".join(validator.issues["info"])
        assert "kebab-case" in info_blob

    @pytest.mark.unit
    def test_camel_case_is_critical(self, tmp_path):
        """
        Scenario: invalid camelCase name
        Given a manifest with name 'badName'
        When I validate
        Then a critical issue cites the invalid name
        """
        plugin = _make_plugin(tmp_path, {"name": "badName", "version": "1.0.0"})
        validator = PluginValidator(plugin)
        validator.validate()
        critical = " ".join(validator.issues["critical"])
        assert "Invalid plugin name" in critical


class TestPathReferences:
    """
    Feature: declared paths must exist

    As a plugin maintainer
    I want broken command paths to fail validation
    So that I cannot ship a manifest that references missing files.
    """

    @pytest.mark.unit
    def test_command_path_must_exist(self, tmp_path):
        """
        Scenario: manifest declares a command file that is not on disk
        Given a manifest with one command path that does not exist
        When I validate
        Then a critical issue names the missing path
        """
        plugin = _make_plugin(
            tmp_path,
            {
                "name": "demo-plugin",
                "version": "1.0.0",
                "commands": ["./commands/missing.md"],
            },
        )
        validator = PluginValidator(plugin)
        validator.validate()
        critical = " ".join(validator.issues["critical"])
        assert "Referenced commands path not found" in critical


class TestHooksAutoLoad:
    """
    Feature: hooks/hooks.json must not be listed in the hooks array

    As a Claude Code runtime
    I want to auto-load hooks/hooks.json without duplicates
    So that hook registrations stay unique.
    """

    @pytest.mark.unit
    def test_explicit_hooks_json_listed_is_critical(self, tmp_path):
        """
        Scenario: array entry duplicates the auto-loaded path
        Given a manifest hooks array containing './hooks/hooks.json'
        When I validate
        Then a critical 'Duplicate hooks.json ref' is reported
        """
        plugin = _make_plugin(
            tmp_path,
            {
                "name": "demo-plugin",
                "version": "1.0.0",
                "hooks": ["./hooks/hooks.json"],
            },
        )
        validator = PluginValidator(plugin)
        validator.validate()
        critical = " ".join(validator.issues["critical"])
        assert "Duplicate hooks.json ref" in critical


class TestSkillFrontmatter:
    """
    Feature: skill files declare YAML frontmatter

    As a skill consumer
    I want skills to expose name/description metadata
    So that discovery and routing work.
    """

    @pytest.mark.unit
    def test_skill_without_frontmatter_warns(self, tmp_path):
        """
        Scenario: SKILL.md missing the leading '---'
        Given a manifest declaring a skill whose SKILL.md lacks frontmatter
        When I validate
        Then a warning notes the missing YAML frontmatter
        """
        skill_md = textwrap.dedent(
            """\
            # My Skill

            Body without frontmatter.
            """
        )
        plugin = _make_plugin(
            tmp_path,
            {
                "name": "demo-plugin",
                "version": "1.0.0",
                "skills": ["./skills/my-skill"],
            },
            files={"skills/my-skill/SKILL.md": skill_md},
        )
        validator = PluginValidator(plugin)
        validator.validate()
        warnings = " ".join(validator.issues["warnings"])
        assert "YAML frontmatter" in warnings


class TestExitCode:
    """
    Feature: exit code reflects critical issues

    As a CI runner
    I want a non-zero exit code on critical findings
    So that broken plugins fail the build.
    """

    @pytest.mark.unit
    def test_clean_plugin_returns_zero(self, tmp_path):
        """
        Scenario: minimal valid manifest
        Given a kebab-case named manifest with no path references
        When I validate
        Then the validator returns exit code 0
        """
        plugin = _make_plugin(
            tmp_path,
            {"name": "ok-plugin", "version": "1.0.0", "description": "x"},
        )
        validator = PluginValidator(plugin)
        assert validator.validate() == 0

    @pytest.mark.unit
    def test_bad_name_returns_one(self, tmp_path):
        """
        Scenario: invalid plugin name
        Given a manifest with camelCase name
        When I validate
        Then the validator returns exit code 1
        """
        plugin = _make_plugin(tmp_path, {"name": "BadName", "version": "1.0.0"})
        validator = PluginValidator(plugin)
        assert validator.validate() == 1
