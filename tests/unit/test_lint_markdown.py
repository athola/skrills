"""scripts/lint-markdown.sh must lint the whole repo from any directory."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "lint-markdown.sh"


def test_runs_from_repo_root_when_called_from_a_subdirectory(tmp_path: Path) -> None:
    shim_dir = tmp_path / "bin"
    shim_dir.mkdir()
    record = tmp_path / "npx-cwd"
    npx = shim_dir / "npx"
    npx.write_text(f"#!/bin/sh\npwd -P > '{record}'\n")
    npx.chmod(0o755)
    env = {
        **os.environ,
        "PATH": f"{shim_dir}{os.pathsep}{os.environ['PATH']}",
        "NPM_CONFIG_CACHE": str(tmp_path / "cache"),
    }
    completed = subprocess.run(
        ["bash", str(SCRIPT)], cwd=REPO_ROOT / "tests", env=env,
        capture_output=True, text=True,
    )
    assert completed.returncode == 0, completed.stderr
    assert Path(record.read_text().strip()) == REPO_ROOT.resolve()
