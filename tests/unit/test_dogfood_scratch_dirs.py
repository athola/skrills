"""Scenario scratch dirs named with $RANDOM can repeat within one run, so a
later scenario could read an earlier one's output and pass on stale data."""

from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("script", ["dogfood-contracts.sh", "dogfood-tui.sh"])
def test_scenario_dirs_come_from_mktemp(script: str) -> None:
    text = (REPO_ROOT / "scripts" / script).read_text()
    assert "$RANDOM" not in text
