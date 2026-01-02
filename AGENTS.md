# AI Agent Development Guidelines

This document provides essential guidelines for building AI coding agents that produce functional, high-quality code.

---

## Guiding Principles

- **Iterate Incrementally**: Prioritize small, functional changes as they are generally safer and more manageable than attempting extensive rewrites
- **Adapt to Project Conventions**: Recognize that each project maintains its unique set of conventions. Adapt to these established patterns rather than rigidly adhering to a prescribed set of external rules
- **Engineer Pragmatically**: Strive for a balanced approach to engineering, carefully weighing trade-offs such as performance, readability, and security within the specific project context
- **Base Decisions on Evidence**: Ground all technical decisions in empirical data, drawing insights from profiling, metrics, and other measurable observations, rather than relying solely on intuition
- **Explore Diverse Solutions**: Generate and evaluate multiple potential approaches before committing to a single one, thereby avoiding the tendency to settle on the initial idea
- **Prioritize Simplicity**: Favor simple, well-established solutions. If code necessitates extensive commenting to convey its intent, it is often an indicator that refactoring is required for clarity
- **Single Responsibility Principle**: Adhere to the Single Responsibility Principle, ensuring that each component serves one distinct and clearly defined purpose
- **Defer Abstraction**: Refrain from introducing abstractions until a clear and recurring pattern has been unequivocally identified (e.g., as per the Rule of Three)
- **Document Assumptions**: When proposing solutions, explicitly document their advantages, disadvantages, and your confidence level. This practice fosters transparency in the decision-making process
- **Cultivate Design Diversity**: Actively explore and embrace different approaches to problem-solving to avoid "mode collapse," a state where one habitually applies the same design pattern to every challenge

---

## Development Workflow

### Implementation Cycle

1. **Understand**: Begin by thoroughly reading existing code, identifying established patterns, and reviewing associated tests
2. **Explore**: Develop multiple viable approaches, clearly articulating their respective trade-offs
3. **Test**: When applicable, author a failing test case prior to implementing new code to drive development
4. **Implement**: Write the minimal amount of code necessary to satisfy the test case
5. **Refactor**: Refine and clean the code while ensuring all tests continue to pass
6. **Commit**: Craft a clear and concise commit message that elucidates the rationale behind the change

### When Stuck

If you have made three unsuccessful attempts to solve a problem:

1. Document all failures, including full error outputs
2. Investigate two or three alternative approaches to the problem
3. Critically re-evaluate underlying assumptions about the problem
4. Experiment with a fundamentally different or simpler approach
5. If the issue persists, seek assistance, providing comprehensive context from the preceding steps

### Session Management

- Utilize the session history to analyze errors and monitor the progression of the development process
- For complex tasks, consider documenting the current state and then clearing the session to initiate a fresh start

---

## Quality Standards

### Commit Requirements

Every commit must:

- The codebase must compile or build successfully
- All existing tests must pass, with no tests skipped
- New functionality must be accompanied by corresponding new tests
- The code must adhere to project linting rules and produce no warnings
- Each commit must include a clear message explaining the change's rationale

### Pre-Commit Workflow

Execute the following command prior to committing to ensure all quality checks pass:

```bash
make fmt lint test --quiet build
```

### Effective Prompting

A meticulously structured prompt significantly increases the likelihood of generating high-quality results. Request detailed comparisons of approaches, with implementation outlines, trade-offs, and complexity assessments.

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
- **Strategize Cache Invalidation**: Develop a clear and robust strategy for invalidating caches whenever underlying data undergoes modification.

---

## Integrating with Existing Codebases

### Learning a New Codebase
Prior to initiating code development, thoroughly familiarize yourself with the existing project codebase:
1. Identify at least three existing features that closely resemble the functionality you intend to build.
2. Pinpoint common patterns for error handling, testing methodologies, and naming conventions within the project.
3. Leverage the project's established libraries and utility functions.
4. Adhere to established patterns and best practices for writing tests within the codebase.

### Tooling and Dependencies
- Utilize the project's established tools and systems.
- Refrain from introducing new tools or external dependencies without a clear and compelling justification.
- Adhere strictly to project conventions and established patterns.
- Prioritize the use of built-in functionality over introducing new external dependencies.

### Automation and Consistency
- Employ automation (e.g., GitHub Actions) for conducting pull request checks.
- Ensure consistent configuration across the codebase, particularly within a monorepo structure.
- Endeavor to maintain consistent patterns across all projects within the organization.

---

## Context Management

### Command Optimization
To maintain clear context and avoid overwhelming output, refrain from executing commands that generate excessive verbose output. Strive for specificity.

**Verbose commands to avoid:**
- `npm install` or `pip install` without employing a quiet flag.
- `git log` or `git diff` without specifying output limits.
- `ls -la` or `find .` without imposing result limits.

**Targeted commands to use instead:**
- For quieter installations, use `npm install --silent` or `pip install --quiet`.
- For concise Git history, employ `git log --oneline -5` or `git diff --stat`.
- To limit file listings, use `ls -1 | head -20` or `find . -name "*.py" | head -10`.

### Session Management
- Utilize `/context` to monitor token usage during sessions.
- Exercise caution with `/compact` due to its potential for opacity and error-proneness.
- Employ `/clear` and `/catchup` commands to facilitate clean session restarts.
- Resume sessions when necessary to analyze errors effectively.

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
<!-- Skills discovered dynamically. Last sync: 1767394163 UTC. Total: 1 skills. -->
<!-- Use CLI commands for current skill inventory:
     jq -r '.skills[].path' ~/.codex/skills-cache.json
     find ~/.codex/skills -name SKILL.md -type f
     skrills analyze           - Analyze skills (tokens/deps) to spot issues
     skrills doctor            - View discovery diagnostics
-->
<!-- available_skills:end -->
