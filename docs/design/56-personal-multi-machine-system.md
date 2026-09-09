# 56 — Personal multi-machine vak

Status: **proposed — implementation plan**, 2026-09-09. Source inspection only;
this document does not certify deployed security or distributed capacity.

## Product contract

One owner operates one or many machines and many agents through the existing
desktop, browser, CLI, headless and channel surfaces. Each machine runs the
same vak runtime. A machine remains useful independently, including offline.
Connected machines provide a combined work list, inbox, administration and
observability. The owner chooses execution location; existing admission,
permissions, outcome contracts and resource limits govern execution there.

The first connected installation is the developer's Mac plus one AWS host.
`https://vak.vakyartha.com` is its browser entry point. One designated host
coordinates connected work; initially this is the always-on AWS host. The Mac
is not a mandatory control plane. Neither login nor cloud availability gates
ordinary local use. Additional owners, organizations and shared tenancy are
outside this release.

Thousands of agents is a capacity target to measure. Distinguish retained
tasks, runnable tasks, provider requests, live processes and open UI streams;
one thousand task records does not require one thousand processes.

## Existing foundations and verified gaps

Inspected against the current dirty working tree; unrelated edits must be
preserved. Design statuses are context, not proof of production enforcement.

| Foundation | Reuse | Required work |
|---|---|---|
| `vak-client-ui/src/host/{port,web,tauri}.ts`, `api.ts` | Shared client and host adapters | Split device affordances from execution destination. Current Tauri calls operate locally and API credentials use a module-global backend. |
| `vak-server/src/web.rs` auth | Existing `/auth/*` entry points | Current login copies the server bearer into the cookie; logout clears the browser cookie. Add independent revocable sessions with server-enforced expiry. |
| `vak-server/src/lib.rs` `active_core`, workspace routes | CorePool and canonical path checks | Resolve workspace per request/session. Shared active selection must not let another tab redirect work. |
| `vak-server/src/bus.rs` | Server event adapter and local mode | `emit` uses the preceding sequence string as `prev_hash`, ignores encryption errors, and substitutes `{}` on serialization failure. Repair before claiming protected distributed delivery. |
| Same bus adapter | Existing status projection | `connected` currently means distributed backend selected; `encrypted` means secret configured. Report observed connection state and actual enforcement, including failed reconnects. |
| `vak-bus` | NATS/JetStream, envelope and queue interfaces | Verify broker-enforced permissions, acknowledgements, replay, restart behavior and node-bound namespaces against a real broker. Library ACL helpers alone are insufficient. |
| `vak-ops`, existing Operations Center | Manager probes, incidents, action receipts, outbox | Extend existing projections with node scope and partial availability. |
| Sessions, managed work, budgets, sandbox | Existing source of truth and execution boundaries | Add placement and remote dispatch to existing admission, not a second agent loop or task scheduler. |

Further audit must trace production callers; the findings above are specific
source observations, not an exhaustive vulnerability assessment.

## Architecture and responsibility

Every host runs one logical node authority with a stable `node_id`. Existing
embedded desktop servers and durable gateway processes must register with or
delegate to that authority: two processes on one host cannot independently
own the same scheduled job, run or node-wide budget. Preserve the present
launchd/systemd lifecycle and deliberately resolve duplicate ownership before
enabling connected execution.

The coordinator is a role of the existing server. It owns enrollment,
connected-task placement, bounded budget allocations and the combined index.
It does not own every local task. Each execution node owns its runs, workspace
mapping, admission, sandbox, credentials, canonical ledgers and service state.
The combined index is rebuildable from node evidence.

Clients reach the coordinator over HTTPS. Enrolled nodes initiate authenticated
outbound connections, allowing the Mac to remain behind its existing firewall.
Use the existing bus for node events and durable work; do not introduce a second
durable queue. HTTP remains the client/control contract. Node broker connectivity
is a separately secured listener with TLS and node-specific broker permissions;
its public/private exposure must be explicit in the deployment manifest. NATS
monitoring and administration are never public. Do not promise that 443 alone
covers this topology unless a tested transport configuration actually does.

