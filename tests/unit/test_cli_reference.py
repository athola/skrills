"""Keep book/src/cli.md in step with the clap definitions.

The reference had drifted to cover about half the subcommands, so these
tests read `enum Commands` in crates/cli/src/cli/mod.rs and fail when a
subcommand, or a `serve` flag, is not documented.
"""

from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CLI_SOURCE = REPO_ROOT / "crates" / "cli" / "src" / "cli" / "mod.rs"
CLI_REFERENCE = REPO_ROOT / "book" / "src" / "cli.md"

VARIANT = re.compile(r"^    ([A-Z][A-Za-z]*)\s*(?:\{|\(|,)")
FIELD = re.compile(r"^\s*([a-z_][a-z0-9_]*)\s*:")


def _kebab(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower().replace("_", "-")


def _commands_block() -> list[str]:
    lines = CLI_SOURCE.read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith("pub enum Commands"))
    end = next(i for i in range(start, len(lines)) if lines[i] == "}")
    return lines[start + 1 : end]


def _subcommands() -> list[str]:
    block = _commands_block()
    names = []
    for index, line in enumerate(block):
        match = VARIANT.match(line)
        if not match:
            continue
        attrs = " ".join(block[max(0, index - 3) : index])
        if "hide = true" in attrs:
            continue
        renamed = re.search(r'#\[command\([^)]*name = "([^"]+)"', attrs)
        names.append(renamed.group(1) if renamed else _kebab(match.group(1)))
    return names


def _serve_flags() -> list[str]:
    block = _commands_block()
    start = next(i for i, line in enumerate(block) if line.startswith("    Serve {"))
    end = next(i for i in range(start, len(block)) if block[i] == "    },")
    flags = []
    pending = ""
    collecting = False
    for line in block[start + 1 : end]:
        stripped = line.strip()
        if stripped.startswith("#[arg("):
            collecting = True
            pending = ""
        if collecting:
            pending += " " + stripped
            if stripped.endswith(")]"):
                collecting = False
            continue
        field = FIELD.match(line)
        if field and pending:
            explicit = re.search(r'long\s*=\s*"([^"]+)"', pending)
            if explicit:
                flags.append("--" + explicit.group(1))
            elif re.search(r"\blong\b", pending):
                flags.append("--" + _kebab(field.group(1)))
            pending = ""
    return flags


def _documented(name: str, reference: str) -> bool:
    heading = re.search(rf"^###+ {re.escape(name)}\s*$", reference, re.MULTILINE)
    usage = re.search(rf"skrills {re.escape(name)}(?![\w-])", reference)
    return bool(heading or usage)


def test_parser_finds_known_subcommands() -> None:
    names = _subcommands()
    assert "serve" in names
    assert "cold-window" in names
    assert "multi-cli-agent" in names


def test_every_subcommand_is_documented_in_cli_reference() -> None:
    reference = CLI_REFERENCE.read_text()
    missing = [name for name in _subcommands() if not _documented(name, reference)]
    assert missing == [], f"undocumented in book/src/cli.md: {missing}"


def test_every_serve_flag_is_documented_in_cli_reference() -> None:
    reference = CLI_REFERENCE.read_text()
    flags = _serve_flags()
    assert "--http" in flags
    missing = [flag for flag in flags if f"`{flag}" not in reference]
    assert missing == [], f"serve flags undocumented in book/src/cli.md: {missing}"


def test_cli_reference_examples_name_real_subcommands() -> None:
    """MCP tool names shown as `skrills <tool>` commands fail with
    "unrecognized subcommand" when a reader copies them."""
    known = set(_subcommands()) | {"sync-from-claude", "suggest-skills", "--version", "--help"}
    examples = re.findall(r"^skrills ([\w-]+)", CLI_REFERENCE.read_text(), re.MULTILINE)
    unknown = sorted({name for name in examples if name not in known})
    assert unknown == [], f"cli.md runs non-existent subcommands: {unknown}"
