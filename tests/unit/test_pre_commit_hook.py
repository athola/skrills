"""The pre-commit hook must not hand git's per-repo variables to `make`.

In a linked worktree git exports GIT_DIR and GIT_INDEX_FILE to hooks. Tests
under `make precommit` that run `git init`/`git commit` in temp dirs then
acted on the real repository: they staged mass deletions and set
core.bare=true on the shared config.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
HOOK = REPO_ROOT / "githooks" / "pre-commit"
LEAKED = ("GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE")


def test_hook_clears_git_repo_variables_before_make(tmp_path: Path) -> None:
    seen = tmp_path / "seen-env"
    stub_dir = tmp_path / "bin"
    stub_dir.mkdir()
    stub = stub_dir / "make"
    stub.write_text(f'#!/bin/sh\nenv > "{seen}"\n')
    stub.chmod(0o755)
    env = {
        **os.environ,
        "PATH": f"{stub_dir}:{os.environ['PATH']}",
        "GIT_DIR": str(REPO_ROOT / ".git"),
        "GIT_INDEX_FILE": str(REPO_ROOT / ".git" / "index"),
        "GIT_WORK_TREE": str(REPO_ROOT),
    }

    subprocess.run(["bash", str(HOOK)], cwd=REPO_ROOT, env=env, check=True,
                   capture_output=True)

    names = {line.split("=", 1)[0] for line in seen.read_text().splitlines()}
    assert not names & set(LEAKED), names & set(LEAKED)