Standalone mode uses local execution without a broker dependency. Connected
mode uses one NATS/JetStream service initially. A failed distributed connection
may preserve local work, but cannot silently turn accepted remote work into an
in-memory queue. One coordinator is an explicit availability tradeoff; automatic
leader election, multi-region replication and transparent failover are deferred.

## Small set of identities

- `owner_id`: stable installation ownership, independent of email and provider.
  An explicitly linked `(OIDC issuer, subject)` authenticates the owner remotely.
- `node_id`: machine enrollment identity, distinct from hostname/IP. Restoring a
  backup on another machine requires re-enrollment to avoid cloned identities.
- Workspace reference: node ID plus node-local workspace ID. Optional explicit
  project association links checkouts across machines; equal names or paths do
  not establish identity or grant trust.
- Existing task/session/run/attempt IDs: retain their semantics; remote
  references add node scope. Execution attempts carry placement generation.
- Command ID: deduplicates administrative requests and exposes their receipts.

Agents are runtime work with brokered capabilities, not thousands of human
accounts. Inside a machine, existing scoped handles and workers suffice.
Credentials for a node cannot create another node or impersonate the owner.

## Authentication and enrollment

Local use preserves explicit local operator trust and offline operation. A
local operator becomes associated with the connected owner only through an
explicit pairing action. A remote request declaring a loopback Host or passing
through a loopback reverse proxy must never obtain local auto-login authority.
Audit peer verification, trusted proxy behavior, exact Origin checks and public
route exemptions before deployment.

For the first public installation, use one configured OIDC provider (Cognito)
and one admitted owner identity. Disable public registration. The server owns
the authorization-code exchange, state/nonce/PKCE verification, fixed callback
validation and independent random session issuance. Pin issuer/client and
validate token type, signature and expiry with a maintained implementation.
Use secure HttpOnly browser cookies, server-side expiration, logout revocation
and a device/session list. MFA and account recovery are configured at the IdP.
The main session cookie must support the chosen callback flow without weakening
CSRF checks; any temporary callback cookie has an explicit narrow lifetime.

Desktop/CLI login opens the system browser and completes a bounded, single-use
local callback/pairing exchange. Store persistent credentials in the OS secure
store where available, otherwise a permission-restricted credential store.
Desktop transport handles authentication and streaming natively; remote bearer
tokens never go into EventSource URLs. Browser cookies and native credentials
resolve to the same request identity and authorization path, not necessarily
the same wire transport.

Enrollment: owner requests a short-lived, single-use invitation, node presents
its generated identity, owner verifies the node fingerprint and approves it,
coordinator issues node-scoped credentials. Support rotation, expiry and
revocation. Secrets never appear in process arguments, transcripts or tool
environments. Connected operation refuses expired authority. Revocation blocks
new connected work immediately at the coordinator and is bounded on disconnected
nodes by their existing lease expiry; an unreachable machine cannot be remotely
stopped instantly. Independent owner-started local work remains a local matter.

Human login, node transport and machine integrations share authorization
resolution but have distinct credentials. Replace superseded credential paths
deliberately under repository version/schema rules; avoid compatibility aliases
that preserve duplicate authority. Any required breaking change needs a major
release decision before implementation.

## Work, ownership and failure semantics

Extend existing managed-work dispatch with explicit placement. Initial placement
is user-selected, with a persisted default per workspace. Capability discovery,
frozen model route, permission ceilings and budget admission remain authoritative
on the destination. Missing inputs or providers yield a reason, not silent
rerouting or credential copying.

Remote command progression is durable:

`requested → accepted → executing → succeeded / failed / cancelled`

Timeout or disconnection produces `unknown/pending reconciliation`, not success
or confirmed cancellation. Persist acceptance and the attempt identity before
dispatch, acknowledge durable messages only after the appropriate record is
committed, and replay using command IDs. Existing run ledgers remain canonical;
the broker transports requests and evidence.

