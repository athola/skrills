"""Every shell script in the repo follows the shared shell rules.

The rules (logging preamble, logic in functions, `main "$@"` last, no `echo`,
help and xtrace flags) only hold if something checks them. These tests check
the parts a linter cannot: they read each script's syntax tree through
`shfmt --to-json`, so comments, heredoc bodies and quoted strings that merely
mention `echo` do not count as calls.
"""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_DIR = REPO_ROOT / "scripts"
LOGGING = SCRIPTS_DIR / "logging.sh"
GATE = SCRIPTS_DIR / "shellcheck.sh"
EXECUTABLES = sorted(p for p in SCRIPTS_DIR.glob("*.sh") if p.name != "logging.sh")
IDS = [str(p.relative_to(REPO_ROOT)) for p in EXECUTABLES]

needs_shfmt = pytest.mark.skipif(shutil.which("shfmt") is None, reason="shfmt not installed")


def _dialect(script: Path) -> str:
    return "bash" if "bash" in script.read_text().splitlines()[0] else "posix"


def _ast(script: Path) -> dict[str, Any]:
    completed = subprocess.run(
        ["shfmt", "--to-json", "-ln", _dialect(script)],
        input=script.read_text(), capture_output=True, text=True, check=True,
    )
    return json.loads(completed.stdout)


def _first_word(call: dict[str, Any]) -> str | None:
    args = call.get("Args") or []
    if not args:
        return None
    parts = args[0].get("Parts") or []
    if len(parts) == 1 and parts[0].get("Type") == "Lit":
        return parts[0]["Value"]
    return None


def _calls(node: Any, in_usage: bool = False) -> Iterator[tuple[str, bool]]:
    """Yield (command name, inside usage()) for every simple command."""
    if isinstance(node, dict):
        if node.get("Type") == "FuncDecl" and node["Name"]["Value"] == "usage":
            in_usage = True
        if node.get("Type") == "CallExpr":
            word = _first_word(node)
            if word is not None:
                yield word, in_usage
        for child in node.values():
            yield from _calls(child, in_usage)
    elif isinstance(node, list):
        for child in node:
            yield from _calls(child, in_usage)


def test_logging_library_is_sourced_not_executed() -> None:
    mode = LOGGING.stat().st_mode
    assert not mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    assert "__logging_loaded" in LOGGING.read_text()


def _log(snippet: str, **env: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["sh", "-c", f'set -eu; . "{LOGGING}"; {snippet}; printf after'],
        env={**os.environ, **env}, capture_output=True, text=True,
    )


def test_log_writes_levels_at_or_below_threshold_to_stderr() -> None:
    completed = _log('log 3 "an error"; log 6 "some info"')
    assert completed.returncode == 0
    assert completed.stderr == "an error\nsome info\n"
    assert completed.stdout == "after"


def test_log_drops_debug_by_default_without_failing_the_caller() -> None:
    completed = _log('log 7 "noisy detail"')
    assert completed.returncode == 0, completed.stderr
    assert completed.stderr == ""
    assert completed.stdout == "after"


def test_log_level_raises_the_threshold() -> None:
    completed = _log('log 7 "noisy detail"', LOG_LEVEL="7")
    assert completed.stderr == "noisy detail\n"


def test_log_without_a_level_logs_the_whole_message_at_info() -> None:
    completed = _log('log "Usage: x [-h]"')
    assert completed.stderr == "Usage: x [-h]\n"
    assert _log('log "Usage: x"', LOG_LEVEL="5").stderr == ""


def test_banner_frames_the_message_on_stdout() -> None:
    completed = _log('banner "Results: 1 passed"')
    sep = "=" * 40
    assert completed.stdout == f"{sep}\nResults: 1 passed\n{sep}\nafter"


@needs_shfmt
@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not installed")
def test_every_script_passes_the_shell_gate() -> None:
    completed = subprocess.run([str(GATE)], cwd=REPO_ROOT / "tests", capture_output=True, text=True)
    assert completed.returncode == 0, completed.stdout + completed.stderr


