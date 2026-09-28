# 79 — Private headless fleet

Status: **proposal, 2026-09-28.** No customer fleet, operator-blind hosted
runtime, attested key release, or fleet panel described here is shipped. The
current single-owner EC2 installation remains a development baseline, not
evidence that the privacy and disaster-recovery acceptance tests below pass.
This document extends the local-first data model in
`73-data-architecture-and-lifecycle.md` and its milestone plan; it does not
start or reorder any pending milestone.

## 1. Product contract and trust boundary

One customer owns one dedicated, continuously running Vakyartha VM, sized
Small, Medium, Large or Extra Large. Several Agents belonging to that customer
may use its VM. No other customer's Agent, workspace, credential, worker or
service runs in it. The VM is the customer execution and content boundary,
not a `CorePool` entry or a workspace inside a shared server. Physical hosts
may carry separate VMs; the product never claims a dedicated physical host.
Size specifies reserved CPU, memory, storage and concurrent-run capacity,
validated with real workloads. It does not change the privacy contract.

"24/7" means the customer's endpoint and schedules are meant to remain
available while their device is off. It is not a claim of zero interruption:
host failure, upgrades, key unavailability and provider outages need explicit
recovery and measured service targets (§8).

The privacy promise has two parts:

1. **Between customers:** separate VM, state, encryption domain, network
   policy, service identity, backups and resource quota. A customer VM has no
   private route or credentials to another. A public endpoint still requires
   authentication; network separation alone is not authorization.
2. **From Vakyartha operators:** staff can manage deployment state, signed
   software and content-free health, but cannot read customer conversations,
   files, memory, secrets, browser sessions, raw logs or backup plaintext.
   There is no support-impersonation or emergency plaintext override.

This second promise is a **cryptographic capability requirement**, not a UI
role. Today an administrator of the hosting AWS account can change software,
obtain a shell, copy a volume/snapshot or change key access; ordinary EBS
encryption and the present headless encrypted-file credential fallback do not
deny that administrator plaintext. A customer-supplied key released to an
unattested, operator-replaceable process has the same problem. No hosted
customer is sold the operator-blind promise until a threat review and tests
prove the key-release, runtime and update chain. "No access through the
panel" alone is a narrower claim and may not be relabelled as zero access.

The chosen inference provider and authorized external connectors receive the
data needed for their calls. Their processing, residency, retention and
subprocessor terms are disclosed separately. The product must not imply
end-to-end secrecy from a provider to which the customer asked Vak to send a
prompt.

## 2. Control plane and customer plane

```
customer ── signup / billing / deployment status ── fleet control plane
   │                                                 │ desired operations
   └── stable HTTPS hostname ── Caddy ── loopback Vak │ signed version + health
                               dedicated customer VM ◀┘
                               ├─ Agents, broker, workers, schedules
                               ├─ tenant records, objects, refs, catalog
                               └─ tenant keys, secrets, encrypted backup
```

The fleet control plane stores deployment id, customer id, region, size,
hostname, signed artifact digest, resource references, operation history,
health and billing quantities. It stores no customer content, provider key,
gateway token, recovery code, raw transcript or filename. Customer content
and detailed run evidence remain in the customer plane. A central provider
registry is infrastructure configuration; the model list remains discovered
using the customer's authorized key (invariant 9).

The fleet panel and the customer's `/admin` are different authority domains.
Fleet operators may provision, resize, deploy an approved release, restart,
replace, recover or retire a VM through typed, auditable operations. They may
not run arbitrary commands, download volumes, browse raw logs, change owner
identity or edit the customer's Agent policy. Customer Admin configures
Agents, provider routes, channel access, permissions, retention, exports and
keys using Vak's existing APIs and authority rules. A fleet operation cannot
silently widen a customer's permission mode or connector access.

## 3. Provisioning and first claim

The fleet reconciles a deployment record through `requested → allocating →
installing → checking → claimable → active`, with explicit `failed`,
`recovering`, `suspended` and `offboarding` branches. Every step is
idempotent; retry looks up the recorded resource before allocating another.
The customer id and deployment id are stable. Paths, instance ids, IP
addresses and hostnames are attributes, never identity.

