"""scripts/verify_publish_order.sh: bash version guard and manifest forms."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "verify_publish_order.sh"


def _bash4() -> str | None:
    for candidate in ("bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash"):
        path = shutil.which(candidate)
        if path and subprocess.run([path, "-c", "declare -A probe"], capture_output=True).returncode == 0:
            return path
    return None


def _old_bash() -> str | None:
    path = "/bin/bash"
    if not Path(path).exists():
        return None
    probe = subprocess.run([path, "-c", "declare -A probe"], capture_output=True)
    return path if probe.returncode != 0 else None


@pytest.mark.skipif(_old_bash() is None, reason="no bash older than 4 to probe with")
def test_old_bash_gets_a_clear_message() -> None:
    completed = subprocess.run([_old_bash(), str(SCRIPT)], cwd=REPO_ROOT, capture_output=True, text=True)
    assert completed.returncode == 1
    assert "bash 4" in completed.stderr


def _crate(root: Path, name: str, extra: str = "") -> None:
    crate = root / "crates" / name
    (crate / "src").mkdir(parents=True)
    (crate / "src" / "lib.rs").write_text("")
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "0.1.0"\n{extra}\n'
    )


@pytest.mark.skipif(_bash4() is None, reason="bash 4+ not installed")
@pytest.mark.parametrize(
    "unpublished",
    ["publish = []", "publish.workspace = true"],
)
def test_glob_members_and_unpublished_forms_are_understood(tmp_path: Path, unpublished: str) -> None:
    (tmp_path / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\npublish = false\n'
    )
    _crate(tmp_path, "alpha")
    _crate(tmp_path, "internal", unpublished)
    scripts = tmp_path / "scripts"
    scripts.mkdir()
    (scripts / "publish_crates.sh").write_text("publish_one alpha\n")
    shutil.copy2(SCRIPT, scripts / SCRIPT.name)
    shutil.copy2(SCRIPT.parent / "logging.sh", scripts / "logging.sh")
    completed = subprocess.run(
        [_bash4(), str(scripts / SCRIPT.name)], cwd=tmp_path, capture_output=True, text=True
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
