# AI Agent Development Guidelines

This document provides essential guidelines for building AI coding agents that produce functional, high-quality code.

---

## Guiding Principles

- **Iterate Incrementally**: Prioritize small, functional changes. They are safer and easier to manage than large rewrites.
- **Adapt to Project Conventions**: Each project has its own conventions. Adapt to them rather than sticking to external rules.
- **Engineer Pragmatically**: Balance trade-offs like performance, readability, and security within the context.
- **Base Decisions on Evidence**: Ground technical decisions in data (profiling, metrics) rather than intuition.
- **Explore Diverse Solutions**: Generate and evaluate multiple approaches before committing to one.
- **Prioritize Simplicity**: Favor simple, standard solutions. Code that needs extensive comments usually needs refactoring.
- **Single Responsibility Principle**: Each component should serve one distinct, clearly defined purpose.
- **Defer Abstraction**: Avoid abstractions until a clear pattern is identified (e.g., Rule of Three).
- **Document Assumptions**: Explicitly document advantages, disadvantages, and confidence levels. This fosters transparency.
- **Cultivate Design Diversity**: Explore different approaches to avoid applying the same pattern to every problem ("mode collapse").

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

- Use session history to analyze errors and monitor progress.
- For complex tasks, document the current state, then clear the session and restart.

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

A structured prompt increases the chance of high-quality results. Request detailed comparisons, implementation outlines, trade-offs, and complexity assessments.

### Role Prompting
Assigning a specific role to an agent can enhance its performance by providing a focused perspective (e.g., a security expert or a senior developer). The precision of the role definition directly correlates with the quality of the agent's output.

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

**Role Prompting Best Practices:**
- Position role definitions prominently at the top of relevant project configuration files.
- Ensure the assigned role aligns accurately with the specific domain of the task.
- For optimal results, integrate role definitions with structured XML and provide concrete examples.
- Experiment with varying levels of role specificity to fine-tune agent behavior.
- Maintain consistency in XML tag names across all prompts for clarity and parseability.

### Creative Problem-Solving
1. **Diverge**: Generate a minimum of five distinct approaches to the problem, deliberately withholding initial judgment.
2. **Converge**: Systematically evaluate the trade-offs and inherent constraints associated with each generated approach.
3. **Select**: Choose the most suitable approach and thoroughly document the underlying reasoning for its selection.
4. **Document**: Record all other approaches considered, along with the justifications for their rejection. This provides valuable context for future developers.

---

## Architecture

Robust architecture facilitates long-term manageability and adaptability.

### Design
- **Favor Composition Over Inheritance**: Prioritize composition and delegation over deep class hierarchies to enhance flexibility and reduce tight coupling.
- **Promote Explicitness**: Write code that is unambiguously clear and easy to understand, actively avoiding 'clever' tricks or implicit behaviors.
- **Utilize Dependency Injection**: Employ dependency injection to pass required dependencies into components, thereby simplifying testing and promoting reusability, rather than relying on singletons.
- **Design for API Stability**: Design public APIs to be stable, minimizing breaking changes. Internal refactoring efforts should not impact external users of the API.
- **Handle Errors Gracefully**: Implement precise error handling, avoiding broad `except:` clauses that can inadvertently mask bugs. Be specific about the types of errors being caught.

### Decision-Making
When evaluating and selecting between different approaches, the following factors should be carefully considered:
- **Testability**: Assess the ease with which the proposed solution can be thoroughly tested.
- **Readability**: Consider whether a new developer joining the project would easily comprehend the code within a reasonable timeframe (e.g., six months).
- **Consistency**: Evaluate how well the solution integrates with existing patterns and conventions within the codebase.
- **Simplicity**: Prioritize the simplest effective solution that addresses the problem.
- **Reversibility**: Assess the complexity and effort required to revert the decision if it proves suboptimal.
- **Maintainability**: Consider the ease with which other developers can understand, modify, and extend the code.

### Security
- **Integrate Security Early**: Incorporate security considerations from the outset of the development lifecycle, rather than treating it as an afterthought.
- **Multi-Layered Defense**: Implement a "defense in depth" strategy by deploying multiple layers of security controls.
- **Principle of Least Privilege**: Adhere to the principle of least privilege by granting only the minimum necessary permissions.
- **Separate Secrets from Code**: Strictly avoid committing secrets to version control systems.
- **Validate All Inputs**: Treat all data originating from external sources as inherently untrusted and subject to rigorous validation.
- **Use Parameterized Queries**: Employ parameterized queries for database interactions, specifically avoiding string concatenation to construct SQL queries.
- **Monitor System Behavior**: Continuously monitor system behavior in real-time to proactively detect anomalies and potential threats.
- **Scan for Covert Threats**: Implement scanning mechanisms to detect malicious content potentially hidden within file uploads or other data streams.

### Performance
- **Measure Before Optimizing**: Avoid premature optimization; first, identify performance bottlenecks through profiling tools before attempting to optimize code.
- **Prioritize Macro-Optimizations**: Recognize that algorithmic and architectural improvements yield significantly greater performance impacts compared to micro-optimizations.
- **Document Performance Trade-offs**: If a security measure introduces a performance impact, document this trade-off and provide a clear explanation of its necessity.
- **Strategize Cache Invalidation**: Develop a reliable strategy for invalidating caches whenever underlying data undergoes modification.

---

## Integrating with Existing Codebases

### Learning a New Codebase
Before starting:
1. Identify at least three existing features similar to what you intend to build.
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
To maintain clear context and avoid overwhelming output, avoid executing commands that generate excessive output. Be specific.

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
- **Over-engineering**: Constructing overly complex systems to address straightforward problems.
- **Hidden Fragility**: Overlooking edge cases and intricate system interactions.
- **Library Misuse**: Employing libraries without a clear understanding of their functionality or implications.
- **"AI Slop"**: Characterized by the use of generic identifiers (e.g., `data`) or an excessive focus on rigid, "machine-perfect" formatting.

### Security
- **"Eyeball Test"**: Assuming code security based solely on superficial review.
- **Static Defense**: Relying exclusively on input filtering rather than a dynamic approach that includes monitoring system behavior.
- **Ignoring Invisible Content**: Failing to scan for threats within metadata or other non-visible data components.
- **Input-Centric Governance**: Focusing exclusively on preventing malicious inputs, without ensuring the safety and integrity of outputs.

### Workflow
- **"Big Bang" Changes**: Implementing large, untested commits that introduce significant risk.
- **Premature Abstraction**: Introducing complex abstractions for inherently simple problems.
- **Documentation Debt**: Neglecting to update documentation concurrently with code changes.
- **Commits Lacking Context**: Commit messages that fail to adequately explain the rationale behind the change.
- **Skipping Tests**: Deferring the creation of necessary test cases.
- **Ignoring Linting**: Disregarding linter warnings, which often highlight genuine issues or potential bugs.
- **"Cargo Culting"**: Blindly copying code or patterns without a fundamental understanding of their underlying principles.
- **Analysis Paralysis**: Excessive planning and deliberation that hinders decisive action.

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
Before proceeding with the implementation of a solution, consider the following questions:
- Is the problem thoroughly understood?
- Have multiple distinct approaches been generated?
- Has the simplest effective solution been selected?
- Have tests been written prior to implementation?
- Do the changes adhere to the project's existing patterns?

### Commit Checklist
Prior to committing changes, ensure the following:
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