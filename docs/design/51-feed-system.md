# Feed system

Status: **retired.** The Python pipeline this describes was deleted at
data-architecture M6.5c (2026-10-06) and replaced by intake
(`docs/design/76-intake-and-knowledge.md`, `crates/vak-intake`,
`vak_core::intake`). Read for history only; what follows was the
implementation contract for the **shipped 5.x Python pipeline**, and
the accurate description of it. Its **target model is superseded by
`docs/design/76-intake-and-knowledge.md`**, which folds feeds into one intake
path feeding the catalog (`docs/design/73-data-architecture-and-lifecycle.md`,
`docs/design/74-lifecycle-and-data-administration.md`); until that lands
(M6.5) this
is what runs, with the one exception of the security fix in doc 76 §9, which
lands independently and now. The feed system is a scoped knowledge extension,
not a source CRUD panel. It owns source discovery, fetching, normalisation,
quarantine, indexing, retrieval, alert evaluation, and the evidence needed to
operate those activities. Transport, permissions, and delivery remain
host-owned capabilities.

## Scope and identity

Feeds have two configuration scopes:

* `user`: inherited by every trusted workspace for the account;
* `workspace`: available only in one workspace.

Scope is explicit in every source, item, alert, run, and API response. A
workspace sees its own sources and inherited user sources. A workspace entry
may override or locally disable an inherited source by stable `source_id`;
display names are labels and are never identities. User configuration is not
copied into workspace configuration.

The effective source set is resolved before ingestion or search. A source
record contains its stable ID, scope, workspace identity when applicable,
driver configuration, enabled state, interval, trust declaration, and the
last operational state. Items retain the source ID, scope, workspace
visibility, canonical URL, external ID, content hash, publication time,
ingestion time, and security disposition.

Global content is visible to a workspace only through the workspace's
resolved feed view. Workspace content never becomes visible to another
workspace merely because the physical database is shared. Every query,
mutation, alert evaluation, and delivery lookup applies the resolved scope
at the storage boundary.

## Lifecycle

The runtime reconciles configuration into source records. Each source has a
due time and a bounded retry state. A scheduled pass fetches only enabled,
due sources in the selected scope. A manual run may request one source or all
visible sources, but it still records a run and uses the same permission and
security path.

Fetch outcomes are values: `succeeded`, `empty`, `failed`, `blocked`, or
`cancelled`. A fetch error is not an empty success. A run records its source,
scope, start and finish times, outcome, counts, error detail, and next retry
time. Concurrent runs for the same source are coalesced or rejected with an
explicit in-flight result.

Items are immutable observations. Re-fetching the same source item is
idempotent by `(source_id, external_id)` when available and otherwise by a
stable canonical URL/content fingerprint. Updates create a new observation
or revision record; they do not rewrite the original audit event.

## Security

Source URLs and every redirect target pass the host SSRF policy. Responses
have bounded size and time. HTML is sanitised before storage and model use.
Instruction-like content from an external source is data and never changes
the agent's authority, tools, prompt layers, or permissions.

Each item has a security disposition: `accepted`, `quarantined`, `blocked`,
or `unavailable`. Detection evidence is retained with the item and exposed
to operators, while quarantined content is excluded from model retrieval and
alerts until an explicit, permissioned operator action releases it. Hooks
are brokered host effects and never run as arbitrary feed subprocesses.

## Search and information architecture

The UI exposes Overview, Sources, Items, Search, Alerts, and Operations.
Overview reports real freshness and failure state. Sources distinguish user,
workspace, inherited, overridden, and locally disabled entries. Items show
provenance and security disposition. Operations shows run receipts and
delivery outcomes.

Search resolves scope and security visibility before querying the full index.
It returns only documents matching the query terms, then ranks by relevance,
freshness, declared trust, and evidence quality. Freshness or trust alone
cannot create a match. Filters for source, scope, tags, date, trust, and
security disposition are applied before ranking. Empty or non-matching
queries return an empty result set with explicit metadata.

## Alerts and delivery

Alerts have the same scope rules as sources. Evaluation is idempotent per
`(alert_id, item_id)`, records a match receipt, and marks delivery successful
only after the canonical host delivery runtime accepts or confirms the
delivery effect. Alert content is a typed delivery payload; feed code never
constructs or writes effect records directly.

## API contract

All feed endpoints accept or derive an explicit workspace scope and return
provenance, scope, operational state, and security state where relevant.
Configuration reads return the layer they write. Effective views are separate
from layer reads. Mutations are atomic and reconcile runtime state without a
restart. Errors preserve their status and reason; a successful HTTP response
cannot contain a hidden script failure or a fabricated empty result.

## Removal rule

The old split authority is retired: no handler may write one configuration
file while ingestion reads another, no source operation may update only a
database shadow, and no feed script may bypass the host delivery runtime.
Compatibility code is removed once the canonical path is live and covered by
regression tests.
