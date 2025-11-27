# AI Agent Development Guidelines

Practical guidance for building AI coding agents that produce working code.

---

## Guiding Principles

- **Iterate Incrementally**: Start with small, working changes. It's safer and more effective than attempting large, risky rewrites.
- **Adapt to the Project**: Apply these guidelines with judgment. Every project has its own conventions and constraints; adapt to them rather than rigidly following rules.
- **Engineer Pragmatically**: Balance trade-offs. Performance, readability, and security are all important, but their relative priority depends on the situation.
- **Use Evidence**: Make decisions based on data. Use profiling, metrics, and other measurements to support your choices, not just intuition.
- **Explore Multiple Solutions**: Generate several approaches before committing to one. This helps avoid settling on the first idea that comes to mind.
- **Keep It Simple**: Prefer simple, proven solutions. If your code needs extensive comments to be understood, it's a sign that it should be refactored.
- **One Job per Component**: Follow the Single Responsibility Principle. Each part of the system should have one clear purpose.
- **Delay Abstraction**: Don't create abstractions until you've identified a clear, recurring pattern (see the Rule of Three). Premature abstraction often leads to unnecessary complexity.
- **State Your Assumptions**: When proposing a solution, document the pros, cons, and your confidence level. This makes the decision-making process transparent.
- **Avoid "Mode Collapse"**: If you find yourself using the same design pattern for every problem, you may be stuck in a rut. Actively explore different approaches to find the best fit.

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
make fmt lint test --quiet build
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

## Architecture

Good architecture is about making decisions that will be easy to live with in the future. Here are a few principles to guide you.

### Design
- **Favor Composition Over Inheritance**: Instead of creating deep class hierarchies, use interfaces and delegation. It's more flexible and easier to test.
- **Be Explicit**: Write code that is easy to understand. Avoid clever tricks or implicit behaviors that might confuse other developers.
- **Use Dependency Injection**: Instead of using singletons, pass dependencies into your components. This makes them easier to test and reuse.
- **Design Stable APIs**: Once an API is public, you should avoid making breaking changes. Internal refactoring should not affect external users.
- **Handle Errors Gracefully**: Avoid broad `except:` clauses that can swallow exceptions and hide bugs. Be specific about the errors you catch.

### Decision-Making
When you're choosing between different approaches, consider these factors:
- **Testability**: How easy will it be to write tests for this?
- **Readability**: Will a new developer be able to understand this code in six months?
- **Consistency**: Does this fit with the existing patterns in the codebase?
- **Simplicity**: Is this the simplest solution that will work?
- **Reversibility**: How hard would it be to undo this decision later?
- **Maintainability**: How easy will it be for other developers to modify this code?

### Security
- **Build Security In**: Don't treat security as something you can add on at the end. Think about it from the beginning.
- **Use Multiple Layers**: Employ a "defense in depth" strategy. Don't rely on a single security control.
- **Grant Minimal Permissions**: Follow the principle of least privilege. Only grant the permissions that are absolutely necessary.
- **Keep Secrets Out of Code**: Never commit secrets to version control. Use environment variables or a secrets management system.
- **Validate All Inputs**: Treat all data from external sources as untrusted. Validate it before you use it.
- **Use Parameterized Queries**: Don't build SQL queries with string concatenation. It's a recipe for SQL injection.
- **Monitor System Behavior**: Don't just validate inputs; monitor the system's behavior in real-time to detect anomalies.
- **Scan for Hidden Threats**: Check for malicious content that might be hidden in file uploads or other data.

### Performance
- **Measure First**: Don't try to optimize code without knowing where the bottlenecks are. Use a profiler to identify them.
- **Focus on a Broader Scale**: Algorithmic and architectural improvements usually have a much bigger impact than micro-optimizations.
- **Document Trade-offs**: If a security measure has a performance impact, document it and explain why it's necessary.
- **Plan for Cache Invalidation**: Caching is a great way to improve performance, but you need a clear strategy for invalidating the cache when the underlying data changes.

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
make fmt lint test --quiet

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