1. Admit region, size, hostname, cost and customer owner. Resolve available
   capacity and quotas before accepting a paid provisioning request.
2. Launch one dedicated VM from a versioned, tested ARM64 image or install
   artifact. Require encrypted storage, IMDSv2, least-privilege instance
   identity, a customer-specific network policy and an explicit resource
   inventory. Do not put secrets in image, user data, tags or logs.
3. Verify OS/service liveness, the manifest and signed release, Caddy TLS,
   loopback-only Vak listener and public readiness. A `200` from a generic
   health path does not prove owner claim or a model turn.
4. Issue a short-lived, single-use claim bound to the deployment and stable
   public origin. The customer enrolls the owner passkey under doc 78. The
   current operator `copy-token` bootstrap is not the fleet claim protocol;
   staff must not receive the gateway bearer. Failed and concurrent claims
   fail closed. Recovery remains customer-controlled.
5. Offer a minimal preset: one Agent, restrictive permissions, web access,
   no channel enabled, no provider key. The customer may add their own AI
   key through Vak's credential store and test a discovered route. A future
   Vakyartha-managed token route uses the same provider/receipt boundary and
   shows its spend limit and provenance. Missing AI setup is an actionable
   state, not a broken VM.

Initially preserve the proven per-VM Caddy and hostname. Vak remains on
loopback and 8901 is never public. The hostname stays stable through
replacement because a passkey is bound to its public origin (doc 78). A
future shared ingress must either pass TLS through to the customer VM or
explicitly narrow the privacy claim: an edge that terminates TLS can see
customer requests. Changing the ingress must not silently alter that claim.

## 4. Network and execution boundaries

Use a customer-specific security group and instance role; deny private
east-west traffic, metadata access from untrusted workers, and management
ports from the internet. A second customer must not become reachable by
visiting its public URL without its own authentication. Public address,
private subnet and DNS changes are tested as separate cases. Shared routing
or NAT infrastructure may carry packets, but cannot receive customer keys or
be represented as a shared customer runtime.

Egress has four independently governed paths: inference providers, channels,
approved Agent/tool destinations, and host update/telemetry endpoints. The
first two use their recipient-scoped credentials. The third uses Vak's broker
and permission engine; a shell, browser or plugin may not inherit the VM's
infrastructure identity. The fourth is never granted to the model. A network
policy is enforced outside the untrusted worker as well as at the broker.
Allowing one provider does not authorize arbitrary outbound traffic, and
blocking a connector does not automatically block the same website.

One VM does not mean one trust level inside it. Office/PDF parsers and Bash
remain worker capabilities under invariant 14; human approval, restricted
filesystem roots, cancellation and resource ceilings remain in force. The
fleet must never turn FullAccess on to repair a failed task.

## 5. Operator-blind keys and deployment placement

The customer controls a key-release authority that the fleet cannot alter.
Backups and remote objects hold ciphertext and wrapped keys. Release of a
decryption key is bound to a customer-approved software measurement and
deployment identity; operators cannot substitute an image or impersonate
the workload to obtain it. An update's new measurement requires the
customer's approval of that exact digest, or a standing policy enforced by
an independent authority that Vakyartha operators cannot change. A vendor
signature by itself only proves who signed a release; it does not prove the
signer could not introduce code that reads customer data. Key revocation
stops decryption, so availability depends on that authority.

Two placements may implement this contract, each requiring its own proof:

- **Customer-owned AWS account:** provisioning uses a limited, revocable
  cross-account role. The customer controls VM administration, snapshots,
  keys and release policy. Vak's fleet role has no shell, raw volume or
  decrypt access. Customer control of the account alone does not protect
  against a malicious auto-update; the customer or an independent update
  authority must approve the exact release measurement before key release.
