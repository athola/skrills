# AI Agent Development Guidelines

Use these guidelines to build AI coding agents that produce functional, high-quality code.

---

## Guiding Principles

Prioritize small, functional changes. They are safer and easier to debug than large rewrites. Adapt to each project's conventions rather than applying external rules rigidly.

Balance trade-offs like performance, readability, and security based on evidence (profiling, metrics) rather than intuition. Explore multiple solutions before committing to one to avoid "mode collapse." Favor simple, standard solutions—code that requires extensive comments often needs refactoring.

Each component should serve one distinct purpose (Single Responsibility Principle). Defer abstraction until a clear pattern emerges (e.g., Rule of Three). Explicitly document assumptions, trade-offs, and confidence levels to maintain transparency.

---

## Development Workflow

### Implementation Cycle

1. **Understand**: Read existing code, identify patterns, and review tests.
2. **Explore**: Develop multiple viable approaches and articulate their trade-offs.
3. **Test**: Write a failing test case before implementing new code (when applicable).
4. **Implement**: Write the minimal code necessary to pass the test.
5. **Refactor**: Clean the code while ensuring tests pass.
6. **Commit**: Write a clear commit message explaining the change.

### When Stuck

If you fail three times:

1. Document failures and error outputs.
2. Investigate 2-3 alternative approaches.
3. Re-evaluate underlying assumptions.
4. Experiment with a simpler approach.
5. If the issue persists, ask for help with context from the previous steps.

### Session Management

Use session history to analyze errors and monitor progress. For complex tasks, document the current state, then clear the session and restart.

---

## Quality Standards

### Commit Requirements

Every commit must:

- Compile or build successfully.
- Pass all existing tests.
- Include tests for new functionality.
- Follow linting rules (no warnings).
- Include a clear message explaining the rationale.

### Pre-Commit Workflow

Run this before committing:

```bash
make fmt lint test --quiet build
```

### Effective Prompting

Structured prompts produce better results. Request detailed comparisons, implementation outlines, trade-offs, and complexity assessments.

### Role Prompting

Assigning a specific role (e.g., security expert, senior developer) focuses the agent's perspective. Precise role definitions improve output quality.

Using XML tags can also help structure prompts and responses:
```xml
<role>You are a Security Researcher specializing in web vulnerabilities.</role>
<context>
Project: Payment gateway integration
Stack: Python, FastAPI, PostgreSQL
</context>
<instruction>
Review the authentication implementation for OWASP Top 10 vulnerabilities.
</instruction>
<output_format>
## Critical Issues
## Recommendations
</output_format>
```

**Best Practices:**
- Place role definitions at the top of configuration files.
- Align the role with the task's domain.
- Integrate role definitions with structured XML and concrete examples.
- Experiment with specificity to fine-tune behavior.
- Use consistent XML tag names.

### Creative Problem-Solving

1. **Diverge**: Generate at least five distinct approaches, withholding initial judgment.
2. **Converge**: Evaluate the trade-offs and constraints of each approach.
3. **Select**: Choose the most suitable approach and document the reasoning.
4. **Document**: Record rejected approaches to provide context for future developers.

---

## Architecture

### Design

- **Favor Composition**: Use composition and delegation to reduce coupling.
- **Be Explicit**: Avoid implicit behaviors or "clever" tricks.
- **Use Dependency Injection**: Pass dependencies to simplify testing and reuse.
- **Design for API Stability**: Minimize breaking changes in public APIs. Internal refactoring should not affect external users.
- **Handle Errors Gracefully**: Catch specific errors; avoid broad `except:` clauses.

### Decision-Making

Consider these factors when selecting an approach:
- **Testability**: Can the solution be thoroughly tested?
- **Readability**: Will a new developer understand the code quickly?
- **Consistency**: Does it fit existing patterns?
- **Simplicity**: Is it the simplest effective solution?
- **Reversibility**: How hard is it to revert if wrong?
- **Maintainability**: Can others modify and extend the code easily?

### Security

- **Integrate Security Early**: Design auth/authz before writing code.
- **Multi-Layered Defense**: Use defense in depth.
- **Least Privilege**: Grant only minimum necessary permissions.
- **Separate Secrets**: Keep secrets out of version control.
- **Validate Inputs**: Validate all external data.
- **Use Parameterized Queries**: Prevent SQL injection.
- **Monitor Behavior**: Watch for anomalies.
- **Scan for Threats**: Check file uploads and data streams.

### Performance

- **Measure First**: Profile before optimizing.
- **Macro-Optimizations**: Focus on algorithms and architecture.
- **Document Trade-offs**: Explain performance costs of security measures.
- **Cache Invalidation**: Plan for invalidation when data changes.

---

## Integrating with Existing Codebases

### Learning a New Codebase

Before starting:
1. Identify three existing features similar to what you intend to build.
2. Identify patterns for error handling, testing, and naming.
3. Use established libraries and utility functions.
4. Follow established testing patterns.

### Tooling and Dependencies

- Use established tools and systems.
- Avoid new tools or external dependencies without a clear justification.
- Stick to project conventions.
- Prefer built-in functionality over new dependencies.

