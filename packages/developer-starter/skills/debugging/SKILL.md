---
name: debugging
description: Diagnose a failure from evidence (reproduce it, find the first real error, trace it to its cause) before proposing or applying a fix.
serves: code-exec, observability
---

# Debugging

1. Reproduce it. Run the failing command or test with `bash` and keep
   its exact output; if it does not fail for you, say so and ask what
   differs, rather than guessing.
2. Find the first meaningful error, not the last line: the earliest
   exception, the first failing assertion, the first non-zero exit.
3. Trace it to the boundary where a correct input becomes a wrong
   result: `grep` for where the value is made, `read` the code around
   it, and check each assumption with a command rather than by reading
   alone.
4. Keep the confirmed cause apart from hypotheses. "The model", "a
   flaky test" or "the environment" is not a cause until evidence shows
   it.
5. Fix the cause with the smallest change, add a test that failed
   before the fix and passes after it, and run it.
6. Report the cause with its evidence, the fix and the run that verifies
   it. Any displayed command output or test report comes from that recorded
   call. Say what is still unexplained.
