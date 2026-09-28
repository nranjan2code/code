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
2. Reserve one prepared, unused VM when available; otherwise launch one
   dedicated VM from a versioned, tested ARM64 image or install artifact.
   Require encrypted storage, IMDSv2, least-privilege instance
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

### 3.1 Seconds-level assignment from an unused VM pool

The fast path prepares capacity **before** signup, not customer state before
consent. Keep a small number of running, unused VMs per region and size,
installed from the same measured release and verified with a synthetic
readiness check. Each pool VM has its own instance identity, network policy,
storage encryption domain and HTTPS endpoint. It holds no customer content,
provider key, owner credential, bot token or shared TLS private key. A pool
VM is not a shared customer runtime: it is assigned to one customer at most.

The fleet atomically changes `ready_unassigned → reserved → bound` with a
single deployment id and a one-use reservation. Concurrent signups cannot
claim the same VM. A failed or timed-out reservation is reconciled against
the VM and key authority before any retry; once customer binding or claim
material was issued, retire the VM through the governed offboarding path.
Never put a previously assigned VM back into the pool. Replenish the unused
pool asynchronously and fall back to a cold launch when a region/size pool
is empty; show the resulting wait instead of promising the same latency.

Preassigning a hostname and issuing an individual certificate to each unused
VM can move DNS and TLS work out of the signup path. Do not copy one wildcard
TLS private key to all customer VMs or terminate customer traffic at the
fleet panel. The hostname must remain the customer's origin after VM
replacement; certificate key handling, renewal, DNS cutover and attested
key release need their own measured checks. A prepared HTTP service is not
equivalent to a customer-authorized plaintext runtime.

