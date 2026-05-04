#!/usr/bin/env python3
"""Check hooks for outdated patterns against Claude Code SDK spec.

Scans plugin hooks for known anti-patterns:
- PostToolUse hooks returning invalid decision values
- PreToolUse hooks using deprecated decision/reason fields
- Hooks missing stdin error handling
- Hooks printing unnecessary stdout on no-op paths

Ported from claude-night-market scripts/check_hook_modernization.py.

Exit codes:
    0 - no issues found (or --json mode)
    1 - error-severity issues detected (text mode only)
"""

from __future__ import annotations

import ast
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Finding:
    """A single modernization issue."""

    plugin: str
    file: str
    pattern: str
    severity: str  # "error" | "warning"
    message: str


@dataclass
class AuditResult:
    """Aggregated audit results."""

    findings: list[Finding] = field(default_factory=list)

    @property
    def error_count(self) -> int:
        return sum(1 for f in self.findings if f.severity == "error")

    @property
    def warning_count(self) -> int:
        return sum(1 for f in self.findings if f.severity == "warning")


# PostToolUse: decision can only be "block" or omitted.
_INVALID_POST_DECISION = {"ALLOW", "allow", "approve", "APPROVE"}

# PreToolUse: deprecated top-level fields.
_DEPRECATED_PRE_FIELDS = {"decision", "reason"}


def find_hooks_json(repo_root: Path) -> list[Path]:
    """Find all hooks.json files under plugins/<name>/hooks/."""
    return sorted(repo_root.glob("plugins/*/hooks/hooks.json"))


def find_hook_scripts(repo_root: Path) -> list[tuple[str, Path]]:
    """Find Python hook scripts paired with their plugin name."""
    results: list[tuple[str, Path]] = []
    for hooks_json in find_hooks_json(repo_root):
        plugin_dir = hooks_json.parent.parent
        plugin_name = plugin_dir.name
        for py_file in sorted(hooks_json.parent.glob("*.py")):
            if py_file.name.startswith("_"):
                continue
            results.append((plugin_name, py_file))
    return results


def get_hook_event_types(hooks_json: Path) -> dict[str, list[str]]:
    """Map script filenames to their hook event types from hooks.json."""
    try:
        data = json.loads(hooks_json.read_text())
    except (json.JSONDecodeError, OSError):
        return {}
    script_events: dict[str, list[str]] = {}
    hooks = data.get("hooks", {})
    for event_name, matchers in hooks.items():
        if not isinstance(matchers, list):
            continue
        for matcher_group in matchers:
            for hook in matcher_group.get("hooks", []):
                cmd = hook.get("command", "")
                for part in cmd.split():
                    if part.endswith(".py"):
                        filename = part.split("/")[-1]
                        script_events.setdefault(filename, []).append(event_name)
    return script_events