### Automation and Consistency

- Use automation (e.g., GitHub Actions) for PR checks.
- Ensure consistent configuration (especially in monorepos).
- Maintain consistent patterns across projects.

---

## Context Management

### Command Optimization

Excessive output floods the context window. Avoid commands that generate large amounts of text.

**Verbose commands to avoid:**
- `npm install` or `pip install` without a quiet flag.
- `git log` or `git diff` without output limits.
- `ls -la` or `find .` without limits.

**Targeted commands to use instead:**
- `npm install --silent` or `pip install --quiet`.
- `git log --oneline -5` or `git diff --stat`.
- `ls -1 | head -20` or `find . -name "*.py" | head -10`.

### Session Management

- Use `/context` to monitor token usage.
- Use `/compact` cautiously (it can hide errors).
- Use `/clear` and `/catchup` to clean up sessions.
- Resume sessions to analyze errors.

---

## Common Anti-Patterns

### Code Quality

- **Over-engineering**: Building complex systems for simple problems.
- **Hidden Fragility**: Ignoring edge cases and system interactions.
- **Library Misuse**: Using libraries without understanding them.
- **"AI Slop"**: Using generic identifiers (e.g., `data`) or rigid, "machine-perfect" formatting.

### Security

- **"Eyeball Test"**: Assuming security based on a quick review.
- **Static Defense**: Relying only on input filtering, ignoring runtime behavior.
- **Ignoring Invisible Content**: Failing to scan metadata or hidden data.
- **Input-Centric Governance**: Focusing only on inputs, neglecting output safety.

### Workflow

- **"Big Bang" Changes**: Large, untested commits.
- **Premature Abstraction**: Abstracting simple problems too early.
- **Documentation Debt**: Changing code without updating docs.
- **Commits Lacking Context**: Unclear commit messages.
- **Skipping Tests**: Not writing tests.
- **Ignoring Linting**: Ignoring linter warnings.
- **"Cargo Culting"**: Copying code without understanding.
- **Analysis Paralysis**: Over-planning.

---

## Quick Reference

### Essential Commands
```bash
# To format, lint, and test the code:
make fmt lint test --quiet

# To execute different test suites:
make test-coverage --quiet
make test-unit --quiet

# For common Git operations:
git log --oneline -5
git diff --stat
git status --porcelain

# For file and directory operations:
ls -1 | head -20
find . -name "*.py" | head -10
```

### Decision Checklist

- Do you understand the problem?
- Did you consider multiple approaches?
- Is this the simplest effective solution?
- Did you write tests first?
- Do the changes follow project patterns?

### Commit Checklist

- The code compiles or builds successfully.
- All tests pass without errors.
- New functionality is covered by corresponding new tests.
- The codebase is free of linting errors.
- The commit message clearly explains the change's rationale.

---

## Available Skills and Agents

Skills and agents are discovered dynamically at runtime. To view available resources:

```bash
# Inspect the last skill scan (paths + hashes)
jq -r '.skills[].path' ~/.codex/skills-cache.json

# Or enumerate from disk
find ~/.codex/skills -name SKILL.md -type f

# Sync agents from external sources
skrills sync-agents --path <agent-manifest>

# View skill discovery diagnostics
skrills doctor
```

### Skill Discovery

Skills are automatically discovered from these locations (in priority order):

1. **Codex skills**: `~/.codex/skills/`
2. **Mirror skills**: `~/.codex/skills-mirror/`
3. **Claude skills**: `~/.claude/skills/` (when `--include-claude` is enabled)
4. **Marketplace cache**: `~/.codex/plugins/cache/`

#### Skill Naming Caveat

Skill names come from the `name:` field in `SKILL.md` frontmatter and should be treated as opaque strings.
They may include punctuation such as `:` for namespacing (for example, `pensive:shared`).

When parsing a rendered “skills list” (session headers, logs, etc.), do **not** split on `:` to extract the
name or description. Prefer extracting the `(file: …/SKILL.md)` path or reading the frontmatter directly.

### Agent Registration

Agents can be registered via:

1. **Manifest files**: YAML/TOML files defining agent specifications
2. **Plugin agents**: Discovered from installed plugins
3. **Sync command**: `skrills sync-agents` to sync from external sources

For detailed configuration options, see `docs/runtime-options.md` and `book/src/cli.md`.

<!-- available_skills:start -->
<!-- Skills discovered dynamically. Last sync: 1765958141 UTC. Total: 120 skills. -->
<!-- Use CLI commands for current skill inventory:
     jq -r '.skills[].path' ~/.codex/skills-cache.json
     find ~/.codex/skills -name SKILL.md -type f
     skrills analyze           - Analyze skills (tokens/deps) to spot issues
     skrills doctor            - View discovery diagnostics
-->
<!-- available_skills:end -->

<!-- available_agents:start -->
<!-- Agents discovered dynamically. Total: 291 agents. -->
<!-- Use CLI commands for current agent inventory:
     skrills sync-agents       - Sync agents from external sources
     skrills doctor            - View agent discovery diagnostics
-->
<!-- available_agents:end -->