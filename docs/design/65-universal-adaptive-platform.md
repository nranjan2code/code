# 65 — Universal and Adaptable Platform

Status: **implemented and audited — September 14, 2026; universal document ingestion, domain templates, living interactive outcome canvas, semantic memory distillation, multi-agent collaboration evaluations, and proactive agent scheduling landed.**

## 1. Product Decision

vak is a universal assistant operating with domain neutrality across engineering,
research synthesis, writing, operational triage, and quantitative analysis.

A user may ask a question, request a complex data extraction, initiate a multi-source research review, or delegate a multi-agent workflow. The interface presents calm, outcome-focused deliverables with living interactivity:
- Every table produced in conversation provides client-side sorting, search filtering, and instant CSV export.
- Documents across all major formats (Markdown, CSV, TSV, JSON, YAML, TOML, INI, ENV, HTML, XML) are ingested token-efficiently without external runtime dependencies.
- Specialist roles (`researcher`, `writer`, `operator`, `analyst`) bring dedicated epistemic stances as prompt layers; a worker under a role has the same tools as any worker.
- Agents durably own scheduled executions and record verifiable run receipts to append-only ledgers.

## 2. Core Architectural Components

### A. Living Interactive Outcome Canvas
1. **Interactive Table Engine (`InteractiveTable`)**:
   - Upgrades standard markdown tables into dynamic client-side data grids within the message stream.
   - Features:
     - Real-time row search filter (`Filter table...`) with active match indicators (`X of Y rows`).
     - Clickable column sort headers with numeric/alphabetical detection and direction indicators (`▲`, `▼`, `↕`).
     - One-click CSV export (`downloadCsv`).
     - Seamless preservation of rich inline markdown nodes (bold, code, links).
2. **Visual Aliases & Primitive Routing** (`STRUCTURED_RENDERERS` in `PresentationRenderer.tsx`):
   - `trend`, `timeseries`, `metric_chart`, `bar_chart`, `comparison_chart` all route to the `chart` primitive via `buildChartSpec` (`bar_chart` passing the forced `"bar"` override).
   - `decision_matrix`, `criteria_matrix`, `tradeoff_analysis` route to the `table` primitive via `buildTableSpec`, each with its own default title.
   - Aliasing is a registry entry, not a component: several semantic types deliberately share one `buildXSpec`/`renderX` pair (`docs/design/67-presentation-renderer-guide.md`).

### B. In-Memory Universal Document Ingestion (`doc_read`)
1. **Tool Contract**:
   - `path`: Target file within the canonical workspace.
   - `section`: Optional section heading (e.g. `'## Methodology'`, `'[server]'`).
   - `view`: `'text'`, `'table'`, `'summary'`, `'outline'`.
   - `offset` & `limit`: Paginated extraction avoiding context window blowouts.
2. **Supported Formats**:
   - **Tabular**: CSV, TSV, JSON array of objects.
   - **Structured Config**: TOML, YAML, INI, ENV, Properties.
   - **Markup & Web**: Markdown, HTML, XML (with tag-aware heading outline and `<table>` row scraper).
   - **Office and PDF**: Word, Excel, PowerPoint and Visio packages and PDF, read by `crates/vak-ooxml` and `crates/vak-pdf` (docs/design/72-openxml-documents.md, 77-pdf-documents.md).
3. **Boundary Confinement (Invariant 10)**:
   - Canonical target path resolution against `ctx.cwd.canonicalize()` fails closed on any path escaping the workspace root.

### C. Specialist Domain Archetypes & Delegation
1. **Domain Templates**:
   - **`researcher`**: Systematic inquiry, empirical evidence gathering, strict citations (`[1]`, `[2]`), source verification (leaf/calm).
   - **`writer`**: Publication-ready drafting, narrative coherence, tone adaptation, progressive refinement (wave/bright).
   - **`operator`**: Operational reliability, environment triage, runbook execution, system safety (sun/calm).
   - **`analyst`**: Quantitative reasoning, data aggregation, decision matrices, statistical synthesis (spark/quiet).
2. **Worker Role Inheritance**:
   - Roles admitted into `Core::prompt_role_names()` and `role_prompts`: the four built-in ones plus any `agents/<role>` prompt layer in the Shared layer or the project.
   - Workers dispatched via `task` automatically inherit domain archetype instructions in their prompt layers.
3. **Verification**:
   - `general_multi_agent_collaboration` eval case tests end-to-end multi-disciplinary delegation combining quantitative metrics, citations, and executive synthesis.

### D. Scheduled work
Scheduled work is an automation (`vak_core::triggers::Trigger`: a prompt or a script on a schedule or an event, owned by the Agent it names), started only through `triggers::claim_due` and run by the server (`crates/vak-server/src/automations.rs`), with a run record for every start. An earlier `AgentSchedule` field on the Agent definition and its `agents_runs.jsonl` ledger were stored but never run, and were removed in the data architecture plan's M0 (`docs/plans/data-architecture-plan.md`).

## 3. Invariants & Guarantees

1. **Workspace Boundary**: All file reading and document extraction strictly resolve inside the canonical workspace.
2. **Non-destructive Extension**: Living outcome canvas and document ingestion enhance assistant capabilities without altering append-only session ledgers or per-turn route receipts.
3. **No external runtime**: Document parsing and tabular conversions are pure Rust with no outside program, run in the broker worker (invariant 14), never in the server, desktop or gateway process.
