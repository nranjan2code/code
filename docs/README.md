# Documentation guide

Vakyartha is the public name; `vak` is the command and the internal crate
prefix. Start with the repository [README](../README.md) for installation and
everyday use. The workspace version in [Cargo.toml](../Cargo.toml) is the
authoritative version.

## Current guidance

| Need | Read |
|---|---|
| Build, install, update or uninstall | [Release and install](release-and-install.md) |
| Develop and verify a change | [Development](development.md) and the repository [agent contract](../AGENTS.md) |
| Run a server or expose it remotely | [Hosting](hosting.md) |
| Understand the product and security boundaries | [Agent-owned platform](design/64-agent-owned-platform.md) and [agent security](design/24-agent-security.md) |
| Work with artwork and public identity | [Brand guide](brand/README.md) |
| Understand how prompts are built and used | [The prompt system, end to end](design/83-prompt-system.md) |
| Explore diagrams | [Architecture diagrams](architecture/README.md) and [tutor](tutor/README.md) |

The architecture overview uses Mermaid and the tutor uses PNGs, so both render
on GitHub. Interactive HTML references are local browser artifacts; GitHub
shows their source. Each dated diagram identifies its historical scope.

## The record: one place for each kind of fact

Start a session here, then read only what the task needs. Each question has
one home; nothing else restates it.

| Question | The one place |
|---|---|
| How do we work, and what may never break? | [AGENTS.md](../AGENTS.md): the rules and invariants, for people and agents alike |
| What is each design doc, and is it built? | [design/INDEX.md](design/INDEX.md), generated from every doc's `Status:` line |
| What does a design say, and which parts are built? | The doc in [design](design/); its `Status:` line is the truth for that doc |
| What is not done yet, across the product? | [plans/open-items.md](plans/open-items.md), the one tracker |
| What did the maintainer decide? | [decisions.md](decisions.md), dated, each line pointing to its full record |
| What did the last session leave? | The one `plans/handover-*.md`: the prompt for the next session and working notes |
| What changed for users? | [CHANGELOG.md](../CHANGELOG.md): `## Unreleased`, then each release |
| How was a plan built, step by step? | Its plan in [plans](plans/) (the data architecture, the visual refresh, mail and calendar) |
| What did a check or review find on a date? | [audits](audits/), [reviews](reviews/), [research](research/): dated records |

Every design doc's and every plan's `Status:` line (the handover and the
tracker aside) starts with one of six words, then says what is built, what
is not, and why:

| Word | Meaning |
|---|---|
| `proposal` | a design not built yet; its paths are targets, not citations |
| `in progress` | partly built; the line names which parts |
| `implemented` | built; the line names anything cut by a decision |
| `reference` | describes shipped code or how to work on it |
| `superseded` | replaced; the line names what replaced it |
| `historical` | a record of the past, kept as it was |

A section of a live doc that records the past (an audit of the tree on a
date) starts with a `> **Historical record**` line; it is kept as written,
never updated to today.

## Keeping it current

Do these in the same commit as the change they describe:

| When you | Update |
|---|---|
| build, finish or cut part of a design | that doc's `Status:` line, then `python3 scripts/check_records.py --write` |
| change behaviour a person can notice | an entry under `## Unreleased` in CHANGELOG.md |
| change a rule or an invariant | AGENTS.md |
| find work left undone, or close it | [plans/open-items.md](plans/open-items.md) |
| get a decision from the maintainer | a dated line in [decisions.md](decisions.md), and the doc that records it |
| end a session | the one handover: replace its prompt and "Where things stand"; keep "Working notes" true |
| cut a release | CHANGELOG.md's `## Unreleased` becomes the version's heading (`docs/release-and-install.md`) |

Docs are kept, never deleted, so the record reads the same for people and
agents. A finished or replaced doc changes its status word; a handover is the
one exception, because only the latest is ever true.

## Older records

The [roadmap](design/00-roadmap.md) is historical; [the agent-owned
platform](design/64-agent-owned-platform.md) is the current product model.
The dated [audits](audits/), [reviews](reviews/), [research](research/),
[visual reports](uiux-audit-report.md), and [LinkedIn drafts](linkedin/) are
records of their own review dates. They can explain a decision, but their
measurements, line numbers, and open findings need fresh verification against
the current tree. The [video](video/) and [brand explorations](brand/) folders
contain production assets and working material.

## Checking a documentation change

Run `python3 scripts/check_records.py`, `python3 scripts/check_doc_paths.py`
and `scripts/check-version.sh` from the repository root (`scripts/gate.sh`
runs all three). The first checks the record above (status words, the
generated index, one handover, the tracker and decision log, a CHANGELOG
entry once code changed since the last release); the second checks cited
repository paths; the third checks version stamps. Compare behavior claims with current code and tests,
and check relative links and rendered layout when editing a page. Neither
script establishes that a statement still describes shipped behavior.
