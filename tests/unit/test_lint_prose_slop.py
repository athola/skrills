"""BDD characterization tests for scripts/lint-prose-slop.sh.

These tests lock the banned-vocabulary detection and exclusion
contract so the AI hygiene gate cannot silently weaken.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "lint-prose-slop.sh"


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
    reason="ripgrep (rg) not installed; lint-prose-slop requires it.",
)


class TestProseSlopLint:
    """
    Feature: lint-prose-slop blocks banned vocabulary in markdown

    As a documentation reviewer
    I want CI to fail when banned words appear in user-facing docs
    So that AI slop never lands in the documentation tree.
    """

    @pytest.mark.unit
    def test_clean_tree_exits_zero(self, tmp_path):
        """
        Scenario: a tree with no banned vocabulary
        Given a markdown file containing only allowed prose
        When I run the lint
        Then it exits 0 and reports the tree as clean.
        """
        (tmp_path / "README.md").write_text(
            "# Project\n\nThis is a clear, direct description.\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
        assert "clean" in result.stdout

    @pytest.mark.unit
    def test_banned_word_in_user_facing_md_fails(self, tmp_path):
        """
        Scenario: banned word in non-archive markdown
        Given a markdown file with the word 'leverage'
        When I run the lint
        Then it exits 1 and explains the failure on stderr.
        """
        (tmp_path / "guide.md").write_text(
            "We will leverage this approach for the rollout.\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 1
        assert "AI-slop vocabulary detected" in result.stderr

    @pytest.mark.unit
    def test_archived_doc_is_excluded(self, tmp_path):
        """
        Scenario: banned word inside docs/archive/
        Given an archived doc that legitimately uses 'leverage'
        When I run the lint
        Then it exits 0 because docs/archive/ is excluded.
        """
        archive = tmp_path / "docs" / "archive"
        archive.mkdir(parents=True)
        (archive / "old-plan.md").write_text(
            "Historical narrative about leverage and seamless rollout.\n"
        )
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
