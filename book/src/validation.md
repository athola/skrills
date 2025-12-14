# Skill Validation

Skrills validates skills for compatibility with Claude Code and Codex CLI. The two CLIs have different requirements for skill frontmatter, and skrills helps ensure your skills work across both.

## Validation Targets

### Claude Code (Permissive)

Claude Code accepts any markdown file as a skill. Frontmatter is optional and can contain any fields.

### Codex CLI (Strict)

Codex CLI requires YAML frontmatter with specific fields:
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

| Issue | Cause | Fix |
|-------|-------|-----|
| Missing frontmatter | No YAML block at start | Use `--autofix` or add manually |
| Missing `name` | Frontmatter lacks name field | Add `name:` to frontmatter |
| Missing `description` | Frontmatter lacks description | Add `description:` to frontmatter |
| Name too long | Exceeds 100 characters | Shorten the name |
| Description too long | Exceeds 500 characters | Condense the description |

## Best Practices

1. **Write for Codex first**: If you follow Codex requirements, your skills work everywhere.
2. **Run validation in CI**: Add `skrills validate --target both --errors-only` to your pipeline.
3. **Use autofix carefully**: Review changes before committing, especially for description generation.
4. **Validate after sync**: Run validation after `skrills sync` to catch issues early.
