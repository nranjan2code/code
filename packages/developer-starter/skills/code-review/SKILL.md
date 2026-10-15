---
name: code-review
description: Review a change for correctness, security, regressions and maintainability, with findings that each name a place, an impact and a fix.
serves: vcs, documents
---

# Code review

1. Get the change: `bash` with `git diff` (or `git show`, or the files
   named), then `read` enough of the surrounding code to know what it
   calls and what calls it.
2. Look for defects in this order: wrong behaviour and broken edge
   cases, security (untrusted input, secrets, permissions), data loss
   and concurrency, missing or weakened tests, then maintainability.
   Style preferences are not findings unless the project's own rules
   say so.
3. Check a suspicion before reporting it: run the test, `grep` for the
   other callers, try the input. Mark what you could not check as
   unverified rather than stating it as fact.
4. Each finding: the file and line, what goes wrong and for which input,
   why it matters, and the fix. Order them most severe first.
5. Show findings as an `emit_table_card` (`Severity | Location |
   Finding | Fix`), or an `emit_list_card` when there are only a few,
   and an `emit_diff_card` for a suggested change. Say plainly when
   nothing was found.
