# Skill Validation

Skrills validates skills for compatibility with Claude Code and Codex CLI. The two CLIs have different requirements for skill frontmatter, and Skrills checks that your skills work across both environments.

## Validation Targets

### Claude Code (Permissive)

Claude Code accepts any markdown file as a skill. Frontmatter is optional and can contain any fields.

### Codex CLI (Strict)

Codex CLI discovers skills only from files named exactly `SKILL.md` under `~/.codex/skills/**/` (recursive, symlinks and hidden entries are skipped).

Codex CLI also requires YAML frontmatter with specific fields:
- `name`: Required, max 100 characters
- `description`: Required, max 500 characters

Skills without proper frontmatter will fail to load in Codex CLI.

## Using the Validator

### Basic Validation

Validate all discovered skills against both targets:

```bash
skrills validate
```

Validate for a specific target:

```bash
skrills validate --target codex    # Strict Codex rules
skrills validate --target claude   # Permissive Claude rules
skrills validate --target both     # Both (default)
```

### Auto-Fix Missing Frontmatter

The `--autofix` flag automatically adds missing frontmatter by deriving values from the file path and content:

```bash
skrills validate --target codex --autofix
```

For safety, create backups before modifying files:

```bash
skrills validate --target codex --autofix --backup
```

### Output Formats

Get machine-readable output for CI/CD pipelines:

```bash
skrills validate --format json
```

Show only skills with validation errors:

```bash
skrills validate --errors-only
```

### Validate Specific Directories

Override the default discovery paths:

```bash
skrills validate --skill-dir ~/my-skills --skill-dir ~/other-skills
```

## MCP Tool

When running as an MCP server (`skrills serve`), the `validate-skills` tool provides the same functionality:

```json
{
  "name": "validate-skills",
  "arguments": {
    "target": "codex",
    "autofix": true
  }
}
```

## Common Validation Issues

Common validation failures often stem from missing YAML frontmatter, specifically the `name` or `description` fields required by Codex. Other issues include names exceeding 100 characters or descriptions longer than 500 characters. You can resolve most of these automatically with the `--autofix` flag, or by manually adding the required fields.

## Best Practices

Write skills with Codex requirements in mind; if they pass Codex validation, they will work everywhere. Integrate validation into your CI pipeline using `skrills validate --target both --errors-only` to catch issues early. When using `--autofix`, review the changes before committing, especially since generated descriptions might need manual refinement. Finally, run validation after syncing to verify that no incompatible changes were introduced.
