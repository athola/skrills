"""BDD characterization tests for scripts/audit-plugins.sh.

These tests lock the help, error, and dispatch behaviour of the
audit shim so future edits cannot silently break the documented
contract.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "audit-plugins.sh"


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["NIGHT_MARKET_ROOT"] = ""
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


class TestHelp:
    """
    Feature: --help prints usage extracted from the script header

    As a plugin author
    I want a help message that summarises flags and environment
    So that I do not have to read the source.
    """

    @pytest.mark.unit
    def test_help_exits_zero(self):
        """
        Scenario: passing --help
        Given the audit shim
        When I invoke it with --help
        Then it exits 0.
        """
        result = _run(["--help"])
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_help_lists_documented_flags(self):
        """
        Scenario: help advertises every flag
        Given the audit shim
        When I invoke --help
        Then the output names each documented flag.
        """
        result = _run(["--help"])
        for flag in ("--fix", "--validate", "--modernize", "--all"):
            assert flag in result.stdout, f"{flag} missing from help output"

    @pytest.mark.unit
    def test_help_does_not_leak_script_body(self):
        """
        Scenario: help stays inside the comment block
        Given the audit shim uses `sed -n 'N,Mp'` to extract help
        When I invoke --help
        Then the script body (e.g. `set -euo pipefail`) does not appear.

        If this fails, the line range in the help extractor is stale
        relative to the header. Tighten the sed range rather than
        weakening this assertion.
        """
        result = _run(["--help"])
        assert "set -euo pipefail" not in result.stdout, (
            "Script body leaked into --help output. "
            "Update the sed line range to stop at the last comment line."
        )


class TestUnknownFlag:
    """
    Feature: unknown flags exit with EX_USAGE (64)

    As a CI gate
    I want an unambiguous failure code for bad invocations
    So that typos cannot silently become no-ops.
    """

    @pytest.mark.unit
    def test_unknown_flag_exits_64(self):
        """
        Scenario: invalid flag
        Given a typo such as --bogus
        When I invoke the shim
        Then it exits 64 and reports the unknown flag on stderr.
        """
        result = _run(["--bogus"])
        assert result.returncode == 64
        assert "Unknown flag" in result.stderr
