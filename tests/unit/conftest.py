"""Pytest configuration for tests/scripts/.

Adds the repo's scripts/ directory to sys.path so test modules can
import the ported validators directly. Registers the 'unit' marker so
the BDD characterization tests do not emit PytestUnknownMarkWarning.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_DIR = REPO_ROOT / "scripts"

if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))


def pytest_configure(config) -> None:  # type: ignore[no-untyped-def]
    config.addinivalue_line("markers", "unit: fast unit/characterization tests")


@pytest.fixture
def failing_rg_env(tmp_path: Path) -> dict[str, str]:
    """Env whose first `rg` on PATH is a shim exiting 2, ripgrep's error code.

    Shared by the prose and decoration lint suites: both assert that a ripgrep
    error exits 2 instead of reading as "no match" and reporting a tree clean.
    """
    shim_dir = tmp_path / "rg-shim-bin"
    shim_dir.mkdir()
    shim = shim_dir / "rg"
    shim.write_text("#!/bin/sh\nexit 2\n")
    shim.chmod(0o755)
    return {**os.environ, "PATH": f"{shim_dir}{os.pathsep}{os.environ['PATH']}"}