One node owns an active attempt. A lost lease is not proof that an external
effect did not happen. Do not automatically reassign ambiguous effectful work.
Reconcile receipts and provider/connector idempotency where supported, otherwise
require explicit resolution. The system does not promise exactly-once external
side effects. Old placement generations cannot commit new connected control
decisions. Work with irreversible effects remains subject to the existing
approval contract.

Remote work can continue through a brief partition within its admitted lease,
capabilities and reserved budget. Renewal failure blocks further dispatch at
expiry; local cancellation preserves partial output. Approval requests expire
closed and bind owner, node, run, exact operation digest and policy generation.
Repeated or stale approvals cannot authorize another operation.

## Capacity and spending

Extend the current scheduler/admission path with per-node ceilings for runnable
work, provider concurrency, processes, memory, CPU, disk, output and queue size.
Reserve capacity for control, approvals and cancellation. Apply backpressure
and show why work is queued. A child task consumes the parent's existing
authority and spending reservation; it cannot multiply them by fan-out.

The coordinator allocates bounded spending slices to connected nodes. A node
cannot allocate new global budget while disconnected or overspend its slice.
Independent offline-local budgets are explicitly separate unless previously
reserved from the shared total. Reconcile measured receipts on reconnect;
report unknown provider costs honestly. AWS billing alerts are notifications,
not real-time spending caps.

## Data and configuration

Keep authoritative session ledgers on their execution node. Replicate durable
summaries/receipts and selected artifacts to the coordinator for the combined
view, with node cursor, checksum, deduplication and provenance. Rebuild indexes
without rewriting history. Offline views show the last observation time and
which detail is unavailable.

Preserve Shared/project/scoped configuration semantics on each machine. Add
explicit publication of selected portable preferences into a node's existing
Shared layer with compare-before-write revision checks. Machine paths, trust,
sandbox configuration, service settings and secrets stay machine-specific.
The admin UI previews the exact destination layer and effective change before
publishing. Background synchronization cannot expand permissions.

Memory sharing uses selected, provenance-preserving publication through existing
memory/learning contracts. Do not treat peer content as trusted instructions.
Source code handoff uses Git; universal artifact handoff uses hashed manifests
and the review/acceptance contract in design 54. A conversation can continue on
another node by a new branch with a recorded context packet, explicit inputs
and fresh admission. It does not move running processes or silently remap paths.

## Unified daily experience

The default connected surface is **All my work**, independent of which device
renders it. Desktop sees cloud and other desktop work; the cloud browser sees
desktop and other cloud work. Every enrolled node publishes its owner-visible
task/session index, status and attention events to the same combined projection.
There is no requirement to switch backend to discover another machine's tasks.
Machine and workspace filters are optional views of this shared collection.

Opening a task resolves its owning node automatically. Reads, streaming,
steering, approvals and cancellation route through the same authenticated
control contract and retain the task's execution destination. Browser clients
never need direct inbound access to a laptop: the enrolled node's outbound link
carries requests and responses. Viewing a task from another device does not
move its execution or widen access to its files and tools.

Replicate owner-visible conversation history incrementally into a read-only
projection, with source entry IDs, ordering, checksums and durable cursors, so
already synchronized conversations remain readable when their machine sleeps.
Canonical ledgers stay on the source node; the replica cannot independently
advance the conversation. Surface unsynchronized ranges explicitly. Large
artifacts follow explicit retention/replication policy; live terminals, previews
and unreplicated files require the owning node to be reachable. Show that
distinction in place without hiding the task or reporting stale state as live.
Enrollment explains this personal-system sharing; excluded/private work stays
local and is visibly marked as such.

Acceptance scenario: create one task on the Mac and another on AWS. Both appear
in the same list in desktop and at `vak.vakyartha.com`; either client can inspect,
steer and cancel either reachable task. Put the Mac to sleep: its synchronized
history remains readable, live controls report unavailable, and AWS work keeps
running. Reconnect the Mac: the index and history catch up without duplicates.

