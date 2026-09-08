# Architecture decision matrix — AWS hosting of vak as a SaaS

Status: research observation, 2026-09-08. This matrix weighs the hosting
options against what vak's design actually *permits* (not what a SaaS
product would ideally look like). Sources: `docs/design/48-web-client.md`
(Phase E, multi‑user/non‑goals), `docs/design/34-channel-onboarding.md`
(Phase 2 "What stays single-process"), `docs/design/24-agent-security.md`,
`docs/design/25-docker-sandbox.md`, `docs/design/42-managed-work-contracts.md`,
and AGENTS.md invariants 10/14/16/25/35. See `cost-model.md` for prices.

## Options

- **A. Per‑tenant VM.** One `t4g.micro` (or larger) per customer. Each runs
  its own `vak serve --trust <workspace>` with a dedicated data home on a
  dedicated volume. This is literally vak docs: *"A workspace that genuinely
  needs process‑level isolation … is out of scope here — that's still 'run
  another gateway process.'"* (doc 34 Phase 2).
- **B. Per‑tenant container on a shared ECS/Fargate fleet**, tenant‑id
  namespaces + per‑tenant volumes. Shared kernel, denser packing.
- **C. Shared multi‑tenant Core.** Tenants coexist in one `vak serve`
  process with tenant routing in the broker.
- **D. Serverless per request.** Lambda + API Gateway for the agent turns.
- **E. Static frontend only (Always‑Free).** S3 + CloudFront + Cognito for
  the public site / login / control plane; no engine hosting.

## Evaluation

Legend: ✅ yes / ✅\* conditional / ⚠ partial / ❌ no / 🔥 breaks an invariant.

| Criterion | A. Per‑tenant VM | B. Shared containers | C. Shared Core | D. Serverless | E. Static only |
|---|---|---|---|---|---|
| Matches vak's single‑tenant model | ✅ | ✅\* | ❌ | ❌ | N/A |
| No change to vak source/build | ✅ | ⚠\*\* | 🔥 | 🔥 | ✅ |
| Process / filesystem isolation between customers | ✅ | ⚠\*\*\* | ❌❌ | ❌ | N/A |
| Works with vak's warm, weeks‑long sessions | ✅ | ✅ | ✅ | ❌ | N/A |
| Supports turns > 15 min (Lambda limit) | ✅ | ✅ | ✅ | ❌❌ | N/A |
| Broker spawns bash/tool subprocesses (vak‑tools worker) | ✅ | ⚠\*\*\*\* | ⚠\*\*\*\* | ❌❌ | N/A |
| Append‑only SQLite ledger (vak‑store, embedded) | ✅ | ⚠\*\*\*\* | ⚠\*\*\*\* | ❌❌ | N/A |
| SSE + WebSocket PTY stay alive | ✅ | ⚠ | ⚠ | ❌ | N/A |
| Free Tier viability (6‑month $200 pot) | ✅ for 1 tenant | ✅ for 1 tenant | ✅ for 0 (control plane only) | ✅ for control plane | ✅ always |
| Ongoing (>6 mo) infra cost/tenant | ~$14/mo | ~$14/mo\* | $0 engine (but rewrite cost) | $0 for bursts | $0 |
| Multi‑user *within* a tenant (teams/roles) | ❌ | ❌ | ❌ | ❌ | ❌ |
| Enterprise private subnet / VPC endpoints / no public IP | ✅ (add NAT/PVI) | ✅ | ✅ | ✅ | N/A |
| Single-tenant Free Plan account doesn't cover | ❌ | ❌ | ❌ | ❌ | ✅ |

Notes:
- **A. ✅\***: matches vak exactly; the only option that requires *zero*
  changes to vak, vak‑core, vak‑server, or the Dockerfile. Isolation is real
  (separate OS process, separate filesystem, separate data home). The docs
  explicitly point here ("run another gateway process"). Cost is the only
  downside.
- **B. ⚠\*\***: Fargate/ECS can run the same Docker image per tenant task on
  a shared cluster, which is denser and lets you patch/scale the cluster
  once. But you must supply per‑tenant volume mounts (EBS vs EFS), per‑tenant
  env injection of `VAK_GATEWAY_TOKEN`, and a scheduler that maps
  authenticated tenant → task. vak's worker‑subprocess model still works
  inside the task's cgroup, but the *shared kernel* means a sandbox escape in
  the bash tool (`docs/design/25-docker-sandbox.md` Seatbelt/Landlock) could
  in principle cross tenants unless you add another isolation ring. This is
  the usual "containers aren't security boundaries" caveat.
- **B. ⚠\*\*\*\* / C. ⚠\*\*\*\***: the broker's bash worker and the SQLite
  store assume a *private* filesystem per Core. In a shared Core (C) or shared
  kernel (B) you must guarantee tenant‑rooted, symlink/escape‑failing‑closed
  path resolution (AGENTS.md invariant 10) per tenant yourself. vak does not
  multi‑tenant this today and punts on it by design.
- **C. ❌❌**: directly contradicts `docs/design/34` ("No cross‑machine /
  cross‑account process isolation") and AGENTS.md invariant 14 (built‑in tools
  execute through the versioned `__tool_worker` protocol in a disposable
  process group) + invariant 25 (a missing/unavailable restricted sandbox
  fails closed). Sharing one Core across tenants means sharing one sandbox
  decision surface — a violation of the permission contract's trust model.
- **D. ❌❌**: Lambda's 15‑minute timeout, cold starts, and frozen‑between‑
  invokes container model break vak's turn model, SSE/PTY channels, and the
  persistent worker subprocesses. Only the *control plane* (registration,
  plan/billing metadata, key rotation) and the *static frontend* are
  Lambda‑shaped. Building the agent engine on Lambda == writing a different
  product.
- **E. ✅**: S3 (5 GB) + CloudFront (15 GB / 2M req) + ACM cert + Cognito
  (10k MAU) is the one combination that is **genuinely Always‑Free** and
  survives past the 6‑month Free Plan. But it hosts *only* static assets and
  login — the engine must live somewhere that isn't free.

## Which is "right" for vak?

**vak's design only has one honest answer: A (per‑tenant VM), or B as a
denser packaging of the same.** This is not a coincidence — `docs/design/34`
and `docs/design/48` exist precisely to say "we deliberately did not build
in‑process multi‑tenant isolation, because process/account isolation is the
trust boundary." Choosing A keeps every vak invariant intact (invariants
10, 14, 25, 35 — workspace‑rooted FS access, brokered tool workers in a
disposable process group, fail‑closed sandboxing, scrubbed/quarantined
execution under `.vak/scratch/`). Choosing C or D would require rewriting
those invariants out of vak.

The trade‑off is therefore **purely economic and operational**, not
architectural: A is correct for vak and costly per tenant; C/D would be
cheaper to run but would stop being vak.
