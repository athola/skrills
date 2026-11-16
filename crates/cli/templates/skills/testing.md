# Testing

Write tests that define behavior before the code that satisfies it. A test
that cannot fail proves nothing.

## Steps

1. State the behavior as one sentence: given X, when Y, then Z.
2. Write the test and run it. Confirm it fails for the expected reason.
3. Write the smallest code that makes it pass.
4. Run the test again, then the whole suite.
5. Refactor with the tests green.

## What to cover

- The normal case.
- Each boundary: empty, one, maximum, just over the maximum.
- Each error path and each `if` branch.
- Behavior that was broken before, as a regression test.

## Rules

- Name tests after the behavior, for example `rejects_expired_token`.
- Mock only external dependencies, never the code under test.
- Keep tests independent of order and of the clock.
- Never delete or loosen a test to get a build passing. Fix the behavior.
