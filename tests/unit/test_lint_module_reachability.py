"""BDD characterization tests for scripts/lint-module-reachability.sh.

These tests lock the "every crate source file is declared" contract so
the gate keeps failing on an orphaned .rs file and keeps passing on each
declaration form rustc accepts.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "lint-module-reachability.sh"


def _run(tree: Path, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    """Run a copy of the lint whose parent directory is `tree`.

    The lint anchors on its own location instead of the caller's cwd, so a
    fixture tree is exercised by placing a copy of the script in it.
    """
    scripts = tree / "scripts"
    scripts.mkdir(exist_ok=True)
    copied = scripts / SCRIPT.name
    shutil.copy2(SCRIPT, copied)
    return subprocess.run(
        ["bash", str(copied)],
        cwd=cwd or tree,
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

    @pytest.mark.unit
    def test_stem_declared_in_a_sibling_module_does_not_satisfy_the_check(
        self, tmp_path
    ):
        """
        Scenario: two directories reuse a module stem
        Given alpha/mod.rs declares `mod shared;` and beta/mod.rs declares none
        And both alpha/shared.rs and beta/shared.rs exist
        When I run the lint
        Then it exits 1 naming only beta/shared.rs, because a declaration in a
        different module does not compile beta's file.
        """
        src = _crate_src(tmp_path, "mod alpha;\nmod beta;\n")
        (src / "alpha").mkdir()
        (src / "alpha" / "mod.rs").write_text("mod shared;\n")
        (src / "alpha" / "shared.rs").write_text("pub fn a() {}\n")
        (src / "beta").mkdir()
        (src / "beta" / "mod.rs").write_text("pub fn b() {}\n")
        (src / "beta" / "shared.rs").write_text("pub fn dead() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 1
        assert "crates/demo/src/beta/shared.rs" in result.stderr
        assert "crates/demo/src/alpha/shared.rs" not in result.stderr

    @pytest.mark.unit
    def test_sibling_file_module_declares_its_directory_children(self, tmp_path):
        """
        Scenario: 2018-edition `foo.rs` beside `foo/`
        Given crates/demo/src/foo.rs declares `mod inner;`
        And crates/demo/src/foo/inner.rs exists
        When I run the lint
        Then it exits 0, because foo.rs is the parent module of foo/.
        """
        src = _crate_src(tmp_path, "mod foo;\n")
        (src / "foo.rs").write_text("mod inner;\n")
        (src / "foo").mkdir()
        (src / "foo" / "inner.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_tree_with_no_crate_sources_exits_two(self, tmp_path):
        """
        Scenario: the crates/*/src glob matches nothing
        Given a tree with no crates/ directory at all
        When I run the lint
        Then it exits 2 instead of reporting the tree clean, because an
        unmatched glob used to disable the gate silently.
        """
        result = _run(tmp_path)
        assert result.returncode == 2, result.stdout
        assert "clean" not in result.stdout

    @pytest.mark.unit
    def test_orphan_is_found_when_invoked_from_an_unrelated_cwd(self, tmp_path):
        """
        Scenario: the lint runs from a directory outside the tree
        Given an orphan under crates/demo/src
        When I run the lint with the cwd set elsewhere
        Then it still exits 1, because the lint anchors on its own location.
        """
        src = _crate_src(tmp_path, "pub fn live() {}\n")
        (src / "orphan.rs").write_text("pub fn dead() {}\n")
        elsewhere = tmp_path.parent / f"{tmp_path.name}-cwd"
        elsewhere.mkdir()
        result = _run(tmp_path, cwd=elsewhere)
        assert result.returncode == 1
        assert "crates/demo/src/orphan.rs" in result.stderr

    @pytest.mark.unit
    def test_path_attribute_with_a_subdirectory_value_passes(self, tmp_path):
        """
        Scenario: `#[path = "gen/impl_a.rs"]`
        Given the attribute value carries a directory component
        When I run the lint
        Then it exits 0, because the value is matched as a path suffix rather
        than as a bare basename.
        """
        src = _crate_src(tmp_path, '#[path = "gen/impl_a.rs"]\nmod a;\n')
        (src / "gen").mkdir()
        (src / "gen" / "impl_a.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_path_attribute_without_spaces_passes(self, tmp_path):
        """
        Scenario: `#[path="foo_impl.rs"]` written without spaces
        Given rustc accepts the attribute either way
        When I run the lint
        Then it exits 0, because the pattern tolerates the missing spaces.
        """
        src = _crate_src(tmp_path, '#[path="foo_impl.rs"]\nmod foo;\n')
        (src / "foo_impl.rs").write_text("pub fn f() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr

    @pytest.mark.unit
    def test_src_bin_targets_need_no_mod_declaration(self, tmp_path):
        """
        Scenario: an auto-discovered binary target
        Given crates/demo/src/bin/tool.rs and no `mod tool;` anywhere
        When I run the lint
        Then it exits 0, because Cargo compiles src/bin/*.rs as its own target.
        """
        src = _crate_src(tmp_path, "pub fn live() {}\n")
        (src / "bin").mkdir()
        (src / "bin" / "tool.rs").write_text("fn main() {}\n")
        result = _run(tmp_path)
        assert result.returncode == 0, result.stderr
