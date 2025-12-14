# Skill Analysis

Skrills analyzes skills for token usage, dependencies, and optimization opportunities. This helps you understand the context window impact of your skills and identify candidates for optimization.

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

Token counts are estimates based on the skill content. They help you:

1. **Budget context window usage**: Large skills consume more of the available context.
2. **Identify optimization targets**: Skills with high token counts may benefit from refactoring.
3. **Compare alternatives**: Choose between skill variants based on efficiency.

## Optimization Suggestions

When using `--suggestions`, skrills provides recommendations such as:

- **Split large skills**: Skills over 2000 tokens may benefit from modular decomposition.
- **Remove redundant content**: Duplicate information across skills.
- **Simplify examples**: Verbose examples can often be condensed.
- **Use references**: Link to external docs instead of embedding large content.

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

1. **Set token budgets**: Establish team guidelines for maximum skill token counts.
2. **Review regularly**: Run analysis after adding new skills or updating existing ones.
3. **Prioritize high-impact skills**: Focus optimization efforts on frequently-used skills.
4. **Test after optimization**: Ensure skills still work correctly after reducing content.
