"""Characterization tests for Makefile recipes that `make -n` can check.

These run make in dry-run mode only, so no target executes; they check the
shell the recipes expand to and the paths they use.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]

pytestmark = pytest.mark.skipif(
    shutil.which("make") is None, reason="make not installed"
)


def _dry_run(*args: str) -> str:
    completed = subprocess.run(
        ["make", "-n", *args],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return completed.stdout


def test_test_coverage_recipe_is_valid_shell() -> None:
    """The browser-opening macro was spliced inside the recipe's `if`, so the
    expanded recipe was a bash syntax error and coverage never ran."""
    recipe = _dry_run("test-coverage")
    checked = subprocess.run(
        ["bash", "-n"], input=recipe, capture_output=True, text=True
    )
    assert checked.returncode == 0, checked.stderr


def test_bin_path_follows_cargo_target_dir() -> None:
    """`cargo build` writes under CARGO_TARGET_DIR, so a BIN_PATH fixed to
    ./target failed dogfood-precommit on every commit when it was set."""
    recipe = _dry_run("dogfood-precommit", "CARGO_TARGET_DIR=/elsewhere")
    assert "/elsewhere/release/skrills" in recipe
    assert "target/release/skrills" not in recipe


def test_demo_fixtures_link_the_configured_binary() -> None:
    recipe = _dry_run("demo-fixtures", "BIN_PATH=/custom/skrills")
    assert "ln -sf /custom/skrills" in recipe


def test_demo_multi_cli_agent_tests_the_crate_that_holds_them() -> None:
    """multi_cli_agent lives in crates/cli; filtering skrills-server ran zero
    tests and still reported a pass."""
    recipe = _dry_run("demo-multi-cli-agent")
    assert "--package skrills --lib multi_cli_agent" in recipe
    assert "--package skrills-server --lib multi_cli_agent" not in recipe


def test_ci_checks_formatting_instead_of_rewriting_it() -> None:
    """`make ci` ran `cargo fmt`, so a local run passed a tree CI rejects."""
    recipe = _dry_run("ci")
    assert "fmt --all -- --check" in recipe
    assert "fmt --all\n" not in recipe


def test_ci_runs_the_script_and_install_gates() -> None:
    recipe = _dry_run("ci")
    assert "tests/unit" in recipe
    assert "./scripts/test-install.sh" in recipe
    assert "lint-markdown.sh" in recipe


@pytest.mark.parametrize(
    "target",
    [
        "dogfood-cold-window-headless",
        "dogfood-cold-window-chaos",
        "dogfood-tui",
        "dogfood-dashboard",
    ],
)
def test_dogfood_timeout_command_is_overridable(target: str) -> None:
    """Stock macOS has no GNU `timeout`; Homebrew installs it as `gtimeout`."""
    recipe = _dry_run(target, "TIMEOUT=/opt/x/gtimeout")
    assert "/opt/x/gtimeout --signal=TERM 3" in recipe


@pytest.mark.parametrize(
    "target",
    [
        "dogfood-cold-window-headless",
        "dogfood-cold-window-chaos",
        "dogfood-tui",
        "dogfood-dashboard",
    ],
)
def test_dogfood_without_timeout_fails_with_install_hint(target: str) -> None:
    """Without a timeout command the recipe must stop, not run the binary
    unbounded or report exit 127 as a contract failure."""
    recipe = _dry_run(target, "TIMEOUT=")
    assert "brew install coreutils" in recipe
