# Your control page and Doctor routing repair — 2026-09-27

The `/security` page now explains six everyday choices: access, review,
information, chat connections, interruption and evidence, and enforcement.
Three generated Songbird illustrations retain the public site's white/charcoal,
indigo and saffron styling. Original assets and prompts are in
`docs/brand/library/public-site-scenes/security/`.

## Evidence for the copy

- Permission defaults, explicit rules and path checks: `crates/vak-permission/src/engine.rs`.
- Worker containment and scoped environments: `crates/vak-tools/src/broker.rs`.
- OS secret service and encrypted fallback: `crates/vak-config/src/credentials.rs`.
- Permission revocation: `apply_permission_mode` in `crates/vak-server/src/lib.rs`.
- Unattended approval and identities: `crates/vak-server/src/gateway.rs`.
- Spending admission: `crates/vak-core/src/finops.rs`.
- Threat model: `docs/design/24-agent-security.md` (read status and shipped implementation).
- Agent scope and storage: `docs/design/64-agent-owned-platform.md` and current AGENTS.md.

Full access, connected-service data flow, irreversible completed effects,
estimated spending and Trash versus erasure remain visible. The review buttons
are explicitly an illustration and never operate on actual files.

## Route regression

The previous website `/doctor` registration collided with the runtime's
existing `GET /doctor`. The public page is now `/meet-doctor`, including all
website navigation. Only the standalone Vercel export redirects the old public
URL. The embedded server keeps `/doctor` authenticated and JSON-producing.

`public_doctor_page_preserves_authenticated_doctor_api` constructs the full
secured router, requests both public-page slash forms, checks anonymous API
rejection and checks authenticated JSON. This catches the composition failure
that isolated site-route tests missed. The original full test run reproduced
34 overlapping-route failures; the new focused regression passes.

## Browser review

Reviewed at 1440 × 900 and 390 × 844 in light and dark themes. Screenshots here
cover the hero, review interaction, information illustration and footer. Mobile
has no horizontal overflow and all three images load. Both example decisions
and keyboard operation work; native details reveal the memory/Trash explanation;
chapter and return links work. Static validation checks all twelve pages,
local assets and fragments, and all 45 GitHub links for a safe new tab.
The shared footer includes the owner's supplied LinkedIn URL in a new tab.

## Publication

Published to https://vakyartha.com with deployment
`dpl_AodPTskqV3ATGPaeWumRkVseA287`. Browser verification confirms the new
security page, Doctor footer target, LinkedIn new-tab target and the public
`/doctor` redirect to `/meet-doctor`. The embedded API is not redirected.

## Final repository checks

Passed `cargo fmt --all --check`, workspace Clippy with warnings denied,
`cargo test --workspace` (including the new route regression), version consistency,
document-path validation, site manifest verification, JavaScript syntax and diff
whitespace checks. These checks ran against the current shared working tree.
