# Debugging

Find the cause of a failure before changing code. A fix that is not tied to a
reproduced cause usually moves the bug.

## Steps

1. Reproduce the failure. Record the exact command, input and error output.
2. Reduce it to the smallest case that still fails.
3. Read the code path between the input and the error. List 2-3 candidate
   causes and what each would predict.
4. Test the candidates one at a time with logs, a debugger or a focused test.
   Change one thing per experiment.
5. Write a failing test that captures the bug, then fix the confirmed cause.
6. Run the full suite and the original reproduction.

## Report

State the symptom, the confirmed cause, the fix, and how you verified it. If
the cause is still unknown, say so and list what was ruled out.

## Avoid

- Changing several things at once, then guessing which one helped.
- Adding retries or sleeps to hide a race.
- Closing the bug when the symptom disappears but the cause is unexplained.
