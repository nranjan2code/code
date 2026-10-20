# Data integrity and hostile-prompt security review — 2026-10-20

Status: historical. Source review and synthetic adversarial probes against commit `0c3c4703afa538f76269b897fedfd57c6ad4f133`, workspace version 8.16.0. Confirmed weaknesses remain open in [the tracker](../plans/open-items.md). No production behavior, installed service, credentials or real user data was changed.

Follow-through: [security hardening and external links](security-hardening-2026-10-20.md).
This baseline is preserved; its vulnerable assertions are evidence, not the
current acceptance tests.


## Assessment

Vakyartha has substantial technical foundations for confidentiality and durable records. It does **not** yet justify a claim that a rogue prompt cannot obtain unauthorized disclosure or corrupt data. A successful injection often needs no sandbox escape: it can misuse an allowed tool or place a network request in the answer the UI draws.

The strongest protections are scoped authenticated encryption, separate worker processes, restricted-mode OS sandboxes, scrubbed subprocess environments, strict grant reads and transactional writer fencing. The weakest link in this review was composing these protections across the model, worker protocol and UI. Existing passing unit tests did not establish that the composed boundary held.

| Threat | Assessment from this review |
|---|---|
| Random byte damage in records | Good detection primitives, but the administrative verifier has a confirmed blind spot |
| Concurrent/deposed writers | Atomic ref fencing exists; broader process interleavings remain unproven |
| Untrusted code reaching arbitrary host files in restricted mode | Meaningful OS containment; not a hostile-concurrency proof |
| Prompt-driven disclosure | Incomplete: answer images and missing provenance labels bypass intended gates |
| Prompt-driven workspace corruption | Workspace Write permits it; its boundary is the folder, not the meaning of the user's request |
| Full Access against a hostile model/prompt | No containment claim is warranted; this deliberately disables sandboxing and the provenance gate |
| Remotely exposed server | Host/origin/auth/audience defenses exist; this was not an exhaustive route penetration test |

Severity below is remediation priority. P1 means an important protection can be bypassed under stated conditions. P2 means a diagnostic or assurance failure without a demonstrated authorization bypass.

## SI-01 — P1: answer rendering is an ungoverned outbound channel

**Confirmed in a real browser with synthetic data.** [The live Markdown renderer](../../crates/vak-client-ui/src/md.ts) turns any HTTP(S) Markdown image into an active `<img src=…>` (lines 69–75). [MarkdownView](../../crates/vak-client-ui/src/components/MarkdownView.tsx) puts this markup into the DOM (line 64). Neither receives the turn's provenance or asks an outbound approval. The browser performs the request; the tool broker never sees it.

A hostile model that knows a private value can write an image whose query string contains that value. A fresh Chrome profile, displaying the output of the real `renderMarkdown`, requested `/collect?marker=SYNTHETIC_PRIVATE_123` from a loopback-only listener without a click. Only the synthetic marker was used; no external collector was contacted.

The Markdown path also ignores the `externalMedia` preference. The settled semantic renderer does check that preference, but [its image branch](../../crates/vak-client-ui/src/components/PresentationRenderer.tsx) still has no per-turn provenance/approval check, and [the preference defaults to true](../../crates/vak-client-ui/src/store.ts). Streaming can disclose before the settled renderer replaces it. The desktop CSP explicitly permits HTTP(S) image sources in [tauri.conf.json](../../crates/vak-desktop/tauri.conf.json).

**Impact:** potential disclosure through a model answer even when restricted Bash has no network and the tool provenance gate would refuse a fetch. The proof establishes actual browser egress, not that a particular live model will choose the attack.

**Required correction:** treat all model-controlled active resources as effects: Markdown/semantic images, audio/video metadata, thumbnails and other renderers. Default to inert placeholders until their exact outbound request has authorization; a general images preference is not permission to include private content in a URL. Apply the same policy during streaming, settled rendering, fallbacks, saved versions and shared views. A proxy alone does not solve disclosure unless it also governs the destination and data sent. Regression acceptance must observe network requests, including with external media off.

## SI-02 — P1: real workers lose the control-file protection context

**Confirmed through the real broker and macOS Seatbelt.** [Broker dispatch](../../crates/vak-tools/src/broker.rs) serializes execution id, Agent, trace and execution root, but not `owner_session_id`. The child constructs a fresh `SandboxEventSink::new_with_id` and optionally attaches the trace (lines 1058–1066); it never attaches an owner session. [is_runtime_call](../../crates/vak-tools/src/sandbox_events.rs) returns true only when that owner is present (lines 149–152).