- **Vakyartha-hosted account:** confidential execution, independent
  attestation and customer-controlled key release must prevent the hosting
  account's operator from using a changed image, root shell or copied disk
  to recover plaintext. Nitro Enclaves or attested confidential VMs are
  candidates, not shipped selections. A small `t4g.small` with today's
  credential fallback has not been shown to meet this contract. Host OS,
  broker, tool workers, file formats, telemetry and provider egress all
  need review; protecting a key in one enclave while exposing the decrypted
  prompt to an operator-controlled process would fail the contract.

The current data-architecture plan's tenant KEK and encryption-at-rest
default remain the local data model. A fleet placement must add a key
custody and attestation contract before claiming operator blindness. No
operator-held escrow copy is created by default. If a customer loses all
key-release and recovery material, we cannot recover its plaintext; the
panel must say this before activation.

## 6. Telemetry, support and privacy

| Central fleet may receive | Stays inside the customer boundary |
|---|---|
| deployment and opaque trace ids, release digest, health state | prompts, replies, transcripts, memory, files, previews |
| coarse CPU/RAM/disk and backup age, bounded counts/latency buckets | raw commands, stdout/stderr, URLs, filenames, document titles |
| typed provider/channel failure class and last-success time | API keys, bot tokens, cookies, browser sessions, recovery codes |
| signed operation/incident receipt without content | raw application/security logs and detailed Run evidence |

No free-form exception string crosses into fleet telemetry. Validate fields
against an allowlist, bound label cardinality and retention, and test with
secret-like and personal content. Opaque ids, IP addresses and timestamps
may remain linkable personal data; access, deletion, export and retention
rules still apply to them. Central traces do not include HTTP paths, query
strings or tool arguments. Diagnostic bundles are produced in the customer
plane with a preview of exact fields before export; the operator cannot
request or create a content-bearing bundle unilaterally. Troubleshooting
starts with measured health and failure class, then synthetic reproduction.

The customer receives its detailed audit and can verify an operation against
the signed fleet receipt. The fleet supports operational work without a
customer impersonation route. Offboarding removes the deployment and follows
doc 74's retention, hold and erasure rules; an operator cannot bypass a hold
through the EC2 terminate path. The fleet's own minimal billing/security
record has a declared retention period and purpose separate from content.

## 7. Software updates and routine operations

Build once from a locked source and produce a signed artifact with version,
commit and component digests (doc 32). A release pipeline tests install,
owner claim, a real turn, restart, backup/restore and rollback compatibility
on a sacrificial customer-shaped VM. Roll out by internal → pilot → cohort →
fleet, stopping when health or data-integrity gates fail. The customer can
pin a supported version or authorize a bounded automatic-update policy
enforced by the independent key-release authority. Fleet approval or a
vendor signature alone cannot qualify an update to decrypt customer data.
This is a new fleet authorization; the current local `self update` remains
opt-in and is not silently changed.

Before an update, capture a verified encrypted recovery point, stop new run
admission, let active turns settle or cancel with partial-output records,
and fence schedule/channel ownership. Install through the managed manifest,
check readiness and data baseline, then resume. A rollback is permitted only
when the prior binary understands the data and the customer's key policy
authorizes its measurement. Never move the version or supported baseline
backwards to force a rollback. Replacement uses the DR handoff below rather
than two active copies of the same VM state.

The panel offers typed actions: restart service, replace VM, resize, apply
approved release, rotate infrastructure identity, test backup, restore and
retire. It shows exact impact, current run count, last recovery point and
result. It does not expose a terminal or generic Run Command. Emergency
network isolation and admission pause may stop work; they do not grant
plaintext access.

## 8. Disaster recovery and continuity

The source of truth is the tenant's records, objects, refs, Desired state,
key grants and customer-owned secret references described in doc 73. VM
image, generated services and caches are reproducible; the running VM and
its root disk are not the only durable copy. A crash-consistent EBS snapshot
alone is insufficient when records, refs, key grants and channel cursors
were written at different times. Backup captures a coherent manifest with
record-chain heads, ref generation, object inventory, key-grant set,
desired-state revision, erasure watermark, software/data baseline and
checksums. The backup job proves every referenced object is present before
declaring success. The KEK/key-release authority is not copied to a
fleet-readable backup.

