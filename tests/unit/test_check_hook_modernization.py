"""BDD tests for scripts/check_hook_modernization.py.

Drives the port of the night-market modernization auditor by describing
the patterns it must detect across the plugins/<name>/hooks/ tree.
"""

from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest

import check_hook_modernization as mod  # type: ignore[import-not-found]  # pyright: ignore[reportMissingImports]


def _make_hook_plugin(
    repo_root: Path,
    plugin_name: str,
    hooks_json: dict,
    scripts: dict[str, str],
) -> Path:
    """Materialize plugins/<plugin_name>/hooks/ with hooks.json + scripts."""
    hooks_dir = repo_root / "plugins" / plugin_name / "hooks"
    hooks_dir.mkdir(parents=True)
    (hooks_dir / "hooks.json").write_text(json.dumps(hooks_json, indent=2))
    for filename, source in scripts.items():
        (hooks_dir / filename).write_text(source)
    return hooks_dir


@pytest.fixture()
def fake_repo(tmp_path: Path) -> Path:
    """An empty fake repo root with a plugins/ directory."""
    (tmp_path / "plugins").mkdir()
    return tmp_path


class TestNoHooksClean:
    """
    Feature: no hooks means no findings

    As a Rust crate maintainer
    I want the audit to be silent when no hook scripts exist
    So that running it on hookless plugins is harmless.
    """

    @pytest.mark.unit
    def test_empty_repo_returns_zero_findings(self, fake_repo: Path):
        """
        Scenario: no hooks at all
        Given a repo with plugins/ but no hook scripts
        When I run the audit
        Then the result has zero findings
        """
        result = mod.run_audit(fake_repo)
        assert result.findings == []
        assert result.error_count == 0
        assert result.warning_count == 0


class TestPostToolUseInvalidDecision:
    """
    Feature: detect invalid PostToolUse decision values

    As a hook author
    I want the auditor to flag 'allow'/'approve' decisions
    So that I migrate to the SDK-valid 'block' or omitted form.
    """

    @pytest.mark.unit
    def test_allow_decision_is_error(self, fake_repo: Path):
        """
        Scenario: PostToolUse hook returns {"decision": "allow"}
        Given a hooks.json registering hook.py for PostToolUse
        And hook.py contains the literal '"decision": "allow"'
        When I run the audit
        Then a finding with pattern 'invalid-post-decision' and severity error appears
        """
        _make_hook_plugin(
            fake_repo,
            "demo",
            hooks_json={
                "hooks": {
                    "PostToolUse": [
                        {"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/hooks/hook.py"}]}
                    ]
                }
            },
            scripts={
                "hook.py": textwrap.dedent(
                    """\
                    import json, sys
                    print(json.dumps({"decision": "allow"}))
                    """
                )
            },
        )
        result = mod.run_audit(fake_repo)
        patterns = {f.pattern for f in result.findings}
        assert "invalid-post-decision" in patterns
        assert any(f.severity == "error" for f in result.findings)


class TestPreToolUseDeprecatedField:
    """
    Feature: detect deprecated PreToolUse decision/reason fields

    As a hook author
    I want a warning when I use top-level 'decision' / 'reason' for PreToolUse
    So that I migrate to hookSpecificOutput.permissionDecision.
    """

    @pytest.mark.unit
    def test_top_level_decision_is_warning(self, fake_repo: Path):
        """
        Scenario: PreToolUse hook prints top-level 'decision'
        Given a hooks.json registering hook.py for PreToolUse
        And hook.py prints {"decision": "..."} at top level
        When I run the audit
        Then a 'deprecated-pre-decision' warning is reported
        """
        _make_hook_plugin(
            fake_repo,
            "demo",
            hooks_json={
                "hooks": {
                    "PreToolUse": [
                        {"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/hooks/hook.py"}]}
                    ]
                }
            },
            scripts={
                "hook.py": textwrap.dedent(
                    """\
                    import json
                    out = {"decision": "block", "reason": "no"}
                    print(json.dumps(out))
                    """
                )
            },
        )
        result = mod.run_audit(fake_repo)
        deprecated = [f for f in result.findings if f.pattern == "deprecated-pre-decision"]
        assert deprecated, "expected at least one deprecated-pre-decision finding"
        assert all(f.severity == "warning" for f in deprecated)


class TestStdinErrorHandling:
    """
    Feature: hooks reading stdin must guard JSONDecodeError

    As a runtime
    I want hooks to handle malformed input gracefully
    So that the dispatcher does not crash.
    """

    @pytest.mark.unit
    def test_unguarded_stdin_warns(self, fake_repo: Path):
        """
        Scenario: hook reads json.load(sys.stdin) without try/except
        Given a hook that reads stdin JSON without error handling
        When I run the audit
        Then a 'missing-stdin-error-handling' warning fires
        """
        _make_hook_plugin(
            fake_repo,
            "demo",
            hooks_json={
                "hooks": {
                    "PreToolUse": [
                        {"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/hooks/hook.py"}]}
                    ]
                }
            },
            scripts={
                "hook.py": textwrap.dedent(
                    """\
                    import json, sys
                    payload = json.load(sys.stdin)
                    print(payload)
                    """
                )
            },
        )
        result = mod.run_audit(fake_repo)
        patterns = {f.pattern for f in result.findings}
        assert "missing-stdin-error-handling" in patterns


class TestCliMain:
    """
    Feature: CLI entry point honors --json and --root

    As a Makefile recipe
    I want a programmatic JSON output mode
    So that downstream tooling can consume findings.
    """

    @pytest.mark.unit
    def test_json_mode_returns_zero_and_prints_json(
        self, fake_repo: Path, capsys: pytest.CaptureFixture[str]
    ):
        """
        Scenario: clean repo + --json
        Given an empty fake_repo
        When I call main(['--json', '--root', <fake_repo>])
        Then the exit code is 0 and stdout parses as the JSON envelope
        """
        rc = mod.main(["--json", "--root", str(fake_repo)])
        captured = capsys.readouterr().out
        assert rc == 0
        payload = json.loads(captured)
        assert payload["success"] is True
        assert payload["errors"] == 0
        assert payload["findings"] == []

    @pytest.mark.unit
    def test_text_mode_with_error_returns_one(self, fake_repo: Path):
        """
        Scenario: text mode + at least one error finding
        Given a hook with an invalid PostToolUse decision
        When I call main(['--root', <fake_repo>])
        Then the exit code is 1
        """
        _make_hook_plugin(
            fake_repo,
            "demo",
            hooks_json={
                "hooks": {
                    "PostToolUse": [
                        {"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/hooks/hook.py"}]}
                    ]
                }
            },
            scripts={
                "hook.py": '''print('{"decision": "allow"}')''',
            },
        )
        rc = mod.main(["--root", str(fake_repo)])
        assert rc == 1
