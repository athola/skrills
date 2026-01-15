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

Token counts provide an estimate of a skill's impact on your context window. This helps you budget available context, identify candidates for refactoring, and compare efficient alternatives. Large skills can displace other important context, so keeping them lean is critical for performance.

## Optimization Suggestions

The `--suggestions` flag identifies potential issues that bloat your context usage. It looks for skills exceeding 2000 tokens that might benefit from modular decomposition or removal of redundant content. It also flags verbose examples that could be simplified and suggests linking to external documentation instead of embedding large blocks of text.

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

Establish a budget for maximum skill token counts and review them regularly, especially after major updates. Focus your optimization efforts on frequently used skills where the savings will have the most impact. Always verify that skills still function correctly after any refactoring to reduce size.
