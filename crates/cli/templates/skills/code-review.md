# Code Review

Review a change for correctness first, then maintainability. Every comment
should say what is wrong, why it matters, and what to do instead.

## Steps

1. Read the description and linked issue. Confirm the change matches the goal.
2. Read the tests before the implementation. Check they would fail without
   the change.
3. Read the diff in dependency order. For each hunk ask what input breaks it.
4. Check error paths, boundary values, concurrency and resource cleanup.
5. Check public API, config and data format changes for compatibility.
6. Run the change locally when behavior is unclear from reading.

## Comments

- Mark each comment as blocking, suggestion or question.
- Point at the line and give a concrete alternative.
- Do not restate what the diff already shows.

## Checklist

- Does it do what the description says, and nothing else?
- Is each new branch tested?
- Are secrets, user input and file paths handled safely?
- Can the change be reverted on its own?
