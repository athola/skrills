# Skill Analysis

Skrills analyzes skills for token usage, dependencies, and optimization opportunities.

## Token Analysis

### Basic Analysis

Analyze all discovered skills:

```bash
skrills analyze
```

### Filter by Token Count

Show only skills exceeding a token threshold:

```bash
skrills analyze --min-tokens 1000
```

### Include Optimization Suggestions

Get actionable recommendations for reducing token usage:

```bash
skrills analyze --suggestions
```

### Output Formats

Get machine-readable output:

```bash
skrills analyze --format json
```

### Analyze Specific Directories

Override default discovery paths:

```bash
skrills analyze --skill-dir ~/my-skills
```

## Understanding Token Counts

Token counts estimate context impact, helping you:
1. **Budget context**: Large skills consume available context.
2. **Target optimization**: Identify candidates for refactoring.
3. **Compare alternatives**: Select efficient skill variants.

## Optimization Suggestions

The `--suggestions` flag flags potential issues:
- **Split large skills**: Skills over 2000 tokens may benefit from modular decomposition.
- **Remove redundant content**: Eliminate duplication.
- **Simplify examples**: Condense verbose examples.
- **Use references**: Link to external docs instead of embedding content.

## MCP Tool

When running as an MCP server (`skrills serve`), the `analyze-skills` tool provides the same functionality:

```json
{
  "name": "analyze-skills",
  "arguments": {
    "min_tokens": 1000,
    "suggestions": true
  }
}
```

## Best Practices

1. **Set budgets**: Establish guidelines for maximum skill token counts.
2. **Review regularly**: Run analysis after adding or updating skills.
3. **Prioritize**: Focus optimization on frequently used skills.
4. **Test**: Verify skills function correctly after refactoring.
