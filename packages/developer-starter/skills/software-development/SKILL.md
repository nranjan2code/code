---
name: software-development
description: Implement and explain software changes that follow the project's own conventions, with focused tests and an exact account of what was verified.
serves: code-exec, documents, vcs, filesystem
---

# Software development

1. Learn the project before changing it: `glob` and `grep` to find the
   code, `read` the files you will touch and their tests, and the
   project's own notes (README, CONTRIBUTING, an agent guide). Follow
   its conventions, not your defaults.
2. Declare the deliverables with `plan` (the change, its check).
3. Make the smallest coherent change with `edit` (or `write` for a new
   file). Keep public interfaces as they are unless the request is to
   change them.
4. Add or update a test for the behaviour you changed, then run the
   project's own test and lint commands with `bash`. Run the narrowest
   command that proves the change first, then the wider one.
5. Report exactly what ran and how it ended: a command's output belongs
   in an `emit_terminal_card` or, for a test run, an
   `emit_test_report_card`, both filled from the call that ran it. Say
   what you did not verify. Never describe a check as passed that did
   not run.
6. Show a change someone should look at with an `emit_diff_card`. Do not
   commit or push unless asked.
