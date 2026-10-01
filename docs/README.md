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

## Design and project records

Each document in [design](design/) carries a `Status:` line. Check it before
treating a design as shipped behavior. The [roadmap](design/00-roadmap.md) is
historical; [the agent-owned platform](design/64-agent-owned-platform.md) is
the current product model. The data architecture and visual refresh
[plans](plans/) track approved work separately from shipped behavior.

The dated [audits](audits/), [reviews](reviews/), [research](research/),
[visual reports](uiux-audit-report.md), and [LinkedIn drafts](linkedin/) are
records of their own review dates. They can explain a decision, but their
measurements, line numbers, and open findings need fresh verification against
the current tree. The [video](video/) and [brand explorations](brand/) folders
contain production assets and working material.

## Checking a documentation change

Run `python3 scripts/check_doc_paths.py` and `scripts/check-version.sh` from
the repository root. The first checks cited repository paths; the second
checks version stamps. Compare behavior claims with current code and tests,
and check relative links and rendered layout when editing a page. Neither
script establishes that a statement still describes shipped behavior.
