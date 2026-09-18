"""BDD characterization tests for scripts/lint-module-reachability.sh.

These tests lock the "every crate source file is declared" contract so
the gate keeps failing on an orphaned .rs file and keeps passing on each
declaration form rustc accepts.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "lint-module-reachability.sh"


def _run(cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT)],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )


def _crate_src(tmp_path: Path, lib_rs: str) -> Path:
    src = tmp_path / "crates" / "demo" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text(lib_rs)
    return src


class TestModuleReachabilityLint:
    """
    Feature: lint-module-reachability blocks undeclared Rust source files

    As a Rust maintainer
    I want CI to fail when a file under crates/*/src has no `mod` item
    So that dead files never sit uncompiled while reading like live code.
    """

    @pytest.mark.unit
    def test_undeclared_file_fails_and_is_named(self, tmp_path):
        """
        Scenario: a source file no `mod` item names
        Given crates/demo/src/orphan.rs with no matching `mod orphan;`
        When I run the lint
        Then it exits 1 and lists the orphan path on stderr.
        """
        src = _crate_src(tmp_path, "pub fn live() {}\n")
        (src / "orphan.rs").write_text("pub fn dead() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 1
        assert "not reachable" in result.stderr
        assert "crates/demo/src/orphan.rs" in result.stderr

    @pytest.mark.unit
    def test_plain_mod_declaration_passes(self, tmp_path):
        """
        Scenario: file declared with `mod foo;`
        Given crates/demo/src/foo.rs and `mod foo;` in lib.rs
        When I run the lint
        Then it exits 0 and reports the tree clean.
        """
        src = _crate_src(tmp_path, "mod foo;\n")
        (src / "foo.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
        assert "clean" in result.stdout

    @pytest.mark.unit
    def test_restricted_visibility_mod_declaration_passes(self, tmp_path):
        """
        Scenario: file declared with `pub(crate) mod foo;`
        Given the declaration carries a visibility qualifier
        When I run the lint
        Then it exits 0, because the optional visibility group matches.
        """
        src = _crate_src(tmp_path, "pub(crate) mod foo;\n")
        (src / "foo.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_path_attribute_declaration_passes(self, tmp_path):
        """
        Scenario: file compiled through `#[path = "..."]`
        Given lib.rs declares `#[path = "foo_impl.rs"] mod foo;`
        When I run the lint
        Then it exits 0, because a path attribute also compiles the file.
        """
        src = _crate_src(tmp_path, '#[path = "foo_impl.rs"]\nmod foo;\n')
        (src / "foo_impl.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_directory_module_passes_via_parent_stem(self, tmp_path):
        """
        Scenario: directory module foo/mod.rs
        Given crates/demo/src/foo/mod.rs and `mod foo;` in lib.rs
        When I run the lint
        Then it exits 0, because mod.rs takes its stem from its directory.
        """
        src = _crate_src(tmp_path, "mod foo;\n")
        (src / "foo").mkdir()
        (src / "foo" / "mod.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_crate_roots_are_exempt(self, tmp_path):
        """
        Scenario: lib.rs and main.rs alone
        Given a crate with only its root files and no declarations
        When I run the lint
        Then it exits 0, because roots need no `mod` item.
        """
        src = _crate_src(tmp_path, "")
        (src / "main.rs").write_text("fn main() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
