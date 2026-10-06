"""BDD characterization tests for the install.sh slice in dogfood-contracts.sh.

The dogfood script sources a slice of scripts/install.sh so the asset-selection
scenarios exercise the real function instead of a copy. `sed` prints to EOF when
its end pattern is absent, so a drifted marker would hand the installer's main
body to `.`, which downloads a release and replaces the script's EXIT trap.
These tests lock the guards that refuse such a slice.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dogfood-contracts.sh"
INSTALL_SCRIPT = REPO_ROOT / "scripts" / "install.sh"
ENTRYPOINT_REL = Path(".github/actions/validate-skills/entrypoint.sh")
END_MARKER = "# --- main"

STUB = "#!/bin/sh\nexit 0\n"

pytestmark = pytest.mark.skipif(
    shutil.which("jq") is None,
    reason="jq not installed; dogfood-contracts requires it.",
)


def _scratch_repo(tmp_path: Path, install_body: str) -> Path:
    """Build the smallest tree dogfood-contracts.sh will run against.

    The scenarios before the install.sh slice need an executable binary, the
    action entrypoint and a skills directory. Stubs are enough: those scenarios
    are expected to report failures, and the slice guards run regardless.
    """
    scripts = tmp_path / "scripts"
    scripts.mkdir()
    shutil.copy2(SCRIPT, scripts / SCRIPT.name)
    (scripts / "install.sh").write_text(install_body)
    entrypoint = tmp_path / ENTRYPOINT_REL
    entrypoint.parent.mkdir(parents=True)
    entrypoint.write_text(STUB)
    (tmp_path / "skills").mkdir()
    stub_bin = tmp_path / "fake-skrills"
    stub_bin.write_text(STUB)
    stub_bin.chmod(0o755)
    return scripts / SCRIPT.name


def _run(tmp_path: Path, install_body: str) -> subprocess.CompletedProcess[str]:
    copied = _scratch_repo(tmp_path, install_body)
    env = {
        **os.environ,
        "BIN_PATH": "fake-skrills",
        # Containment, not part of the assertion. Writing these tests without
        # them installed a release into ~/.skrills/bin and ran `skrills setup`,
        # which is precisely the damage the guards under test prevent. If a
        # guard ever regresses, the damage lands in the throwaway tree instead.
        "SKRILLS_BIN_DIR": str(tmp_path / "would-be-install"),
        "SKRILLS_NO_HOOK": "1",
        "SKRILLS_SKIP_PATH_MESSAGE": "1",
    }
    return subprocess.run(
        ["bash", str(copied)],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


class TestInstallSliceGuards:
    """
    Feature: dogfood-contracts refuses an install.sh slice it cannot trust

    As a maintainer running `make precommit`
    I want the dogfood script to abort when the slice markers drift
    So that it never sources the installer's main body and downloads a release.
    """

    @pytest.mark.unit
    def test_renamed_end_marker_aborts_before_sourcing(self, tmp_path):
        """
        Scenario: the `# --- main` marker is renamed
        Given an install.sh whose end marker no longer matches
        When the dogfood script runs
        Then it exits 2 naming the missing marker, and not the later
        "SELECT_ASSET_FROM_JSON undefined" message, which would mean the slice
        had already been sourced.
        """
        body = INSTALL_SCRIPT.read_text().replace(END_MARKER, "# --- entry", 1)
        result = _run(tmp_path, body)
        assert result.returncode == 2, result.stdout
        assert "marker" in result.stderr
        assert "SELECT_ASSET_FROM_JSON undefined" not in result.stderr

    @pytest.mark.unit
    def test_slice_reaching_the_main_body_is_refused(self, tmp_path):
        """
        Scenario: both markers present but the end marker sits after main
        Given an install.sh whose `# --- main` line was moved to the end
        When the dogfood script runs
        Then it exits 2, because the slice carries the top-level asset_url
        assignment and the DOWNLOAD_AND_EXTRACT call.
        """
        lines = INSTALL_SCRIPT.read_text().splitlines(keepends=True)
        marker_line = next(ln for ln in lines if ln.startswith(END_MARKER))
        body = "".join(ln for ln in lines if ln is not marker_line) + marker_line
        result = _run(tmp_path, body)
        assert result.returncode == 2, result.stdout
        assert "main body" in result.stderr
        assert "SELECT_ASSET_FROM_JSON undefined" not in result.stderr

    @pytest.mark.unit
    def test_intact_markers_are_accepted(self, tmp_path):
        """
        Scenario: the real install.sh
        Given install.sh with both markers where they belong
        When the dogfood script runs
        Then it does not exit 2, so the guards do not fire on a good slice.
        """
        result = _run(tmp_path, INSTALL_SCRIPT.read_text())
        assert result.returncode != 2, result.stderr
        assert "marker" not in result.stderr