def check_python_source(
    source: str,
    plugin: str,
    filename: str,
    event_types: list[str],
) -> list[Finding]:
    """Check a Python hook source for anti-patterns."""
    findings: list[Finding] = []

    # Invalid PostToolUse decision values
    if "PostToolUse" in event_types or not event_types:
        for invalid in _INVALID_POST_DECISION:
            pattern = f'"decision": "{invalid}"'
            if pattern in source or f"'decision': '{invalid}'" in source:
                findings.append(
                    Finding(
                        plugin=plugin,
                        file=filename,
                        pattern="invalid-post-decision",
                        severity="error",
                        message=(
                            f'PostToolUse hook uses invalid decision value "{invalid}". '
                            f'Valid values: "block" or omit entirely.'
                        ),
                    )
                )

    # Deprecated PreToolUse top-level decision/reason
    if "PreToolUse" in event_types:
        for dep_field in _DEPRECATED_PRE_FIELDS:
            pattern = f'"{dep_field}":'
            if pattern in source:
                lines = source.split("\n")
                for i, line in enumerate(lines):
                    stripped = line.strip()
                    if pattern in stripped and "hookSpecificOutput" not in stripped:
                        context = "\n".join(lines[max(0, i - 3) : i + 1])
                        if "hookSpecificOutput" not in context:
                            findings.append(
                                Finding(
                                    plugin=plugin,
                                    file=filename,
                                    pattern="deprecated-pre-decision",
                                    severity="warning",
                                    message=(
                                        f'PreToolUse hook uses deprecated "{dep_field}" '
                                        f'field. Use "hookSpecificOutput.permissionDecision"'
                                        f" instead."
                                    ),
                                )
                            )
                            break

    # Stdin error handling
    if "sys.stdin" in source or "json.load" in source:
        has_try = "try:" in source
        has_json_except = "JSONDecodeError" in source or "ValueError" in source
        if not (has_try and has_json_except):
            findings.append(
                Finding(
                    plugin=plugin,
                    file=filename,
                    pattern="missing-stdin-error-handling",
                    severity="warning",
                    message=(
                        "Hook reads stdin but lacks try/except for "
                        "JSONDecodeError. Malformed input will crash the hook."
                    ),
                )
            )

    # Noisy no-op (PostToolUse with many stdout writes)
    if "PostToolUse" in event_types:
        try:
            tree = ast.parse(source)
        except SyntaxError:
            return findings
        print_count = 0
        for node in ast.walk(tree):
            if isinstance(node, ast.Call):
                func = node.func
                if isinstance(func, ast.Name) and func.id == "print":
                    print_count += 1
                elif isinstance(func, ast.Attribute) and func.attr == "write":
                    if (
                        isinstance(func.value, ast.Attribute)
                        and func.value.attr == "stdout"
                    ):
                        print_count += 1
        if print_count > 3:
            findings.append(
                Finding(
                    plugin=plugin,
                    file=filename,
                    pattern="noisy-no-op",
                    severity="warning",
                    message=(
                        f"PostToolUse hook has {print_count} stdout writes. "
                        "Consider silent exit for no-op paths (no output = allow)."
                    ),
                )
            )
    return findings


def run_audit(repo_root: Path) -> AuditResult:
    """Run the full modernization audit."""
    result = AuditResult()
    for plugin_name, py_file in find_hook_scripts(repo_root):
        hooks_json = py_file.parent / "hooks.json"
        event_map = get_hook_event_types(hooks_json)
        event_types = event_map.get(py_file.name, [])
        try:
            source = py_file.read_text()
        except OSError:
            continue
        findings = check_python_source(source, plugin_name, py_file.name, event_types)
        result.findings.extend(findings)
    return result


def format_text(result: AuditResult) -> str:
    """Format findings as a human-readable table."""
    if not result.findings:
        return "No modernization issues found."
    lines = ["Hook Modernization Audit", "=" * 60, ""]
    for f in result.findings:
        icon = "ERROR" if f.severity == "error" else "WARN "
        lines.append(f"  [{icon}] {f.plugin}/{f.file}")
        lines.append(f"          Pattern: {f.pattern}")
        lines.append(f"          {f.message}")
        lines.append("")
    lines.append(f"Total: {result.error_count} errors, {result.warning_count} warnings")
    return "\n".join(lines)


def format_json(result: AuditResult) -> str:
    """Format findings as JSON."""
    return json.dumps(
        {
            "success": True,
            "errors": result.error_count,
            "warnings": result.warning_count,
            "findings": [
                {
                    "plugin": f.plugin,
                    "file": f.file,
                    "pattern": f.pattern,
                    "severity": f.severity,
                    "message": f.message,
                }
                for f in result.findings
            ],
        },
        indent=2,
    )


def main(argv: list[str] | None = None) -> int:
    """CLI entry point."""
    args = argv if argv is not None else sys.argv[1:]
    output_json = "--json" in args
    repo_root = Path(__file__).resolve().parent.parent
    for i, arg in enumerate(args):
        if arg == "--root" and i + 1 < len(args):
            repo_root = Path(args[i + 1])
    result = run_audit(repo_root)
    if output_json:
        print(format_json(result))
        return 0
    print(format_text(result))
    return 1 if result.error_count > 0 else 0


if __name__ == "__main__":
    raise SystemExit(main())
