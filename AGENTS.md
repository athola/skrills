# AI Agent Development Guidelines

Practical guidance for building AI coding agents that produce working code.

---

## Core Philosophy

- **Incremental progress**: Small, working iterations are better than large, risky rewrites.
- **Context-aware application**: Adapt to project realities instead of rigidly applying rules.
- **Pragmatic engineering**: Balance competing concerns like performance and readability based on the situation.
- **Evidence-based decisions**: Use metrics and measurements to guide decisions, not just intuition.
- **Diverse solution exploration**: Generate multiple approaches before choosing one.
- **Simplicity**: Prefer simple, proven solutions. If code becomes so complex it requires extensive comments, refactor it.
- **Single responsibility**: Each component should have one clear purpose.
- **Avoid premature abstractions**: Don't create abstractions until a clear pattern has emerged (see the Rule of Three).
- **Explicit trade-offs**: Clearly document the pros, cons, and confidence level for each proposed solution.
- **Avoid pattern fixation**: If you find yourself using the same design pattern for every problem, you may be experiencing "mode collapse". Actively explore different approaches.

---

## Development Workflow

### Implementation Cycle
1.  **Understand**: Read existing code, identify patterns, and check tests.
2.  **Explore**: Generate multiple approaches with clear trade-offs.
3.  **Test**: Write a failing test first (when applicable).
4.  **Implement**: Write the minimum code required to pass the test.
5.  **Refactor**: Clean up the code while all tests are passing.
6.  **Commit**: Write a clear commit message explaining the "why" behind the change.

### When Stuck
If you've made three unsuccessful attempts to solve a problem:
1.  Document the failures, including the full error output.
2.  Research two or three alternative approaches.
3.  Question your fundamental assumptions about the problem.
4.  Try a completely different approach or simplify the problem.
5.  If you're still stuck, ask for help and provide the context from the previous steps.

### Session Management
- Use session history to analyze errors and track the development process.
- For complex tasks, it can be useful to document the current state and clear the session to start fresh.

---

## Quality Standards

### Commit Requirements
Every commit must:
- Compile or build successfully.
- Pass all existing tests, with no tests skipped.
- Include new tests for any new functionality.
- Adhere to project linting rules, generating zero warnings.
- Have a clear commit message that explains the "why" behind the change.

### Pre-Commit Workflow
This command should be run before committing to ensure all quality checks pass:
```bash
make format && make lint && make test --quiet && make build
```

### Effective Prompting
A well-structured prompt is more likely to yield good results.

For example, instead of asking a general question like:
> "How should I implement user authentication?"

Ask for a detailed comparison:
> "Generate four different approaches to user authentication. For each, provide:
> a) An implementation outline.
> b) Security trade-offs.
> c) A complexity assessment.
> d) A confidence score (0-100%)."

### Role Prompting
You can improve an agent's performance by assigning it a role. This helps the agent adopt a specific point of view, such as a security expert or a senior developer.

For example, you can add this to a project's `AGENTS.md` or `CLAUDE.md` file:
```markdown
You are a Senior Python Developer with 15 years of backend experience, specializing in API design and performance optimization.

Approach all tasks with:
- A test-driven development mindset.
- A "Functional Core, Imperative Shell" architectural style.
- A security-first approach.
- Clear documentation.
```

The more specific the role, the better. For instance, "Senior Data Scientist at a Fortune 500 retail company, specializing in customer churn prediction and A/B testing" is better than "a data scientist".

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

Some best practices for role prompting:
- Place the role definition at the top of project configuration files.
- Align the role with the task's domain (e.g., assign a security role for security tasks).
- For best results, combine a role definition with XML structure and examples.
- Experiment with different levels of specificity to find what works best.
- Use consistent XML tag names across prompts.

### Creative Problem-Solving
1.  **Diverge**: Generate at least five distinct approaches without judging them.
2.  **Converge**: Evaluate the trade-offs and constraints of each approach.
3.  **Select**: Choose the best approach and document why you chose it.
4.  **Document**: Record the other approaches you considered and why you rejected them. This can be valuable context for future developers.

---

## Architecture Guidelines

### Design Principles
- **Composition over inheritance**: Favor interfaces and delegation.
- **Explicit over implicit**: Write clear code that doesn't hide its behavior.
- **Interfaces over singletons**: Use dependency injection to make code more testable.
- **Stable public APIs**: Internal changes should not break external consumers.
- **Thorough error handling**: Avoid broad `except:` clauses that can hide bugs.

### Decision-Making
When choosing an approach, evaluate it on:
- **Testability**: How easily can I write tests for this?
- **Readability**: Will this make sense to a new developer in six months?
- **Consistency**: Does this match the project's existing patterns?
- **Simplicity**: Is this the simplest solution that works?
- **Reversibility**: How difficult would it be to change this decision later?
- **Maintainability**: Can other developers easily understand and modify this code?

