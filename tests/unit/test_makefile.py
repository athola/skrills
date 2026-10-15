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


def _help() -> str:
    return subprocess.run(
        ["make", "help"], cwd=REPO_ROOT, capture_output=True, text=True, check=True
    ).stdout


def test_check_deps_reports_the_tools_precommit_and_dogfood_call() -> None:
    recipe = _dry_run("check-deps")
    for tool in ("npx", "rg", "jq", "curl", "tmux", "python3", "pytest", "gtimeout", "shellcheck"):
        assert tool in recipe, tool
    assert "declare -A" in recipe  # bash 4+ probe for verify-publish


def test_integration_tests_follow_the_thread_setting() -> None:
    recipe = _dry_run("test-integration", "TEST_THREADS=3")
    assert "--test-threads=3" in recipe


def test_help_states_the_make_version_parallel_runs_need() -> None:
    assert "GNU make 4.4" in _help()


def _prerequisites(target: str) -> list[str]:
    text = (REPO_ROOT / "Makefile").read_text()
    line = next(l for l in text.splitlines() if l.startswith(f"{target}:"))
    return line.split(":", 1)[1].split()


def test_release_checks_formatting_instead_of_rewriting_it() -> None:
    assert "fmt" not in _prerequisites("release")
    assert "fmt --all -- --check" in _dry_run("release")


def test_all_does_not_format_in_parallel_with_lint_and_tests() -> None:
    """`fmt` as a sibling prerequisite of lint/test lets `make -j` run clippy
    and tests against a half-formatted tree."""
    assert "fmt" not in _prerequisites("all")
    assert "fmt --all\n" in _dry_run("all")


def test_help_lists_targets_it_used_to_omit() -> None:
    help_text = _help()
    for target in ("cold-window", "lint-reachability", "verify-publish", "lint-shell"):
        assert f"  {target} " in help_text, target
    hygiene = next(l for l in help_text.splitlines() if l.strip().startswith("lint-hygiene"))
    assert "lint-reachability" in hygiene and "lint-shell" in hygiene
    lint = next(l for l in help_text.splitlines() if l.strip().startswith("lint "))
    assert "three" in lint


def test_ensure_mdbook_finds_a_copy_it_installed_earlier() -> None:
    recipe = _dry_run("book", "CARGO_HOME=/ch")
    assert "PATH=/ch/bin:$PATH command -v mdbook" in recipe


def test_install_puts_the_binary_in_the_users_cargo_bin() -> None:
    recipe = _dry_run("install")
    assert "cargo install --path crates/cli --locked" in recipe
    assert "CARGO_HOME=" not in recipe


@pytest.mark.parametrize(
    "target", ["lint-md", "lint-prose", "lint-decoration", "lint-reachability",
               "dogfood-validate-contract", "dogfood-tui-interactive"]
)
def test_scripts_run_through_their_shebang(target: str) -> None:
    assert "/bin/bash ./scripts/" not in _dry_run(target)


def test_recipe_pipelines_fail_when_an_early_stage_fails(tmp_path: Path) -> None:
    probe = tmp_path / "probe.mk"
    probe.write_text("pipefail-probe:\n\tfalse | cat\n")
    completed = subprocess.run(
        ["make", "-f", "Makefile", "-f", str(probe), "pipefail-probe"],
        cwd=REPO_ROOT, capture_output=True, text=True,
    )
    assert completed.returncode != 0


def test_cli_demo_asserts_the_expected_errors() -> None:
    recipe = _dry_run("demo-cli")
    assert '|| true' not in "\n".join(
        l for l in recipe.splitlines() if "grep -q" in l
    )
    assert 'grep -q "No skills"' not in recipe


def test_cert_demo_does_not_hide_failures_behind_head() -> None:
    assert "| head -5" not in _dry_run("demo-cert")


def test_uninstall_demo_checks_the_registration_is_gone() -> None:
    recipe = _dry_run("demo-setup-uninstall")
    assert ".claude/.mcp.json" in recipe.split("setup --uninstall", 1)[1]


def test_setup_both_demo_checks_codex_too() -> None:
    recipe = _dry_run("demo-setup-both").split("setup --client both", 1)[1]
    assert ".codex/config.toml" in recipe


def test_dogfood_scratch_paths_are_private_and_unpredictable() -> None:
    for target in ("dogfood-cold-window-browser", "dogfood-tui", "dogfood-dashboard",
                   "dogfood-skill-diff", "plugin-modernize-direct"):
        recipe = _dry_run(target)
        assert "/tmp/skrills-dogfood-" not in recipe, target
        assert "mktemp -d" in recipe, target


def test_lint_hygiene_runs_shellcheck() -> None:
    assert "shellcheck" in _dry_run("lint-hygiene")


@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="shellcheck not installed")
def test_shell_scripts_have_no_shellcheck_warnings() -> None:
    completed = subprocess.run(["make", "lint-shell"], cwd=REPO_ROOT, capture_output=True, text=True)
    assert completed.returncode == 0, completed.stdout + completed.stderr
