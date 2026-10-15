"""scripts/publish_crates.sh skips versions already on crates.io by asking
for the exact version, and does not sleep between publishes."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "publish_crates.sh"

pytestmark = pytest.mark.skipif(shutil.which("jq") is None, reason="jq not installed")


def _shim(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)


def test_publishes_only_versions_crates_io_lacks(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    log = tmp_path / "calls.log"
    crates = [line.split()[1] for line in SCRIPT.read_text().splitlines()
              if line.startswith("publish_one ")]
    metadata = {"packages": [{"name": c, "version": "9.9.9"} for c in crates]}
    (tmp_path / "metadata.json").write_text(json.dumps(metadata))
    _shim(bin_dir, "cargo", f"""case "$1" in
  metadata) cat '{tmp_path}/metadata.json' ;;
  publish) echo "publish $3" >> '{log}' ;;
  search) echo "search $2" >> '{log}'; echo '' ;;
esac
""")
    # crates.io knows skrills-snapshot 9.9.9 only.
    _shim(bin_dir, "curl", f"""for arg; do last="$arg"; done
echo "curl $last" >> '{log}'
case "$last" in
  */crates/skrills-snapshot/9.9.9) printf 200 ;;
  *) printf 404 ;;
esac
""")
    _shim(bin_dir, "sleep", f"echo sleep >> '{log}'\n")
    env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
           "CARGO_REGISTRY_TOKEN": "test"}
    completed = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True)
    assert completed.returncode == 0, completed.stderr
    calls = log.read_text().splitlines()
    assert "publish skrills-snapshot" not in calls
    assert "publish skrills" in calls
    assert "sleep" not in calls
    assert not any(c.startswith("search") for c in calls)