The shared client shows one personal work list, grouped/filterable by machine
and workspace. Every task, approval, terminal, preview, setting and action has
an unambiguous execution destination. Machine choice is a compact control next
to workspace selection; a cloud filesystem picker lists cloud files even when
used inside the desktop. Native dialogs and notifications remain device features.

Task navigation binds node/workspace/session in the URL and client state. Open
two machines or workspaces in separate tabs without changing either tab's target.
Switching views does not cancel work. Reconnection resumes from durable cursors;
missed ephemeral events trigger rehydration. Preserve unsent drafts locally but
never automatically resubmit ambiguous commands after a disconnect.

Existing workspace UI owns doing/reviewing work; existing administration owns
system management. Share navigation, identity and selection context. Extend the
existing Operations Center rather than making another dashboard.

## Administration and observability

| Existing area / extension | Evidence and controls |
|---|---|
| Overview | Nodes online/stale/unreachable, runnable/queued/blocked work, spend reservations, incidents requiring attention |
| Machines | Enrollment/revocation, versions, runtime capabilities, manager state, resource pressure, drain/resume, maintenance |
| Work | Task/run/attempt lineage, execution location, queue reason, last progress, cancel and reconcile |
| Approvals/inbox | One deduplicated attention list with exact destination, deadline, scope and verified resolution |
| Runtime/channels | Broker connection/replay lag, services, delivery outbox, subscription/consumer failures |
| Providers/spend | Node-specific configured availability, frozen routes, observed usage, reserved versus actual costs |
| Security/settings | Active sessions/devices, node credentials metadata, policy revisions, exact-layer config changes |
| Incidents/recovery | Source evidence, action receipts, backup freshness, restore verification, update/rollback state |

Each control uses the same typed API through desktop, browser and CLI. Bind it
to a target and expected revision; persist actor, command ID, before/after state
and verified/pending/failed outcome. Remote service operations delegate to the
destination manager. A health timeout never starts a competing supervisor.
Drain means stop new admission and allow existing work to settle. Emergency
stop reports acknowledgements by node and marks unreachable nodes unknown.

Every diagnostic event carries available owner/node/workspace/task/run/attempt,
trace and command IDs, plus source sequence and observation time. Do not order
cross-machine causality by wall clock alone. Separate durable audit/receipts,
bounded diagnostic logs, sampled metrics and transient token/terminal streams.
Keep model-visible content reconstructable in sessions, but avoid duplicating
prompts, keys and tool contents into general logs. Redact before persistence
and export; protect log viewing and downloads with authentication.

Measure queue age, admission/rejection rate, provider latency/retries, active
workers, resource pressure, stream backlog, reconnect/replay lag, cancellation
latency and backup age. Aggregate idle/background tasks; stream detailed process
telemetry only for inspected runs. Persist incident transitions and use the
existing inbox for meaningful changes. Missing observations must remain unknown.

## Deployment, security and recovery

First install: one EC2 host, Caddy, vak and the connected-mode broker supervised
by systemd, persistent encrypted storage and one managed human IdP. Caddy can
manage its TLS certificate; a separate ACM certificate is not inherently needed.
DNS points `vak.vakyartha.com` to the selected stable endpoint. Validate the
broker listener's TLS/auth/access policy separately from the web proxy.

Protect the AWS account and domain registrar with MFA and recovery codes. Use
temporary operator credentials and SSM administration. Agent workers cannot
reach instance metadata, management sockets, node credentials, policy stores
or provider secrets. IMDSv2 alone does not isolate same-host workers: enforce
metadata/network exclusion at the execution boundary. Keep runtime identity
minimally privileged and release binaries unwritable by agent execution.

Before public exposure, inspect every unauthenticated endpoint, proxy header
handling, artifact renderer, download and preview route. Public health discloses
only minimal liveness; operational detail requires authentication. Verify sandbox
and permission revocation using real Linux workers, including non-Bash tools.
One trusted owner does not make untrusted model/tool content trustworthy.

