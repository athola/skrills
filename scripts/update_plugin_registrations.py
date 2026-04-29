#!/usr/bin/env python3
"""Audit and sync plugin.json files with disk contents.

Phase-1-only port of claude-night-market
plugins/sanctum/scripts/update_plugin_registrations.py.

Skipped from the upstream:
- Phase 2 PerformanceAnalyzer (depends on night-market metrics DB)
- Phase 3 MetaEvaluator (skill-eval framework)
- Phase 4 KnowledgeQueueChecker (memory-palace queue)
- Skill module audit (no module-bearing skills here yet)

Kept verbatim:
- Disk vs manifest comparison for commands/skills/agents/hooks
- Hooks resolution from hooks.json (auto-loaded by Claude Code)
- --fix mode that rewrites plugin.json with sorted entries
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

CACHE_EXCLUDES = {"__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "node_modules"}


class PluginAuditor:
    """Audit and sync plugin.json registrations with disk contents."""

    def __init__(self, plugins_root: Path, dry_run: bool = True) -> None:
        self.plugins_root = plugins_root
        self.dry_run = dry_run
        self.discrepancies: dict[str, Any] = {}

    def _should_exclude(self, path: Path) -> bool:
        return any(exclude in path.parts for exclude in CACHE_EXCLUDES)

    def scan_disk_files(self, plugin_path: Path) -> dict[str, list[str]]:
        results: dict[str, list[str]] = {
            "commands": [],
            "skills": [],
            "agents": [],
            "hooks": [],
        }

        commands_dir = plugin_path / "commands"
        if commands_dir.exists():
            for cmd_file in commands_dir.rglob("*.md"):
                if self._should_exclude(cmd_file):
                    continue
                rel_to_commands = cmd_file.relative_to(commands_dir)
                if any(
                    "module" in part.lower() or part == "steps"
                    for part in rel_to_commands.parts[:-1]
                ):
                    continue
                if len(rel_to_commands.parts) == 1:
                    results["commands"].append(f"./commands/{cmd_file.name}")

        skills_dir = plugin_path / "skills"
        if skills_dir.exists():
            for skill_dir in skills_dir.iterdir():
                if skill_dir.is_dir() and not self._should_exclude(skill_dir):
                    has_skill_md = (skill_dir / "SKILL.md").exists()
                    has_root_md_files = any(
                        f.suffix == ".md" for f in skill_dir.iterdir() if f.is_file()
                    )
                    if has_skill_md or has_root_md_files:
                        results["skills"].append(f"./skills/{skill_dir.name}")

        agents_dir = plugin_path / "agents"
        if agents_dir.exists():
            for agent_file in agents_dir.glob("*.md"):
                if not self._should_exclude(agent_file):
                    results["agents"].append(f"./agents/{agent_file.name}")

        hooks_dir = plugin_path / "hooks"
        if hooks_dir.exists():
            for hook_file in hooks_dir.iterdir():
                if self._should_exclude(hook_file):
                    continue
                if hook_file.is_file() and hook_file.suffix in (".sh", ".py"):
                    if not hook_file.name.startswith("test_") and not hook_file.name.startswith("_"):
                        results["hooks"].append(f"./hooks/{hook_file.name}")

        for items in results.values():
            items.sort()
        return results

    def read_plugin_json(self, plugin_path: Path) -> dict[str, Any] | None:
        plugin_json = plugin_path / ".claude-plugin" / "plugin.json"
        if not plugin_json.exists():
            return None
        try:
            with plugin_json.open(encoding="utf-8") as f:
                return json.load(f)
        except (json.JSONDecodeError, OSError) as exc:
            print(f"[ERROR] Failed to read {plugin_json}: {exc}")
            return None

    def resolve_hooks_json(
        self, plugin_path: Path, hooks_json_ref: str
    ) -> list[str] | None:
        hooks_json_path = plugin_path / hooks_json_ref.lstrip("./")
        if not hooks_json_path.exists():
            print(f"[WARN] hooks.json reference not found: {hooks_json_path}")
            return None
        try:
            with hooks_json_path.open(encoding="utf-8") as f:
                hooks_data = json.load(f)
        except (json.JSONDecodeError, OSError) as exc:
            print(f"[ERROR] Failed to read {hooks_json_path}: {exc}")
            return None

        hook_scripts: set[str] = set()
        for _event, matchers in (hooks_data.get("hooks") or {}).items():
            if not isinstance(matchers, list):
                continue
            for matcher in matchers:
                if not isinstance(matcher, dict):
                    continue
                for hook_def in matcher.get("hooks") or []:
                    if not isinstance(hook_def, dict):
                        continue
                    cmd = hook_def.get("command", "")
                    if cmd:
                        script_path = self._extract_script_path(cmd)
                        if script_path:
                            hook_scripts.add(script_path)
        return sorted(hook_scripts)

    def _extract_script_path(self, command: str) -> str | None:
        match = re.search(r"\$\{CLAUDE_PLUGIN_ROOT\}/(.+?)(?:\s|$)", command)
        if match:
            return f"./{match.group(1).strip()}"
        match = re.search(r"(\./hooks/[^\s]+)", command)
        if match:
            return match.group(1)
        return None

    def _resolve_hooks_set(
        self, plugin_path: Path, json_value: Any
    ) -> set[str] | None:
        standard_hooks_json = plugin_path / "hooks" / "hooks.json"
        if isinstance(json_value, str):
            if json_value.endswith(".json"):
                resolved = self.resolve_hooks_json(plugin_path, json_value)
                return set(resolved) if resolved is not None else None
            print(f"[WARN] Unexpected hooks format: {json_value}")
            return None
        if standard_hooks_json.exists():
            resolved = self.resolve_hooks_json(plugin_path, "./hooks/hooks.json")
            return set(resolved) if resolved is not None else set()
        return set(json_value) if json_value else set()

    def compare_registrations(
        self,
        plugin_path: Path,
        on_disk: dict[str, list[str]],
        in_json: dict[str, Any],
    ) -> dict[str, Any]:
        plugin_path = Path(plugin_path)
        discrepancies: dict[str, Any] = {"missing": {}, "stale": {}}
        for category in ("commands", "skills", "agents", "hooks"):
            disk_set = set(on_disk[category])
            json_value = in_json.get(category, [])
            if category == "hooks":
                hooks_set = self._resolve_hooks_set(plugin_path, json_value)
                if hooks_set is None:
                    continue
                json_set = hooks_set
            else:
                json_set = set(json_value) if json_value else set()
            missing = disk_set - json_set
            stale = json_set - disk_set
            if missing:
                discrepancies["missing"][category] = sorted(missing)
            if stale:
                discrepancies["stale"][category] = sorted(stale)
        return discrepancies

    def audit_plugin(self, plugin_name: str) -> bool:
        plugin_path = self.plugins_root / plugin_name
        if not plugin_path.exists() or not plugin_path.is_dir():
            print(f"[SKIP] {plugin_name}: not a directory")
            return False
        plugin_json_data = self.read_plugin_json(plugin_path)
        if plugin_json_data is None:
            print(f"[SKIP] {plugin_name}: no valid plugin.json")
            return False
        on_disk = self.scan_disk_files(plugin_path)
        discrepancies = self.compare_registrations(plugin_path, on_disk, plugin_json_data)
        has_discrepancies = bool(discrepancies["missing"] or discrepancies["stale"])
        if has_discrepancies:
            self.discrepancies[plugin_name] = discrepancies
            self._print_discrepancies(plugin_name, discrepancies)
        return has_discrepancies

    def _print_discrepancies(
        self, plugin_name: str, discrepancies: dict[str, Any]
    ) -> None:
        print(f"\n{'=' * 60}")
        print(f"PLUGIN: {plugin_name}")
        print("=" * 60)
        if discrepancies["missing"]:
            print("\n[MISSING] Files on disk but not in plugin.json:")
            for category, items in discrepancies["missing"].items():
                print(f"  {category}:")
                for item in items:
                    print(f"    - {item}")
        if discrepancies["stale"]:
            print("\n[STALE] Registered in plugin.json but not on disk:")
            for category, items in discrepancies["stale"].items():
                print(f"  {category}:")
                for item in items:
                    print(f"    - {item}")

    def _discover_plugin(
        self, plugin_name: str
    ) -> tuple[Path, Path, dict[str, Any]] | None:
        if plugin_name not in self.discrepancies:
            return None
        plugin_path = self.plugins_root / plugin_name
        plugin_json_path = plugin_path / ".claude-plugin" / "plugin.json"
        try:
            with plugin_json_path.open(encoding="utf-8") as f:
                plugin_data = json.load(f)
        except (OSError, json.JSONDecodeError) as exc:
            print(
                f"[ERROR] {plugin_name}: failed to read {plugin_json_path}: {exc}",
                file=sys.stderr,
            )
            return None
        return plugin_path, plugin_json_path, plugin_data

    def _validate_registration(
        self,
        plugin_name: str,
        plugin_path: Path,
        plugin_data: dict[str, Any],
    ) -> tuple[dict[str, Any], bool]:
        disc = self.discrepancies[plugin_name]
        standard_hooks_json = plugin_path / "hooks" / "hooks.json"
        hooks_need_manual_fix = False

        for category, items in disc["missing"].items():
            if category == "hooks":
                hooks_ref = plugin_data.get("hooks", "./hooks/hooks.json")
                if standard_hooks_json.exists() or isinstance(plugin_data.get("hooks"), str):
                    print(f"[MANUAL] {plugin_name}: hooks are auto-loaded from hooks.json")
                    print(f"         Update {hooks_ref} to add missing hooks:")
                else:
                    print(f"[MANUAL] {plugin_name}: no hooks.json found, create one with:")
                for item in items:
                    print(f"           - {item}")
                hooks_need_manual_fix = True
                continue
            if category not in plugin_data:
                plugin_data[category] = []
            plugin_data[category].extend(items)
            plugin_data[category].sort()

        for category, items in disc["stale"].items():
            if category == "hooks":
                if not hooks_need_manual_fix:
                    hooks_ref = plugin_data.get("hooks", "./hooks/hooks.json")
                    print(f"[MANUAL] {plugin_name}: hooks are auto-loaded from hooks.json")
                    print(f"         Update {hooks_ref} to remove stale hooks:")
                for item in items:
                    print(f"           - {item}")
                continue
            if category in plugin_data:
                plugin_data[category] = [
                    item for item in plugin_data[category] if item not in items
                ]
        return plugin_data, hooks_need_manual_fix

    def _apply_fixes(
        self,
        plugin_name: str,
        plugin_json_path: Path,
        plugin_data: dict[str, Any],
    ) -> bool:
        disc = self.discrepancies[plugin_name]
        non_hooks_changes = any(
            cat != "hooks"
            for cat in list(disc["missing"].keys()) + list(disc["stale"].keys())
        )
        if not non_hooks_changes:
            return True
        if not self.dry_run:
            with plugin_json_path.open("w", encoding="utf-8") as f:
                json.dump(plugin_data, f, indent=2, ensure_ascii=False)
                f.write("\n")
            print(f"[FIXED] {plugin_name}: plugin.json updated")
        else:
            print(f"[DRY-RUN] {plugin_name}: would update plugin.json")
        return True

    def fix_plugin(self, plugin_name: str) -> bool:
        if plugin_name not in self.discrepancies:
            return True
        discovered = self._discover_plugin(plugin_name)
        if discovered is None:
            return False
        plugin_path, plugin_json_path, plugin_data = discovered
        plugin_data, _hooks_manual = self._validate_registration(
            plugin_name, plugin_path, plugin_data
        )
        return self._apply_fixes(plugin_name, plugin_json_path, plugin_data)

    def audit_all(self, specific_plugin: str | None = None) -> int:
        if specific_plugin:
            plugins = [specific_plugin]
        else:
            plugins = sorted(
                p.name
                for p in self.plugins_root.iterdir()
                if p.is_dir() and not p.name.startswith(".")
            )
        print(f"Auditing {len(plugins)} plugin(s)...\n")

        plugins_with_issues = 0
        for plugin_name in plugins:
            if self.audit_plugin(plugin_name):
                plugins_with_issues += 1

        print(f"\n{'=' * 60}")
        print("AUDIT SUMMARY")
        print("=" * 60)
        print(f"Plugins audited: {len(plugins)}")
        print(f"Plugins with registration issues: {len(self.discrepancies)}")
        print(f"Plugins clean: {len(plugins) - plugins_with_issues}")

        if not self.dry_run and plugins_with_issues > 0:
            print(f"\n{'=' * 60}")
            print("FIXING DISCREPANCIES")
            print("=" * 60)
            for plugin_name in self.discrepancies:
                self.fix_plugin(plugin_name)

        return plugins_with_issues


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Audit and sync plugin.json files with disk contents (Phase 1)."
    )
    parser.add_argument(
        "plugin", nargs="?", help="Specific plugin to audit (default: all plugins)"
    )
    parser.add_argument(
        "--fix",
        action="store_true",
        help="Fix discrepancies by updating plugin.json files",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        default=True,
        help="Show discrepancies without making changes (default)",
    )
    parser.add_argument(
        "--plugins-root",
        type=Path,
        default=Path.cwd() / "plugins",
        help="Root directory containing plugins (default: ./plugins)",
    )
    args = parser.parse_args()
    if args.fix:
        args.dry_run = False

    if not args.plugins_root.exists():
        print(f"[ERROR] Plugins root not found: {args.plugins_root}")
        sys.exit(1)

    auditor = PluginAuditor(args.plugins_root, dry_run=args.dry_run)
    issues_found = auditor.audit_all(args.plugin)

    if issues_found > 0 and args.dry_run:
        print("\n[HINT] Run with --fix to automatically update plugin.json files")
        sys.exit(1)
    elif issues_found > 0:
        sys.exit(1)
    else:
        print("\n[SUCCESS] All plugins have consistent registrations!")
        sys.exit(0)


if __name__ == "__main__":
    main()