### Security Principles
- **Build security in from the start**: Don't treat security as an afterthought.
- **Defense in depth**: Use multiple layers of security controls.
- **Least privilege**: Grant only the minimum permissions necessary.
- **Never commit secrets**: Use environment variables or a secrets management system.
- **Validate all input**: Trust no data coming from external sources.
- **Use parameterized queries**: Do not use string concatenation to build SQL queries.
- **Monitor outcomes**: Don't just validate inputs; monitor the behavior of the system in real-time.
- **Scan for hidden threats**: Check for malicious content that may be hidden in uploads or other data.

### Performance Principles
- **Measure before you optimize**: Use a profiler to identify bottlenecks, don't guess.
- **Focus on architectural improvements**: Algorithmic changes usually have a bigger impact than micro-optimizations.
- **Document security-performance trade-offs**: If a security measure impacts performance, document the reasoning.
- **Plan for cache invalidation**: Before you implement a cache, have a clear strategy for invalidating it.

---

## Integrating with Existing Codebases

### Learning a New Codebase
Before writing any code, get familiar with the project:
1.  Find three features that are similar to what you're building.
2.  Identify common patterns for things like error handling, testing, and naming.
3.  Use the project's existing libraries and utility functions instead of reinventing the wheel.
4.  Follow the established patterns for writing tests.

### Tooling and Dependencies
- Use the project's existing tools and systems.
- Don't introduce new tools without a clear justification.
- Follow the project's conventions and patterns.
- Prefer built-in functionality over new external dependencies.

### Automation and Consistency
- Use automation, like GitHub Actions, to handle pull request checks.
- Maintain consistent configuration, especially in a monorepo.
- Strive for consistent patterns across different projects within an organization.

---

## Context Management

### Command Optimization
Avoid commands that produce a large amount of output. Be specific.

Verbose commands to avoid:
- `npm install` or `pip install` without a "silent" or "quiet" flag.
- `git log` or `git diff` without limits.
- `ls -la` or `find .` without limits.

Targeted commands to use instead:
- `npm install --silent` or `pip install --quiet`.
- `git log --oneline -5` or `git diff --stat`.
- `ls -1 | head -20` or `find . -name "*.py" | head -10`.

### Session Management
- Use `/context` to monitor token usage.
- Avoid using `/compact`, as it can be opaque and error-prone.
- Use `/clear` and `/catchup` for clean restarts.
- Resume sessions to analyze errors.

---

## Common Anti-Patterns

### Code Quality
- **Over-engineering**: Don't build complex systems for simple problems.
- **Hidden fragility**: Be aware of edge cases and how different parts of the system interact.
- **Library misuse**: Don't use libraries without understanding them, and don't invent libraries that don't exist.
- **"AI slop"**: Avoid generic names like `data` or `value`, and don't obsess over "machine-perfect" formatting.

### Security
- **The "eyeball test"**: Don't assume code is secure just by looking at it.
- **Static defense**: Don't just filter inputs; monitor the system's behavior.
- **Ignoring invisible content**: Scan for threats hidden in metadata or other non-visible parts of data.
- **Governing inputs, not outcomes**: Don't just focus on preventing bad inputs; make sure the system's outputs are also correct and safe.

### Workflow
- **"Big bang" changes**: Avoid large, untested commits.
- **Premature abstraction**: Don't create complex abstractions for simple problems.
- **Documentation debt**: Update documentation as you change the code.
- **Commits without context**: Write commit messages that explain the "why" of the change.
- **Skipping tests**: Don't defer writing tests.
- **Ignoring linting**: Pay attention to linter warnings; they often point to real issues.
- **"Cargo culting"**: Don't copy code or patterns without understanding why they are used.
- **Analysis paralysis**: Avoid over-planning at the expense of getting things done.

---

## Quick Reference

### Essential Commands
```bash
# Run the full development cycle of formatting, linting, and testing.
make format && make lint && make test --quiet

# Run different test suites.
make test --quiet
make test-coverage --quiet
make test-unit --quiet

# Common Git operations.
git log --oneline -5
git diff --stat
git status --porcelain

# File and directory operations.
ls -1 | head -20
find . -name "*.py" | head -10
```

### Decision Checklist
Before implementing a solution, ask yourself:
- Have I fully understood the problem?
- Have I generated multiple approaches?
- Have I chosen the simplest working solution?
- Have I written tests first?
- Have I followed the project's existing patterns?

### Commit Checklist
Before committing code, ensure that:
- The code compiles or builds successfully.
- All tests pass.
- New tests for new functionality are included.
- The code is free of linting errors.
- The commit message explains the "why" behind the change.
