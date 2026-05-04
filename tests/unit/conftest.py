"""Pytest configuration for tests/scripts/.

Adds the repo's scripts/ directory to sys.path so test modules can
import the ported validators directly. Registers the 'unit' marker so
the BDD characterization tests do not emit PytestUnknownMarkWarning.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_DIR = REPO_ROOT / "scripts"

if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))


def pytest_configure(config) -> None:  # type: ignore[no-untyped-def]
    config.addinivalue_line("markers", "unit: fast unit/characterization tests")
