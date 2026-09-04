---
version: 1
slug: "crates-vak-server-site-src-pages-index-html"
primary_target: "crates/vak-server/site/src/pages/index.html"
related_targets: ["crates/vak-server/site/src/styles.css","crates/vak-server/site/src/layout.html"]
---

Scope: the public website vak-server serves at `/`, `/surfaces`, `/security` and `/install` — source in `crates/vak-server/site/src`, built by `site/build.py` into the committed, embedded `site/dist`.

Visitor mode: Persuade. The visitor decides whether this runtime is trustworthy and either opens a surface on this machine or installs it.

Audience: a solo, security-conscious technical user evaluating an agent runtime — often already sitting at the machine serving the page, sometimes reaching a headless box for the first time.

Job: understand the mechanism (ledger, gate, receipt, reading) well enough to believe the auditability claim, then open `/app`, open `/admin`, or copy the install command. The sub-pages exist for the reader who wants to check the claim rather than accept it.

Proof and content: product truth only. There are no testimonials, customers, benchmarks or pricing, and none may be invented. The one live fact a page may show is `/version`; a test enforces that no page fetches anything else, because every route here answers an unauthenticated stranger.

Constraints:
- CSS and script inline per page; the only sub-resource is the shared, deferred, content-hashed motion.dev build. The front door must render before, and independently of, anything else being up.
- No-JS renders the finished page; no-Motion renders it with CSS transitions.
- Marketing type ramp (60px display down to 11.5px micro) rather than the 13px app chrome; tokens, weights and the one-accent rule unchanged.
- The gate simulator on `/security` must keep reproducing `crates/vak-permission/src/engine.rs`. If that precedence changes, the page is lying until someone changes it too.

Direction: The Instrument Panel (surface concept seed bdd3f44f, dealt lead, locked with no steer). The hero is the product: a session console that plays itself and stops on a real approval gate.

Memorable moments: answering the gate on the home page (Approve runs the command and lands a receipt; Deny stops the run, keeps the edit already made, and records the denial), and running the real precedence yourself on `/security`.

Unresolved: no external documentation host exists yet, so the footer links only to routes this server actually serves.
