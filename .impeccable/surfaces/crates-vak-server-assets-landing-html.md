---
version: 1
slug: "crates-vak-server-assets-landing-html"
primary_target: "crates/vak-server/assets/landing.html"
related_targets: []
---

Scope: the public front door served at `/` by vak-server (`crates/vak-server/assets/landing.html`), a single self-contained file with no bundle and no build step.

Visitor mode: Persuade. The visitor decides whether this runtime is trustworthy and either opens a surface on this machine or installs it.

Audience: a solo, security-conscious technical user evaluating an agent runtime — often already sitting at the machine serving the page, sometimes reaching a headless box for the first time.

Job: understand the mechanism (ledger, gate, receipt, reading) well enough to believe the auditability claim, then open `/app`, open `/admin`, or copy the install command.

Proof and content: product truth only. There are no testimonials, customers, benchmarks or pricing, and none may be invented. The one live fact the page may show is `/version` — `/` is auth-exempt, so bind address, permission mode and workspace names must never appear here.

Constraints:
- Inline CSS and JS, no external requests beyond `/version` and `/app/vak-icon.png`; the front door must render before, and independently of, any bundle.
- No-JS renders the finished page: script only opts elements into an entrance.
- Marketing type ramp (60px display down to 11.5px micro) rather than the 13px app chrome; tokens, weights and the one-accent rule are unchanged.

Direction: The Instrument Panel (surface concept seed bdd3f44f, dealt lead, locked with no steer). The hero is the product: a session console that plays itself and stops on a real approval gate.

Memorable moment: the visitor answers the gate. Approve and the command runs and a receipt lands; Deny and the run stops, the edit already made is kept, and the denial is recorded. Both outcomes are the argument.

Unresolved: no external documentation host exists yet, so the footer links only to routes this server actually serves.