[Read](../../crates/vak-tools/src/read.rs), [write/edit protection](../../crates/vak-tools/src/write.rs) and [Bash's control-file guard](../../crates/vak-tools/src/bash.rs) depend on this predicate. As a result, protections tested by direct tool fixtures are inactive in the real child. The Seatbelt profile allows the workspace subtree and has no separate control-file deny rules.

The probe supplied an owner session to the parent, enabled `Seatbelt::new(WorkspaceWrite, …)`, and demonstrated:

- direct `ReadTool` refused the synthetic `.env`;
- the brokered read returned its synthetic marker;
- brokered `WriteTool` wrote `.vak/permissions.local.toml`.

**Impact:** a model can modify the very files intended to control its authority. Trusted workspace openings load local allow rules in [Core construction](../../crates/vak-core/src/lib.rs), lines 1449–1453. This demonstrates writable persistent policy, not immediate escalation of the current in-memory engine. No real Vakyartha credential store was read; a workspace `.env` may belong to another application.

**Required correction:** make control-state protection intrinsic to production worker execution, not optional telemetry ownership. Carry and validate required admission context in the versioned protocol. Protect control paths at the OS/filesystem boundary across all tools, interpreters, aliases, rename operations and directory replacement. Merely restoring the flag is insufficient: Bash currently checks literal substrings, so computed paths evade it; direct grep of a control file has no equivalent guard.

## SI-03 — P1: provenance misses hostile repository and skill text

**Confirmed with the real agent loop and a scripted provider.** [provenance_of_reach](../../crates/vak-tools/src/lib.rs), lines 113–122, labels every `ReadPath`/`ReadStore` result private only. A checked-out README, downloaded document, dependency output or plugin skill is not necessarily owner-authored. [SkillTool](../../crates/vak-core/src/skills.rs) declares `ReadStore` and does not override that label for third-party instructions.

The probe made the provider call a file reader for repository instructions, a file reader for private data, then a network tool. The network tool already had an explicit allow rule. The call ran with zero approval requests, because no source set `untrusted`. Normal network mode defaults would still ask; this is a bypass of the additional provenance protection, not of a deny rule or OS network restriction.

**Required correction:** derive trust from the source and recorded lineage, not filesystem location or tool reach alone. Unknown/third-party workspace content and executable instructions must be conservatively untrusted. Preserve those labels through skills, learned state, recall, delegation, compaction and later turns. Test the actual source adapters as well as the gate's truth table.

## SI-04 — P1: confidential owner input does not set the private label

**Confirmed with the real agent loop and a scripted provider.** A synthetic confidential marker in the user's message followed by two external tool results produced zero outbound provenance approvals when the network tool was preallowed. The first external result supplies `untrusted`; the user text never supplies `private`, so the second exit is not gated.

The gate accumulates inherited and tool-result labels; the probe shows that the prompt itself is absent from the protection. This also matters when a user pastes private material instead of asking the model to read a file. Attachments and other model-visible input paths require a separate coverage audit; this review does not claim each was exercised.

**Required correction:** conservatively label owner input as private at admission and carry it through the ledger/context/turn graph. Distinguish the owner's authority to request work from whether the text contains private information. Reproduce owner-text + web/email instructions + preallowed exit, including across turns and workers.

## SI-05 — P2: integrity verification can report an unreadable chain as intact

**Confirmed using real record and segment APIs.** The probe wrote two records, sealed their first segment, flipped a byte in `seals.log`, and compared readers. `SegmentSet::read(1, None)` correctly rejected the corrupted manifest; [vak_session::verify::verify](../../crates/vak-session/src/verify.rs) reported two frames and no broken segment.

The verifier checks frame hashes from `prev_head`, but does not compare each sealed segment's count/head with its own seal entry. For segment 1, `prev_head(1)` returns genesis without reading the manifest. The administrative [data_integrity](../../crates/vak-core/src/integrity.rs) consumes this checker. Its directory walk also flattens traversal errors, skips the object store, and `Integrity::sound()` considers only broken segments and invalid receipt signatures. It is not a full inventory/authentication check of every retained object, key and projection.

`prev_head` also uses `seals()`, which may trim a torn manifest tail; the purported read-only verifier should use strict non-repairing APIs. That mutation path was identified by source review, not separately exercised here.

**Required correction:** verify the manifest strictly, bind every retained sealed segment to its manifest count/head, surface enumeration failures/incomplete coverage, and explicitly report which object/key classes were verified. Do not silently repair while observing. Add independent-reader comparisons and interrupted/multi-process cases. No forged ciphertext or successful grant bypass was demonstrated by this probe.

## Further material limits

**Full Access is explicit unrestricted trust.** [provenance_gate](../../crates/vak-agent/src/lib.rs), lines 9729–9736, deliberately bypasses provenance checks in Full Access. Its regression test confirms this behavior. A malicious instruction can use allowed reads/writes/network with the process user's authority. Encryption at rest does not protect data once an authorized runtime has decrypted it. This is an accepted product policy, not a newly discovered implementation bug; changing it requires an explicit product decision.

**Workspace Write does not authorize by intent.** A scripted provider read private and external content, then used the real `WriteTool` to replace a synthetic `accounts.csv` while the user request only asked for a review. There was no approval. The provenance gate governs exits, not local mutations, and the permission mode allows workspace writes. For sensitive original files, a review-and-promotion boundary or read-only admission is needed; asking the model to be careful is not that boundary.

**Inference is an outbound trust decision.** Content in model requests reaches the selected provider. Blocking tool network does not block that channel. This review did not inspect provider retention or an installed route and does not claim a model request is an unauthorized transmission merely because it uses a configured provider.

**SSRF defenses remain partial.** [webfetch](../../crates/vak-tools/src/webfetch.rs) screens DNS before the HTTP client resolves again (lines 124–127). [webbrowse](../../crates/vak-tools/src/webbrowse.rs) screens only initial navigation; browser redirects/subresources are unscreened and the browser has no Seatbelt/Landlock wrapper. A public page may cause requests toward internal services; complete response extraction depends on redirects, origin/CORS and the target. This was verified in code, not with a live rebinding/internal-service exploit. Use pinned resolution and an enforcing network boundary on every connection before treating it as comprehensive SSRF protection.

**Filesystem races and test assurance remain open.** Descriptor-relative no-follow operations are still absent from relevant path checks. Sandbox containment reduces the scope of a race, but canonicalize-then-open is not a race-proof guarantee. Existing blueprint F3 already records insufficient independent chain models and multi-process interleavings. This review does not establish power-loss durability on all platforms or complete route/connector correctness.

## What passed and how to reproduce

All tests used disposable workspaces and isolated test data homes. There were no live model calls, real-secret reads or external uploads. The scripted provider deliberately chooses the unsafe actions so the test measures runtime enforcement independently of model obedience or refusal.

- `cargo test -p vak-storage -p vak-permission -p vak-control --quiet`: 128 tests passed (17 + 9 + 40 + 58 + 4).
- `cargo build -p vak-server --bin vak-tool-worker --quiet`: passed.
- Boundary probes: two passed, demonstrating the control-file failure and repository label gap.
- Integrity probe: one passed, demonstrating the verifier blind spot.
- Agent probes: 14 passed, comprising 11 existing gate/account-recording tests plus three adversarial cases. The existing gate works for already correctly labeled private + untrusted content, rejects unanswered approvals, and carries labels across turns/workers.
- Browser proof: real Markdown rendering generated a local `/collect?marker=SYNTHETIC_PRIVATE_123` request without interaction.

The source fixtures are preserved under [security-integrity-2026-10-20](security-integrity-2026-10-20/). They intentionally assert the vulnerable behavior; they are audit evidence, not passing security acceptance tests. To rerun, temporarily copy `boundary_probe.rs` to `crates/vak-tools/tests/audit_boundary_probe.rs`, `integrity_probe.rs` to `crates/vak-session/tests/audit_integrity_probe.rs`, and `provenance_probe.rs` to `crates/vak-agent/tests/audit_provenance_probe.rs`. Run the three corresponding `cargo test -p <crate> --test <name> -- --nocapture` commands after building the worker. Remove only those temporary files afterwards.

For the browser proof from the repository root:

```sh
mkdir -p /tmp/vak-security-audit-2026-10-20
crates/vak-client-ui/node_modules/.bin/esbuild docs/audits/security-integrity-2026-10-20/render_probe.ts --bundle --platform=node --format=cjs --outfile=/tmp/vak-security-audit-2026-10-20/render_probe.cjs
node /tmp/vak-security-audit-2026-10-20/render_probe.cjs > /tmp/vak-security-audit-2026-10-20/rendered.html
python3 docs/audits/security-integrity-2026-10-20/browser_probe.py
```

The browser fixture uses Chrome on macOS with a fresh temporary profile and a loopback-only synthetic collector. It is a renderer/network proof, not a full installed-app penetration test.

## Recommended order

1. Close answer-rendering egress and worker control-file access, with actual browser and broker regression tests.
2. Complete provenance input/source coverage and make unknown source trust conservative; test every outbound adapter.
3. Make integrity observation strict and complete about its coverage; expand independent-reader and multi-process tests.
4. Close rendered-browser/DNS network gaps. Separately decide how much hostile-prompt protection the product promises in Full Access and for writes to original files.

Prompt wording can reduce attack success but cannot replace these runtime boundaries. Until these failures are closed, the honest claim is meaningful containment in restricted modes with identified holes, not reliable prevention of rogue-prompt objectives.
