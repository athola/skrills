# Refactoring

Change the structure of code without changing its behavior. Each step should
be small enough to review and revert on its own.

## Steps

1. Confirm tests cover the behavior you will touch. Add characterization
   tests first if they do not.
2. Run the tests and record that they pass before you start.
3. Make one structural change: rename, extract, inline, move or delete.
4. Run the tests again. If they fail, revert the step and take a smaller one.
5. Repeat until the target shape is reached.
6. Commit refactoring separately from behavior changes.

## Targets

- Duplicated logic that changes together.
- Functions that do several unrelated things.
- Names that no longer match what the code does.
- Abstractions with a single user and no second use in sight.

## Avoid

- Mixing a refactor with a bug fix or new feature.
- Editing tests so they pass after the change.
- Reformatting code the change does not need to touch.
