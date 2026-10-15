"""Behaviour of scripts/install.sh that the helper-function suite cannot see."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALL_SCRIPT = REPO_ROOT / "scripts" / "install.sh"


def test_truncated_install_script_runs_nothing(tmp_path: Path) -> None:
    """Under `curl | sh` a dropped connection hands sh a partial script. Every
    command must sit inside main(), so a copy missing its final line (the
    `main` call) defines functions and does nothing else."""
    lines = INSTALL_SCRIPT.read_text().rstrip("\n").splitlines()
    truncated = tmp_path / "install.sh"
    truncated.write_text("\n".join(lines[:-1]) + "\n")

    shim_dir = tmp_path / "bin"
    shim_dir.mkdir()
    marker = tmp_path / "curl-called"
    curl = shim_dir / "curl"
    curl.write_text(f"#!/bin/sh\ntouch '{marker}'\nexit 1\n")
    curl.chmod(0o755)

    env = {
        **os.environ,
        "PATH": f"{shim_dir}{os.pathsep}{os.environ['PATH']}",
        "HOME": str(tmp_path),
        "SKRILLS_TARGET": "x86_64-unknown-linux-gnu",
        "SKRILLS_NO_HOOK": "1",
    }
    completed = subprocess.run(
        ["sh", str(truncated)], env=env, capture_output=True, text=True
    )
    assert not marker.exists(), completed.stderr
    assert completed.returncode == 0, completed.stderr


def test_install_script_ends_by_calling_main() -> None:
    last = INSTALL_SCRIPT.read_text().rstrip("\n").splitlines()[-1]
    assert last == 'main "$@"'