The backup is encrypted under customer-controlled keys and kept outside the
VM's failure domain. Use a separate availability zone for recovery copies;
cross-region copies require the customer's residency and key policy. Apply
retention and holds to backups. An erasure tombstone and watermark are
restored before any recovered record becomes readable, so an old backup
cannot resurrect erased data (doc 74). A missing key or missing object is a
failed restore, never a partial success.

| Failure | Detection | Recovery and proof |
|---|---|---|
| Vak process or channel bridge | service state + readiness + channel lag | service manager restarts; verify one consumer and delivery cursor |
| VM or availability zone | EC2 status + external synthetic probe | fence old VM, launch replacement, restore state in chosen region/AZ, move stable hostname, verify owner sign-in and one turn |
| corrupt disk, lost ref or missing object | chain/ref integrity + backup verification | restore last complete manifest; report the exact recovery point and missing interval |
| bad release | cohort health and data-baseline check | stop rollout; use only a compatible, customer-approved release, else repair forward |
| lost provider/channel access | typed auth/connection failure | notify customer; never reset their key or impersonate them |
| lost customer key authority | key-release health | pause admission and preserve ciphertext; customer restores authority; no operator recovery bypass |

Only one VM may own a conversation writer lease, schedule slot or channel
poller at a time. Replacement first fences the old generation, including
after a partition, then obtains a new epoch and resumes from a verified HEAD.
The old VM may not publish or deliver after losing its epoch. Store-and-forward
delivery and inbox receipts survive the move; externally completed actions
are not blindly replayed. Passkey origin stays the same. If the old VM is
unreachable, a tested fencing path must still prevent a delayed recovery
from creating two active writers.

The initial **design targets**, not current SLOs, are recovery point ≤15
minutes and service restoration ≤60 minutes for a single-VM/AZ loss when
the customer key authority and AWS region are available. Define separate
targets for regional loss and customer key outage before selling them. The
panel reports measured backup lag and drill results, never an inferred RPO
or RTO. Run scheduled restores to an isolated, non-public test VM and prove:
manifest integrity, `derive_messages()` equivalence, owner authentication,
schedule/lease fencing, one model turn, channel delivery dedupe, erasure
watermark and absence of cross-customer access. Delete the test VM through
the governed lifecycle after the drill.

## 9. Data protection and release gates

The first customer release requires a data-flow map across the VM, backup,
telemetry, chosen model providers, connectors, channel transports and fleet
processors; retention by class; customer export and erasure; subprocessor and
region/transfer disclosures; a processor agreement where applicable; and an
incident process with evidence and notification responsibilities. This is a
design and legal review requirement, not a claim of GDPR compliance. Data
minimization applies to telemetry even when it contains no conversation
content. Customer-held keys do not remove obligations for linkable metadata.

Acceptance scenarios before the operator-blind claim:

1. Compromise customer A's VM and try private networking, credentials,
   backup references and authenticated APIs for customer B; every attempt
   fails and the audit contains no B content.
2. Give a fleet operator the maximum production role. They can deploy an
   approved release and see only allowlisted telemetry; shell, raw logs,
   snapshot plaintext, key release and owner impersonation all fail.
3. Have an infrastructure administrator replace the binary, image or key
   request. Customer key release refuses the unapproved measurement; the
   original customer VM continues or stops safely. The test includes the
   hosted-account root path, not only the fleet UI role.
4. Insert secret, filename, URL and prompt canaries in every error and tool
   path. None appears in central logs, spans, metrics or diagnostic export.
5. Kill a VM mid-turn, during a schedule claim, during delivery and during
   an update. Recover on another VM with one writer, preserved partial
   output, no duplicated external action and measured RPO/RTO.
6. Restore a backup from before an erasure. The erased content stays
   unavailable; the receipt names any external processor beyond our reach.
7. Revoke customer key access. New reads and runs stop, backups remain
   ciphertext, fleet health stays available, and no operator bypass exists.

The current headless install, doc 73 milestones and this fleet design must
each retain their own status. Passing a unit test or encrypting an EBS volume
does not satisfy the customer-release scenarios above.
