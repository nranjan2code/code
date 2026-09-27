# Provider, model and BYOK experience audit

Status: **implemented and checked in the web UI, 2026-09-27.** Scope is the
shared everyday client and provider/model controls in the admin portal. The
per-turn route planner, provider dispatch, retry ladder, secret backend and
server authorization are not changed.

## Findings and fixes

- Everyday setup began by asking a person to choose a service, then a model,
  before saying that an API key is billed by that service, that a chat plan
  may not cover API usage, where a key comes from, or where their messages go.
  The sheet said keys stayed "on this device" even in the web client, where
  the credential is saved on the server running Vakyartha. Replaced that with
  plain service/account language, the server storage location, billing and
  destination context, and a collapsed key-help explanation. A local model
  server no longer receives an unsupported "private and free" promise.
- The same sheet only accepted an agent config snapshot, then wrote the route
  to that agent. From Shared defaults it could therefore show the wrong route
  and write the wrong layer. It now reads the shared layer when opened there,
  captures the target agent and scope for the whole flow, and writes provider
  plus model atomically to the selected layer.
- Checking the model catalogue silently chose its first result in both the
  client and admin. Catalogue order is not a recommendation. The everyday
  flow now keeps the person's current model, selects automatically only when
  there is exactly one choice, and asks otherwise. Changing the service,
  opening the sheet or failing a save never changes the route. The admin keeps
  exact saved/custom IDs visible when they are absent from a fresh catalogue.
- A provider could be changed while its discovery was pending. A late result
  could then replace the new provider's model list. Settings and admin discard
  stale responses. Submitting is disabled during the request; changing
  service clears the old key draft and model choice.
- The admin Bedrock list displayed a listed model as selectable before its
  availability check refreshed, and unknown check results could leave a save
  path open. Only explicitly invokable models are enabled. A failed check
  leaves every Bedrock model unavailable until a successful check.
- Admin void-returning writes passed `handle(res)` into `void` from `.then`.
  HTTP errors rejected an unobserved inner promise while the caller refreshed
  and showed success. The API now awaits and propagates those errors, so model
  route and key writes surface failures honestly. This shared helper fixes the
  same defect on the other admin write endpoints using that pattern.
- Provider/model labels in everyday Settings now show the actual agent or
  Shared-layer choice. The quick model menu was removed: it read the global
  health model list while saving to the active agent, which could present a
  misleading choice. Change service/model opens the same guided flow from
  setup, Settings and the message menu. Key revocation names the shared scope
  and warns that other agents can lose access.
- Settings described old conversations as fixed to their starting model.
  The wording now says an already-running task keeps its current choice and
  a change applies from the next message. This matches the per-turn route
  contract; no route-planning or dispatch behavior was changed.

## Blast radius

The everyday client changes are in `crates/vak-client-ui/src/components/ConnectSheet.tsx`,
`Settings.tsx`, `Composer.tsx`, `SetupBanner.tsx`, `WorkspaceHeader.tsx`, and
`ChatPane.tsx`; `store.ts` captures the sheet scope, `api.ts` types discovery
and reads Shared defaults, `types.ts` carries non-secret credential-source
flags, `modelChoices.ts` preserves deliberate selections and fails closed for
unknown Bedrock status, and `styles.css` handles the compact sheet and key
help. `Composer` no longer supplies a one-off picker with mismatched scope.

Advanced controls stay in `crates/vak-admin-ui`: `App.tsx` fixes discovery
races, Bedrock availability and user-facing save behavior; `api.ts` now waits
for write errors. Its service-specific IDs, custom model input, key scopes,
connection probe, provider pool and routing controls remain available. API
write error propagation affects every admin mutation that shared the
`void handle` pattern, as listed by those call sites; successful empty-body
responses retain their existing `handle` semantics.

No Rust/server path, route ladder, model catalogue source, provider key
lifecycle, secret store, credentials precedence, or session contract changed.
No provider/model IDs were added to source. A catalogue remains provider
reported; exact IDs and provider-specific selection remain an admin task.

## Verification

- `npm --prefix crates/vak-client-ui run typecheck` and the client web/Tauri
  build passed; the embedded `dist-web` bundle was regenerated.
- `npm --prefix crates/vak-admin-ui run check` and build passed; the committed
  admin bundle was regenerated.
- `node crates/vak-client-ui/tests/model-choices.mjs`: 7 checks passed.
- `tests/ai-service.html`: 15 browser checks passed, including no writes on
  open, shared/agent scope, a delayed response, key and save errors, explicit
  model choice and Bedrock denial. Screenshots: `docs/assets/visual-refresh-2026/after/AI-service-*`.
- `crates/vak-admin-ui/tests/model-settings.html`: 7 browser checks passed,
  including delayed provider responses and Bedrock availability transitions.
- `crates/vak-admin-ui/tests/api-errors.mjs`: rejected route/key writes reach
  their caller with the server error.
- Visual inspection used the real Vite builds at 1440×900 and 390×844, light
  and dark. This was a deterministic fixture, not a live credential or
  provider test. The local UI fixture does not test the native desktop shell.