@needs_shfmt
@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not installed")
def test_shell_gate_rejects_a_misformatted_script(tmp_path: Path) -> None:
    bad = tmp_path / "bad.sh"
    bad.write_text("#!/usr/bin/env bash\nmain() {\n    true\n}\nmain \"$@\"\n")
    completed = subprocess.run([str(GATE), str(bad)], capture_output=True, text=True)
    assert completed.returncode != 0
    assert "bad.sh" in completed.stdout + completed.stderr


@pytest.mark.parametrize("script", EXECUTABLES, ids=IDS)
def test_executable_ends_by_calling_main(script: Path) -> None:
    assert script.read_text().rstrip("\n").splitlines()[-1] == 'main "$@"'


@needs_shfmt
@pytest.mark.parametrize("script", EXECUTABLES, ids=IDS)
def test_executable_runs_no_logic_at_top_level(script: Path) -> None:
    # POSIX mode parses `readonly` as a plain command, not a declaration.
    allowed_calls = {"set", ".", "main", "readonly"}
    for stmt in _ast(script)["Stmts"]:
        cmd = stmt["Cmd"]
        kind = cmd["Type"]
        if kind in ("FuncDecl", "DeclClause"):
            continue
        assert kind == "CallExpr", f"top-level {kind} at line {stmt['Pos']['Line']}"
        word = _first_word(cmd)
        assert word is None or word in allowed_calls, (
            f"top-level `{word}` at line {stmt['Pos']['Line']}"
        )


@needs_shfmt
@pytest.mark.parametrize("script", EXECUTABLES, ids=IDS)
def test_executable_never_calls_echo_dirname_or_basename(script: Path) -> None:
    banned = {"echo", "dirname", "basename"}
    found = [word for word, _ in _calls(_ast(script)) if word in banned]
    assert not found, found


@pytest.mark.parametrize("flag", ["-h", "--help"])
@pytest.mark.parametrize("script", EXECUTABLES, ids=IDS)
def test_executable_prints_usage_and_exits_zero(script: Path, flag: str, tmp_path: Path) -> None:
    # A HOME and PATH-free env keep a script that ignored the flag from
    # touching anything real before the assertion catches it.
    env = {"PATH": os.environ["PATH"], "HOME": str(tmp_path)}
    shell = "bash" if _dialect(script) == "bash" else "sh"
    completed = subprocess.run(
        [shell, str(script), flag], cwd=tmp_path, env=env,
        capture_output=True, text=True, timeout=30,
    )
    assert completed.returncode == 0, completed.stderr
    assert "Usage:" in completed.stderr


@pytest.mark.parametrize("script", EXECUTABLES, ids=IDS)
def test_executable_traces_with_x(script: Path, tmp_path: Path) -> None:
    env = {"PATH": os.environ["PATH"], "HOME": str(tmp_path)}
    shell = "bash" if _dialect(script) == "bash" else "sh"
    completed = subprocess.run(
        [shell, str(script), "-x", "-h"], cwd=tmp_path, env=env,
        capture_output=True, text=True, timeout=30,
    )
    assert completed.returncode == 0, completed.stderr
    assert "+ usage" in completed.stderr


def test_makefile_shell_gate_runs_the_gate_script_and_checks_for_shfmt() -> None:
    makefile = (REPO_ROOT / "Makefile").read_text()
    recipe = makefile.split("\nlint-shell:\n", 1)[1].split("\n\n", 1)[0]
    assert "./scripts/shellcheck.sh" in recipe
    check_deps = makefile.split("\ncheck-deps:\n", 1)[1].split("\n\n", 1)[0]
    assert "shfmt" in check_deps


def test_ci_hygiene_job_installs_shfmt() -> None:
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yml").read_text()
    job = workflow.split("\n  hygiene-and-scripts:\n", 1)[1].split("\n  dogfood:\n", 1)[0]
    assert "shfmt" in job
