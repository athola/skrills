"""Gates the pre-commit hook enforces must also run in CI, or a PR from a
contributor without the hook merges failures that then block everyone."""

from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"


def _job(name: str) -> str:
    text = CI_WORKFLOW.read_text()
    match = re.search(rf"^  {name}:\n(.*?)(?=^  [\w-]+:\n|\Z)", text, re.MULTILINE | re.DOTALL)
    assert match, f"job {name} not found in ci.yml"
    return match.group(1)


def test_hygiene_job_runs_markdown_lint() -> None:
    assert "run: make lint-md" in _job("hygiene-and-scripts")


def test_hygiene_job_installs_shellcheck_for_lint_shell() -> None:
    assert "shellcheck" in _job("hygiene-and-scripts")