The panel reports four separate timestamps: VM reserved, authenticated HTTPS
reachable, owner claimed, and first successful AI turn. Only the first two
are candidates for a seconds-level target; claim time depends on the person,
and an AI turn depends on their key, chosen provider and its availability.
Measure p50/p95/p99 by region and size, including pool misses, certificate
renewal and key-authority delays, before setting a public promise. Keep the
pool small enough that idle running-VM cost is explicit; stopped warm
instances cost less but still incur start time and must pass readiness again.
AWS warm-pool lifecycle hooks must prove setup finished before an instance
enters the usable pool: [AWS warm-pool guidance](https://docs.aws.amazon.com/autoscaling/ec2/userguide/ec2-auto-scaling-warm-pools.html),
[lifecycle hooks](https://docs.aws.amazon.com/autoscaling/ec2/userguide/warm-pool-instance-lifecycle.html).

### 3.2 Signup, login and owner recovery

The fleet portal and each customer VM are separate relying parties and
authorization domains. A portal account can order a size, pay, see deployment
status and perform permitted infrastructure actions. It cannot mint a Vak
browser session, read an Agent, change a provider key or reset the VM owner.
The VM's owner signs in at its stable HTTPS origin with doc 78's passkey and
revocable, server-side browser session. The fleet portal may use passkeys or
an enterprise identity provider for its own login; email verification is for
notifications and account contact, not proof of authority over VM plaintext.

The proposed customer journey is:

1. Create a portal account, verify the contact address, enroll a passkey,
   choose region/size and accept the displayed recurring cost, residency and
   recovery limits. Payment and anti-abuse admission precede allocating
   scarce capacity. Billing credentials stay with the payment processor.
2. Reserve one VM (§3.1) and bind the immutable customer/deployment ids.
   Expose deployment status through the portal without publishing a gateway
   bearer, instance secret or unclaimed Admin surface.
3. Give the authenticated purchaser a short-lived, single-use claim that is
   bound to that deployment, exact HTTPS origin and intended owner key.
   Redeeming it atomically enrolls the first owner passkey on the VM. A
   second claim or a race must fail; successful claim revokes all bootstrap
   material. The claim is never sent to fleet staff or used as a login token.
4. The customer stores independent recovery material, adds a second passkey
   where possible, then connects its AI provider key inside the VM. Portal
   signup, VM claim and a working provider route are distinct states.

For the operator-blind promise, an operator-controlled portal assertion alone
cannot establish the first owner or release customer decryption keys. The
key authority must require a customer-held credential and attest the VM's
approved measurement/deployment identity before releasing keys. A malicious
portal or changed web client must not be able to impersonate the customer or
silently add an operator passkey. Exact enrollment and key-release proofs are
release-gate work under §5, including a threat test against a compromised
fleet portal. Until they pass, the panel must not claim operator blindness.

Recovery is customer-controlled. An unused code or another enrolled passkey
can restore owner access under doc 78; recovery of the decryption authority
is separate and must use customer-held material or its independent policy.
Email, billing support and fleet administrators cannot reset either into
plaintext access. Loss of every owner authenticator and key-recovery path may
make data permanently unreadable, and signup must say so plainly. Sensitive
changes such as adding a passkey, rotating recovery, changing key policy,
exporting data or deleting a deployment require fresh owner authentication
and a recorded, content-free decision. A recovery session has only the
capability needed to restore a passkey until step-up succeeds.

The initial product keeps doc 78's **one owner per VM**. Portal organization
members may have billing or fleet-operation roles without access to Vak data.
Enterprise OIDC/SSO, invitations and SCIM are later work: no teammate is
given a VM session until every content API, websocket, event stream, secret,
Agent and approval has tenant- and role-scoped authorization. Federation
must bind an immutable subject to a customer-approved owner and validate
issuer, audience, state, nonce and PKCE; an email match is insufficient.

Rate-limit and audit signup, claim, passkey challenges and recovery without
logging tokens, assertions or personal content. Sessions are revocable and
scoped to one plane and origin; neither a portal cookie nor a VM cookie is
accepted by the other. Exact-origin checks guard state changes, untrusted
Host/proxy headers fail closed, and public readiness exposes no customer
configuration. Security alerts about a new passkey, recovery use, changed
key policy and failed claims go to customer-controlled contacts without
including secret material. [NIST SP 800-63B](https://pages.nist.gov/800-63-4/sp800-63b/authenticators/)
describes WebAuthn's verifier-name binding; its recovery guidance is a review
input, not a claim that this proposal has reached a NIST assurance level.

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

### 7.1 OS, stack and security patch lifecycle

Track each VM against an approved image/manifest: OS base and kernel,
installed packages, Vak binary and worker artifact, Caddy, channel bridges,
backup agent and configuration schema. Inventory stores component versions,
build digests, patch state, last verified backup and customer-approved
measurement, never package-manager output or application logs. Security
advisories create a patch decision with severity, affected image cohort,
deadline, owner and exception expiry. A pinned Vak version does not mean an
unbounded right to run a vulnerable OS; disclose the supported patch window
and stop serving an unsafe version if its customer policy cannot authorize a
safe image. Do not silently bypass the key policy to meet a patch deadline.

Use an image pipeline for the normal path: build a patched ARM64 image from
reviewed inputs, test OS boot, broker sandbox, Caddy, Vak, migration,
backup/restore and attested key release, then sign the exact result. [EC2
Image Builder's patch guidance](https://docs.aws.amazon.com/imagebuilder/latest/userguide/security-patch-management.html)
supports this pattern. Replacing a VM from that image avoids an unbounded
series of live package changes and gives a reproducible rollback candidate.
The new VM stays non-public and unable to poll channels or run schedules
until it has the approved key release, a current recovery point, a passing
integrity check and the new fenced writer epoch. Then move the stable origin
and observe it through a soak period before retiring the old VM. One
customer still has only one *active* VM; a short-lived replacement is an
isolated, non-serving recovery candidate and never another customer's host.

An emergency in-place OS or component patch is a separately approved path
when replacement is not timely. It requires a verified recovery point,
drained runs, a bounded maintenance window, pinned package inputs, reboot
plan, post-patch measurement and customer key-policy approval before data
is reopened. Patch success is service readiness plus data integrity, not
only a package-manager exit code. [Systems Manager Patch Manager](https://docs.aws.amazon.com/systems-manager/latest/userguide/patch-manager.html)
can report missing patches or automate installation, but general remote
command execution and its output are incompatible with the proposed
operator-blind fleet role. Do not enable it on customer VMs merely for
convenience. Any narrow patch integration needs the same typed action,
output suppression, attestation and privacy tests as the rest of the fleet.

Use small rollout cohorts, concurrency limits, customer maintenance windows,
release freeze on rising failures and an explicit emergency path. Expose
next maintenance, patch age, image baseline, last update result and support
deadline in both the fleet and customer portal. A customer notification
names expected interruption and recovery point. An update failure leaves
the previous fenced VM serving if compatible and safe, or leaves service
paused with an incident and a repair-forward plan; it never starts two
writers or restores a pre-erasure backup.

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

Back up the canonical data home and registered durable state, including
Agents, sessions, memory, tasks and cursors, workspaces, configuration,
owner passkey public records, encrypted credential material, object/ref
stores, and the erasure/hold ledger. Record excluded caches and transient
files explicitly. Preserve the public origin and certificate continuity,
but never put a reusable TLS private key or a plaintext decryption key in
fleet-readable storage. The future doc 73 schema uses the coherent manifest
above; until it ships, a backup adapter must enumerate
`vak_core::state::REGISTRY` and every canonical path and prove that nothing
durable was omitted. A passing EC2 snapshot alone is not this proof.

### 8.1 Encryption, acknowledged writes and the last byte

Encrypt customer records, objects, credentials and backups **inside the
customer trust boundary** before they reach remote storage. Use separate
customer key domains and versioned key grants; keep plaintext keys out of
fleet IAM, tags, user data, logs and backup manifests. TLS protects transit;
EBS and backup-service encryption add defense in depth but do not provide
operator blindness by themselves (§5). A key rotation must rewrap every
retained version needed for restore before retiring an old grant. Verify
checksums and object/record references on write and again on restore; an
encrypted but missing or corrupt byte is still data loss.

Define a customer-visible **saved** acknowledgement precisely. The eventual
strong durability mode acknowledges a durable mutation only after its local
transaction is committed and the encrypted payload, required objects and
ordered commit marker have reached an independently durable off-VM store.
The commit marker is written last and names the exact ledger/ref generation
and checksum set. A restore chooses the latest *complete* marker and rejects
partial uploads. This is the path to recovering every acknowledged byte
after losing a VM or availability zone, when the remote store and customer
key authority are available. [S3 successful PUTs are strongly
consistent](https://docs.aws.amazon.com/AmazonS3/latest/userguide/Welcome.html),
but that property does not make a multi-object Vak transaction atomic; Vak
must supply the commit protocol and verification. Every durable write path,
including file tools, credential changes, browser owner state, schedules,
channel cursors and external-action receipts, must participate. A direct
local file write outside it invalidates the guarantee. Doc 73's later
milestones are prerequisites; today's single-instance 4.x tree cannot
honestly promise zero loss of acknowledged writes.

If the remote durability path is unavailable, choose explicitly between
pausing writes (preserve the strong acknowledgement contract) and a
customer-enabled degraded local-only mode that states the current data-loss
exposure. Never show a local-only write as fully protected. Reads and
already-running delivery may continue only where their own safety permits.
Network partitions, disk-full, process crash, partial multipart upload and
power loss must be tested at every acknowledgement boundary. No finite
design can recover bytes that were still in a browser buffer, uncommitted
worker output or an external provider when the failure occurred; preserve
and label partial output whenever it reached Vak (invariants 1 and 5).

Continuous off-VM durability is distinct from backup history. Keep
versioned, policy-retained recovery points so a logically corrupt write,
ransomware or mistaken deletion can be rolled back without discarding the
current erasure ledger. Immutable retention, if used, holds ciphertext only
and must be reconciled with customer erasure and crypto-shred; it is not a
license to resurrect deleted content. Cross-region replication is a separate
consent and measured-RPO decision, not proof of zero regional loss.

To meet the proposed 15-minute RPO, schedule recovery points often enough
and measure *committed source data through verified off-VM backup*, not job
start time. A daily full plus frequent incremental transfer is one candidate,
subject to workload tests. Use application-aware quiesce or a recorded
sequence/watermark so SQLite and file/object refs agree; copy ciphertext and
wrapped keys only. Keep at least one recoverable copy in a separate failure
domain and test the chosen retention against ransomware, accidental deletion,
key revocation, legal hold and crypto-shred. Backup deletion follows doc 74;
an old copy cannot become an erasure escape hatch. Cloud backup services
may schedule and retain encrypted volumes, but customer data restore still
requires the logical manifest and independent key-release contract.

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
minutes for a periodic-backup first release and service restoration ≤60
minutes for a single-VM/AZ loss when the customer key authority and AWS
region are available. The later strong durability mode in §8.1 targets no
loss of *acknowledged* writes; it cannot be sold until every write path and
restore test satisfies that contract. Define separate
targets for regional loss and customer key outage before selling them. The
panel reports measured backup lag and drill results, never an inferred RPO
or RTO. Run scheduled restores to an isolated, non-public test VM and prove:
manifest integrity, `derive_messages()` equivalence within the protected
runtime, owner credential records, schedule/lease fencing, delivery cursors,
erasure watermark and absence of cross-customer access. Return only a signed,
content-free validation result to fleet staff. Keep real channels, schedules
and provider credentials disabled in the drill; a synthetic provider and
fixtures exercise a turn. A periodic customer-assisted drill can additionally
prove actual passkey sign-in and a real authorized route. Delete the test VM
through the governed lifecycle after the drill. [AWS Backup restore testing](https://docs.aws.amazon.com/aws-backup/latest/devguide/restore-testing-validation.html)
can orchestrate resource restoration and validation, but its job success is
not proof that Vak's records, keys and erasures are correct.

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
8. Race two signups against one remaining warm VM, exhaust a size pool, and
   fail a claim halfway through. Exactly one customer binds the VM, misses
   take the cold path, and no previously bound VM returns to inventory.
   Verify the four readiness timestamps and that pool instances have no
   customer secrets or shared TLS key.
9. Compromise a portal session or its operator role and attempt VM login,
   first-owner substitution, passkey enrollment and key release. None succeeds
   without the customer-held credential and approved VM measurement. Repeat
   after owner recovery and after changing the public hostname; no portal
   cookie, email reset or stale claim becomes VM authority.
10. In strong durability mode, acknowledge writes across each durable class,
    then destroy the VM before the next scheduled backup. Recover every
    acknowledged byte, ref and receipt from the complete remote marker.
    Interrupt before that marker and prove no partial state is advertised as
    saved. Rotate keys, retain older versions, erase one scope and repeat;
    neither rotation nor backup restoration loses live data or revives erased
    content.

The current headless install, doc 73 milestones and this fleet design must
each retain their own status. Passing a unit test or encrypting an EBS volume
does not satisfy the customer-release scenarios above.

## 10. Fleet operations system

This is a separate service from Vak's existing per-VM Operations Center
(doc 28). Doc 28 remains the customer's detailed, authenticated view of its
own Agents, runs, channels and service manager. The fleet system manages
customer accounts, deployment inventory and safe infrastructure operations
without importing those detailed records. A central login does not become a
route into `/admin` (§3.2).

### 10.1 Inventory and ownership

The fleet database is the authoritative map from customer to deployment and
from deployment to infrastructure. Keep stable UUIDv7 ids; an EC2 instance id,
IP, hostname or AWS tag is an observed resource attribute, never the primary
key. The minimal records are:

| Record | Purpose | Allowed central fields |
|---|---|---|
| Customer | commercial and contact relationship | id, billing reference, selected region/residency, notification preferences, status |
| Deployment | one dedicated VM entitlement | id, customer id, size, stable origin, desired release, desired lifecycle state, generation, key-policy reference |
| Resource binding | accountable AWS inventory | instance/volume/security-group references, zone, image digest, network and backup policy ids, observed state |
| Operation | every requested change | id, type, actor, reason, target, idempotency key, expected generation, state, timestamps, receipt |
| Health sample | bounded fleet visibility | deployment id, probe result, service state, resource buckets, backup age, release digest, collector time |
| Incident | ownership of a service problem | id, fingerprint, severity, affected deployments, first/last seen, acknowledgement, resolution evidence |

Never store owner credentials, provider keys, content-derived Agent names,
filenames, free-form error strings or raw VM logs in these records. AWS tags
carry opaque fleet ids and approved operational labels only. Billing and
portal contact data have their own retention/access policy; a terminated VM
does not automatically erase legally required commercial records. Reconcile
the database against AWS resource inventory and management audit, flagging
orphaned volumes, instances, security groups, backups and DNS records. A
resource with an unknown owner is quarantined for review, never silently
assigned to a customer.

### 10.2 Reconciler and safe operations

The panel writes desired state and an operation record; a managed workflow
applies it. The UI never calls EC2 directly. Prefer [Step Functions Standard
Workflows](https://docs.aws.amazon.com/step-functions/latest/dg/choosing-workflow-type.html)
for long-running provision/replace/retire actions rather than building a
queue and workflow engine. A per-deployment conditional state update and
generation check serialize conflicting operations. Each effectful step is
idempotent and records the AWS request id, before/after state and a
verified/pending/failed receipt. Workflow history is useful for debugging
but has finite retention and is not the durable fleet ledger (§13); inputs
and outputs contain opaque ids, never customer content or secrets. On retry,
the workflow compares observed resources before another action. Standard
workflow execution semantics do not make an external EC2 or payment side
effect magically exactly-once. A failed API call never implies that no
resource was created. Cancellation stops future steps and reports what
already happened; it does not erase receipts.

The permitted actions are provision, pause new run admission, restart Vak,
replace or resize VM, apply an approved release, rotate infrastructure
identity, run a backup/restore drill, isolate network and retire. Each has a
precondition, owner, timeout, rollback or repair-forward path, and postcheck.
Replacing a VM invokes the fenced DR protocol (§8). Resizing requires a
capacity check and downtime notice. Retire requires the customer-authorized
lifecycle policy and retention/hold check; an operator's EC2 terminate click
cannot skip it. No generic SSH, SSM Run Command, arbitrary shell, raw console
output, volume attach or snapshot download is exposed in the fleet role.

Separate roles for billing support, operations, release management, security
response and customer owner. Production changes use short-lived staff
identity, least privilege and audit; high-impact actions such as mass rollout,
network isolation and retirement require an independent approval. Approval
never grants data-plane access. Break-glass may stop service or isolate a VM,
but never reads plaintext or bypasses customer key authority. Infrastructure
API calls and staff access are correlated to the operation receipt; [AWS
CloudTrail management events](https://docs.aws.amazon.com/awscloudtrail/latest/userguide/logging-management-events-with-cloudtrail.html)
provide another audit source, with tag/parameter content kept non-sensitive.

At 10 customers, one regional reconciler and modest warm pool suffice. At
100 and 1,000+, partition work by deployment id and region, bound concurrent
EC2/DNS/certificate calls, maintain capacity forecasts and AWS quota headroom,
and keep managed workflow steps stateless and horizontally replaceable. The same state
machine and privacy boundary apply at every scale. The fleet control plane
must be recoverable from its own database and operation ledger; a portal
outage leaves existing customer VMs running and able to serve their owners.
Back up and drill restoration of the fleet database separately from customer
content. Reconciliation after its recovery must not create duplicate VMs or
mistake an active customer VM for an unused pool member.

### 10.3 Dashboards and measured service health

The operator landing page answers: how many customers and VMs exist by
region/size/state; how many are provisioning, degraded or recovering; pool
capacity and signup latency; backup lag and last drill; release adoption;
open incidents; cost/idle capacity; and quota headroom. Every count links to
the exact deployment or receipt that explains it. A customer detail page
shows commercial entitlement, VM lifecycle, version, public readiness,
allowlisted resource use, backup age, operations and incidents. It has no
transcript, Agent/file browser or link that impersonates the owner.

Use three independent health layers: AWS instance/EBS status, an external
HTTPS synthetic probe at the customer origin, and a content-free Vak
heartbeat/readiness report. EC2 status checks alone do not prove the app
works ([AWS status-check guide](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/monitoring-system-instance-status-check.html)).
The synthetic probe must not sign in as the customer or run a real prompt.
Customer-side measures may include bounded schedule lag, delivery backlog,
provider connectivity class and last successful operation, exported only
through §6's allowlist. A missing heartbeat is `unknown` after a short grace
window, never green; reconcile it with the independent probe before restart.
The service manager remains the owner of process restart (doc 28). Fleet
automation acts only on a diagnosed, sustained failure, with a cooldown and
one active remediation owner, so an EC2 alarm and fleet worker cannot race
to replace the same VM.

CloudWatch metrics/alarms can carry infrastructure signals and a small set
of allowlisted fleet metrics; its logs must not become a central raw VM log
sink. [Composite alarms](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/Create_Composite_Alarm.html)
can reduce duplicate pages, but the fleet incident record is the product
source of truth. A panel can be built over the fleet database first and
augmented with CloudWatch; adopting a tracing product must pass the same
content/label review before ingestion. Dashboard access is role-scoped,
time-bounded and audited.

### 10.4 Alerting, incidents and support

Alert on customer impact and recovery risk, not every noisy metric. A
candidate routing table, to tune from measured baselines before an SLA:

| Signal | First response | Escalation |
|---|---|---|
| public origin unavailable and Vak not ready | open per-deployment incident; test service-manager recovery | page on-call if sustained or several customers affected |
| VM/EBS impaired, writer epoch at risk | fence and follow §8 recovery runbook | page immediately if recovery fails or no safe replacement |
| backup age exceeds policy or restore drill fails | stop claiming recoverability; create risk incident | page before the RPO target is breached |
| key authority unavailable | pause affected reads/runs; notify customer | page for widespread outage; no operator decrypt override |
| pool depleted or signup p95 rising | replenish/cold-launch; show wait | capacity owner if sustained or paid signup fails |
| provider/channel authentication failure | notify customer with typed class | fleet page only if widespread platform cause |
| telemetry absent | mark unknown; cross-check external probe | page if blind to active customer impact |

Deduplicate by deployment plus causal fingerprint; a regional incident can
group many deployments without losing the per-customer timeline. Record
first detection, acknowledgement, owner, actions, customer notices and
verified resolution. Alert delivery is itself monitored through a separate
test path. The customer portal shows only that customer's incident, impact,
recovery point and maintenance window. Internal support sees typed failure
classes and action receipts; it asks the customer to export an explicit,
previewable diagnostic bundle if content is genuinely needed (§6). Support
never requests a passkey, provider key, recovery code or unrestricted VM
access. Scheduled maintenance and security advisories have a distinct notice
path from incident pages.

Release gates for this operations system include inventory reconciliation
after lost events, duplicate operation dispatch, a control-plane database
restore, compromised staff role, missing telemetry, failed alert delivery,
regional alarm storm, customer-visible status accuracy and evidence that no
central dashboard or log query can retrieve customer content.

## 11. Ingress, egress and traffic

Start with a small, inspectable AWS footprint: one VPC per region, dedicated
customer VMs with per-VM security groups, one public HTTPS origin per VM,
Route 53 records, Caddy on the VM, an internet gateway, encrypted backup
storage and the separate fleet control plane. No shared customer runtime,
shared application load balancer that terminates TLS, Kubernetes cluster or
NAT gateway is needed merely to reach the first customers. AWS's [internet
gateway](https://docs.aws.amazon.com/vpc/latest/userguide/VPC_Internet_Gateway.html)
supports direct public-instance ingress and egress; traffic and public IPv4
still have costs. Add a private-subnet/NAT or more elaborate edge only after
measured need and a renewed privacy/cost review.

### 11.1 Inbound path

`browser/channel → DNS → customer VM public address → security group → Caddy
TLS → loopback Vak`. Only the public HTTPS port is required for normal use;
HTTP may exist solely for controlled certificate issuance/redirect if that
method is chosen. Vak's port 8901, SSH, management protocols and databases
are never public. Caddy enforces request size, connection/idle timeouts and
coarse unauthenticated rate limits; Vak enforces identity, exact origin,
authorization, admission and per-tool policy. Channel webhook endpoints need
their own signature/identity checks; an address or AWS security group cannot
identify a customer or bot. Traffic spikes get per-VM limits so one
customer's overload does not exhaust another's VM.

DNS cutover is part of replacement and must preserve the hostname and
passkey origin. Route 53 reports when a change is `INSYNC`, but recursive
resolver caches still obey TTL; a seconds-level warm assignment may use a
preassigned, already-resolving origin, while DR measures the actual cutover
time. [AWS DNS guidance](https://docs.aws.amazon.com/Route53/latest/DeveloperGuide/best-practices-dns.html)
is the basis for the propagation check. A shared front door could provide
stronger volumetric protection later, but if it terminates TLS it can see
customer traffic and requires a changed privacy claim. TLS passthrough may
preserve the boundary if its operational and certificate behavior is proven.

### 11.2 Outbound path and management channel

The VM initiates an authenticated outbound management connection or polls
for signed, narrowly typed desired-state commands. The fleet does not need
an inbound management port. VM identity is unique and short-lived; commands
carry deployment id, generation, expiry, allowed action and nonce, and the
VM records a receipt. This channel cannot carry arbitrary shell, customer
credentials or data extraction. A compromised fleet controller must still
be unable to make an unapproved release decrypt customer data (§5).

Separate outbound traffic into: customer-authorized inference and channel
destinations, customer-authorized Agent tools, customer-controlled key
authority, encrypted backup/object transport, and vendor update/allowlisted
health transport. The broker governs tool requests and recipient-scoped
credentials; infrastructure identity is unavailable to tools. Network rules
deny private east-west traffic and metadata access from workers, with
host-level controls where a security group is too coarse. An agent that can
make arbitrary HTTP requests could still reach another customer's public
URL, so that URL must authenticate every request. For strict egress plans,
add an independently enforced proxy/firewall with explicit destination
policy; expose its restrictions and failures to the customer rather than
claiming that an EC2 security group enforces domain allowlists. [Security
group destinations](https://docs.aws.amazon.com/vpc/latest/userguide/working-with-security-group-rules.html)
are addresses, security groups and prefix lists, not arbitrary FQDN policy.
The general-purpose Agent may need wider internet access when the customer
authorizes it; that is a customer setting and a separately tested risk.

Traffic accounting measures bytes by deployment and broad class, connection
rate, public ingress errors, egress errors and spend; it never records URL,
query, prompt, body or packet capture in the fleet plane. IP/port flow
metadata can itself reveal relationships, so [VPC Flow Logs](https://docs.aws.amazon.com/vpc/latest/userguide/flow-log-records.html)
are disabled or narrowly scoped and retained only under an explicit security
purpose and access policy. Alert on unexpected egress volume or blocked
private destinations without feeding destinations to an AI model. Show the
customer bandwidth usage and limits before throttling; security isolation
may act immediately and produce a receipt. Track public IPv4, data transfer,
DNS, certificates, backup movement and any NAT/proxy as distinct cost lines.

Tests cover direct and private attempts from customer A to B, metadata and
link-local access from every worker type, forged Host/proxy headers, webhook
spoofing, TLS/certificate rotation, DNS cutover, traffic floods, provider
egress failure and loss of the management channel. Loss of the fleet channel
must leave the existing VM serving its owner safely; it must not silently
open a management port or switch to a less restrictive network policy.

## 12. AI-assisted fleet management

Use a fleet agent as an **analyst and runbook operator**, not as an AWS
administrator with a shell. Its input is the same allowlisted inventory,
health samples, incident records, approved runbooks and operation receipts
available to a human fleet operator. It receives no customer prompts,
files, raw logs, secrets or unrestricted CloudWatch query. Each tool is a
typed read or an existing §10.2 operation with the same policy, generation
check and receipt. An AWS skill used by a developer to explain an API does
not itself grant the production agent credentials or change this boundary.

The agent can summarize fleet posture, correlate an EC2 impairment with a
failed public probe, identify pool depletion, estimate affected customers,
recommend a runbook and draft a customer-safe incident notice. It may run
reversible, preapproved low-risk actions such as retrying a failed synthetic
probe or requesting an extra unused pool VM within a budget/capacity cap.
Release rollout, replacement, resize, traffic isolation, backup restore,
retirement, key-policy change and customer notices need the action's normal
human/customer authority. A model suggestion cannot widen its own role or
turn a failed operation into a more permissive one. The deterministic
reconciler remains the only component that changes AWS resources.

Every AI conclusion cites the exact metric, resource state or receipt and
its observation time; missing telemetry is stated as unknown. Model-visible
fleet input and tool results are recorded in a fleet audit ledger with a
retention policy, while §6's content-free rule applies before anything
reaches the model. Fleet AI requests themselves use an approved provider and
regional/data-processing policy. Evaluate against injected incident text,
false-green telemetry, duplicate alarms, a compromised portal account and
a request to fetch raw customer logs. The expected result is a bounded
diagnosis or refusal, not an improvised AWS command.

## 13. Systems of record and bookkeeping

Keep one authority for each fact; dashboards and AI summaries are projections.
Do not build a second customer data catalog, payment processor, workflow
engine or monitoring backend inside the fleet service.

| Fact | Authoritative record | Fleet projection or evidence |
|---|---|---|
| customer content, Agent/Run history, secrets, owner identity | customer's protected Vak data home and doc 73 record/object/ref model | no central copy; only approved health/usage classes |
| customer entitlement, region, size, lifecycle and assigned VM | fleet inventory database | EC2/Route 53 observed resources checked against it |
| requested infrastructure action and outcome | append-only fleet operation events and signed/attributed receipts | Step Functions execution and CloudTrail as independent evidence |
| AWS resource state | EC2, EBS, security group, DNS and backup service APIs | last-observed inventory with observation time and drift flag |
| customer charge, invoice, refund and tax | chosen payment/billing provider | opaque invoice/payment references and entitlement changes |
| billable VM time and optional managed-token usage | fleet usage ledger from verified lifecycle/usage events | invoice line items reconciled to usage; no model content |
| AWS expense | AWS billing export | cost and margin view reconciled by opaque resource/deployment id |
| infrastructure signals | CloudWatch/EC2 health | fleet health and incidents; no raw VM content |

For the initial control plane, a managed [DynamoDB transaction](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/transaction-apis.html)
can atomically claim an unused VM, advance a deployment generation, insert an
operation event and update its current-state projection. Conditional writes
prevent double assignment. Use one small, explicitly indexed inventory and
ledger schema with point-in-time recovery, access controls and a restore
drill; a second SQL/event store is not needed until real query requirements
justify it. [DynamoDB point-in-time restore](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/pointintimerecovery_restores.html)
creates a new table, so the recovery runbook must reconcile it with live AWS
resources and with any events written after the selected restore time.
App-level event immutability is enforced by write permissions and conditional
inserts; AWS account administrators remain a separate audit threat, so
CloudTrail and controlled exports provide independent evidence.

An operation event contains full UUIDv7 id, deployment id, generation,
actor/authority, action, decision, idempotency key, timestamps, referenced
AWS request/execution ids and typed outcome. Corrections append compensating
events; no event is edited to make a failed operation look successful.
Current state is rebuilt from events plus verified AWS observations. Every
dashboard number can name its source and as-of time. A missing or conflicting
record blocks destructive action and opens an incident. Define retention for
operational, security and billing classes separately, with export and
deletion controls consistent with §9.

The usage ledger has a start/stop interval for each billable deployment
generation, size and region, plus explicit credits/adjustments. It never
infers customer charges from an EC2 running flag alone: failed provisioning,
maintenance, suspension, replacement overlap and complimentary pool VMs
need defined billing rules. Each invoice line refers to immutable usage
event ids and a pricing version; a late correction becomes a credit or
adjustment, not a silent rewrite. The payment provider owns settlement and
refund truth. Reconcile paid entitlement ↔ active deployment daily and
customer invoice ↔ usage events before collection. Match AWS expenses to
resource bindings using [AWS Data Exports/CUR resource IDs](https://docs.aws.amazon.com/cur/latest/userguide/Lineitem-columns.html)
where available; costs may arrive late and are for margin analysis, not the
sole basis of a real-time customer bill. Never place names, email addresses
or secrets in cost-allocation tags.

The smallest AWS-managed slice to implement this proposal is: EC2/EBS,
security groups and Route 53 for each VM; DynamoDB for fleet inventory,
operation and usage events; Step Functions Standard for long operations;
CloudWatch alarms/metrics and CloudTrail for infrastructure evidence; and
encrypted backup storage. Use the chosen payment provider for money and
Vak's per-VM Operations Center for customer detail. Add EventBridge
scheduling, cost export, a specialized firewall, or a separate analytics
store only when a measured requirement needs them. Managed service use
removes routine infrastructure work, but its inputs, IAM roles, telemetry
and recovery still have to satisfy the operator-blind boundary.
