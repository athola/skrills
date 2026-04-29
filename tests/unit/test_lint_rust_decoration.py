"""BDD characterization tests for scripts/lint-rust-decoration.sh.

These tests lock the decorative-separator detection contract so
the AI hygiene gate cannot regress when ports are refactored.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "lint-rust-decoration.sh"


def _run(cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT)],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )


pytestmark = pytest.mark.skipif(
    shutil.which("rg") is None,
    reason="ripgrep (rg) not installed; lint-rust-decoration requires it.",
)


class TestRustDecorationLint:
    """
    Feature: block `// ─{20,}` decorative separator comments

    As a Rust reviewer
    I want CI to fail when AI-style separator comments appear
    So that source files stay free of generation artefacts.
    """

    @pytest.mark.unit
    def test_clean_rust_tree_exits_zero(self, tmp_path):
        """
        Scenario: ordinary Rust source
        Given a .rs file with only short comments
        When I run the lint
        Then it exits 0 and reports the tree as clean.
        """
        (tmp_path / "ok.rs").write_text(
            "// short note\nfn main() {}\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
        assert "clean" in result.stdout

    @pytest.mark.unit
    def test_long_separator_fails(self, tmp_path):
        """
        Scenario: 25 box-drawing chars in a comment
        Given a .rs file with a 25-char `// ─` separator
        When I run the lint
        Then it exits 1 and explains the failure on stderr.
        """
        decoration = "─" * 25
        (tmp_path / "bad.rs").write_text(
            f"// {decoration}\nfn main() {{}}\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 1
        assert "decorative separator" in result.stderr

    @pytest.mark.unit
    def test_short_separator_passes(self, tmp_path):
        """
        Scenario: 5 box-drawing chars (below threshold)
        Given a .rs file with only 5 separator chars
        When I run the lint
        Then it exits 0 because the threshold is 20.
        """
        short = "─" * 5
        (tmp_path / "short.rs").write_text(
            f"// {short}\nfn main() {{}}\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