Back up ledgers, coordinator enrollment metadata, config and selected artifacts
outside the instance. Protect backups from deletion by the execution identity;
bound retention and cost. Treat credential recovery separately and re-enroll
restored nodes. Validate integrity and restore into an isolated installation.
Updates drain the destination, verify release artifacts, preserve data, restart
through the manager and report readiness. Refuse unsupported schema versions;
rollback only to binaries that can read the persisted schema. Pin compatible
wire protocol versions and reject incompatible mutation before dispatch.

## Delivery sequence and acceptance gates

| Slice | Concrete deliverable | Gate |
|---|---|---|
| 0. Baseline | Trace auth, workspace routing, bus, scheduler/daemon ownership and admin routes; repair observed defects | Two tabs cannot redirect each other's work; expired/revoked sessions fail; crypto errors fail closed; real hash links and observed broker state |
| 1. Local node contract | Stable node/workspace references, explicit request context, destination-aware client adapter | Existing local desktop/CLI/headless work offline; two local processes cannot duplicate job ownership |
| 2. Human access | One-owner OIDC, revocable sessions, native login/transport, protected public endpoints | Login/logout/expiry/recovery, CSRF/rebinding/proxy spoofing and remote streaming tests; no secret in URL |
| 3. Two-node control | Enrollment, real broker credentials, outbound node link, durable commands and receipts | Revoke, reconnect, duplicate delivery, restart and stale command tests against two real nodes |
| 4. Unified operations | Machine/work views, cross-node inbox, evidence drilldowns, drain/cancel/config publication | Every action has target and receipt; disconnected nodes never show false success; one workflow works on desktop and browser |
| 5. Remote execution | Placement through existing scheduler, attempt ownership, reserved capacity/budgets | Lost-ack and partition tests cannot duplicate known effects or expand permission/budget; ambiguous effects require reconciliation |
| 6. Continuity/recovery | Artifact handoff, context branch, backup/restore, version negotiation and rolling manual updates | Local-to-cloud-to-local task retains provenance; recovery preserves ledgers and avoids credential clones |
| 7. Capacity qualification | Repeatable benchmark and adversarial failure suite | Publish tested limits for queued tasks, active inference and processes separately, including control responsiveness under saturation |

First public release requires slices 0–4 plus deployment/restore verification.
It can operate independent local/cloud tasks with unified control. Advertise
distributed task placement only after slice 5; advertise cross-node continuation
only after slice 6. Production scale claims require slice 7.

Capacity tests progress through 10, 100 and 1,000 retained/queued tasks, then
separately increase active mocked inference and bounded execution on measured
hardware. Suggested initial gates: p95 control API below one second under the
declared supported load; connected cancellation acknowledgement within two
seconds; bounded memory/queues during a slow consumer; no lost acknowledged
commands through process crashes. These are proposed test targets, not current
guarantees. Paid inference is separately budgeted and never needed for load CI.

Exercise coordinator loss, node sleep, duplicate messages, out-of-order events,
disk full, clock skew, worker crash, policy revocation, credential rotation,
stale approval, auth expiry during SSE, partial restore and version mismatch.
Include real AWS/Linux smoke verification after deterministic local tests.

## Engineering boundaries and completion

Keep orchestration in the existing managed-work/core path; transport in
`vak-bus`; node routing/auth/control in `vak-server`; service lifecycle in
`vak-ops`; records in existing session/store contracts; execution in the broker
and `vak-sandbox`; presentation in the two existing client/admin applications.
Create small modules as needed, not a service per concept. No Kubernetes,
automatic leader election, global filesystem, general multi-tenant RBAC,
custom cryptography or second scheduler is required for this plan.

Implementation slices must carry production-path tests, an operational failure
reason, evidence in the existing admin view, and documentation matching shipped
behavior. Completion means the owner can work on the Mac, let AWS continue while
the Mac sleeps, inspect and control both from desktop or browser, identify where
every action happened, recover after a failure and revoke a machine without
granting agents administrative authority.
