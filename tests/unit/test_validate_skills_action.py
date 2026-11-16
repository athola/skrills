"""The validate-skills action must install the skrills it was asked for.

Copied out of this repository (a Marketplace listing, say), the action has no
local scripts/install.sh and downloads one. Fetching it from HEAD meant a
workflow pinned to `version: 0.9.1` ran whatever installer master had that
day, so the download follows the requested release tag instead.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ACTION = REPO_ROOT / ".github" / "actions" / "validate-skills" / "action.yml"


def _install_script() -> str:
    text = ACTION.read_text()
    match = re.search(
        r"- name: Install skrills\n.*?run: \|\n(.*?)(?=\n    - name:|\Z)",
        text,
        re.DOTALL,
    )
    assert match, "Install skrills step not found in action.yml"
    lines = match.group(1).splitlines()
    return "\n".join(line[8:] for line in lines)


def _fetched_url(tmp_path: Path, version: str) -> str:
    """Run the install step from a copy with no local installer."""
    action_path = tmp_path / "standalone" / "action"
    action_path.mkdir(parents=True)
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    curl_log = tmp_path / "curl-args"
    curl = fake_bin / "curl"
    curl.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{curl_log}"\nprintf "exit 0\\n"\n')
    curl.chmod(0o755)

    script = _install_script().replace("${{ github.action_path }}", str(action_path))
    env = {
        "PATH": f"{fake_bin}:{os.environ['PATH']}",
        "SKRILLS_VERSION": version,
        "RUNNER_TEMP": str(tmp_path),
        "GITHUB_PATH": str(tmp_path / "github-path"),
    }
    subprocess.run(["bash", "-c", script], env=env, check=True)
    return next(arg for arg in curl_log.read_text().split() if arg.startswith("https://"))


def test_pinned_version_downloads_installer_from_its_release_tag(tmp_path: Path) -> None:
    url = _fetched_url(tmp_path, "0.9.1")
    assert url == "https://raw.githubusercontent.com/athola/skrills/v0.9.1/scripts/install.sh"


def test_latest_version_downloads_installer_from_default_branch(tmp_path: Path) -> None:
    url = _fetched_url(tmp_path, "latest")
    assert url == "https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh"
