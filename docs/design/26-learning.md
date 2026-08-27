# 26 — Learning and skills

Learning is a Runtime-mediated write path. Model suggestions are untrusted
data until a user reviews them; a suggestion can never alter policy or become
an instruction merely by being stored.

## Memory

The `remember` operation creates a scoped memory record through the Runtime.
It records project/session provenance, is audited, and becomes searchable only
according to its scope. Amend and forget use the same revision-checked CRUD
path; forgetting creates a tombstone rather than rewriting history.

## Skills

Skills are Runtime records with name, description, body digest, project scope,
and review status. `GET /skills` lists them. Promotion and rejection are
explicit Runtime mutations. A promoted skill is included in discovery for
subsequent admissions, while the session that proposed it remains unchanged.

## Trust boundary

Skill and memory content is never copied into permissions, environment
variables, credentials, or executable commands. Runtime validates all paths and
applies the capability epoch before any file or process effect.

## Surfaces and verification

CLI, TUI, desktop, and admin show the same proposal and memory records and use
the same promote/reject/amend/forget commands. Tests cover duplicate screening,
scope isolation, provenance, audit entries, and discovery after promotion.
