# Native channel cards — implementation acceptance, 2026-10-22

Status: historical: source implementation and local acceptance of
[the native channel design](../design/102-channel-cards.md). Real provider/client
visual qualification remains open. The source acceptance below preceded installation; release/install evidence
follows separately. No message was posted to a real chat account.

## Implemented

`crates/vak-delivery/src/channel/forms.rs` validates and projects the twenty
current forms by `StructuredOutput.block`. The transient elements carry prose,
labelled facts, lists, tables, code, links and disclosure. A kind or topic never
selects the drawing. `inline.rs` parses authored prose syntax into flat styled
runs, keeps HTML literal, converts inline images to links and retains unsafe
link destinations as inert text. It does not classify prose or admit a card.

`channel/native/` contains separate Telegram, Slack and Discord compilers with
published protocol limits and structural continuation. Telegram emits rich
blocks and source disclosure; facts remain visible. Slack emits native rich
text/tables and complete per-message accessible text. Discord emits bounded
embeds with nested findings and points preserved. Long tables continue by
rows/columns, oversized cells become labelled records, and long links/code
are bounded without truncating destinations or nested tails.

`channel::messages` is the request plan for both bridge replies and proactive
adapters. All cards compile before the first HTTP request. Cards precede their
trailing narration; actions follow the final continuation. A malformed later
card prevents any send. The existing effect dispatcher still owns proactive
accounting and retry. Discord hashes the complete effect/message-index pair
before fitting its nonce to the provider's limit. A refusal stops delivery;
partial/unknown outcomes stay distinct.

System notices remain literal basic text: they are not answer cards. Removed
the three generic payload-key card projectors, their legacy-only fixtures,
the automatic Telegram HTML-to-plain retry and its compatibility facade. No
historical card-schema reader, model-name rule, topic list or new dependency
was introduced.

## Verification

- `cargo clippy -p vak-delivery -p vak-server --all-targets -- -D warnings`:
  passed, including tests.
- `cargo test -p vak-delivery -p vak-server`: passed **835 tests**, zero
  failures, five explicitly ignored tests, across 22 test/doc-test suites.
  This includes all 91 delivery unit tests and the server integration suite.
- Current-form coverage tests exercise all twenty forms on all three surfaces;
  the real screenshot's summary/sources shape, observation labels and source
  title/URL/date/finding are checked separately. Kind independence, native
  emphasis/links, inert HTML/media, more than ten cards, Unicode tails, lexical
  table values, wide/long tables, oversized cells, bounded code/link output,
  complete Slack accessible text, no blank sends and final action placement
  have explicit cases.
- Real HTTP fixture tests call production direct-send and proactive adapters,
  compare their requests to the shared compiler, assert the Telegram method
  and native provider bodies, stop on application-level refusal, preserve
  partial classification, and prove a bad later card sends nothing. The
  fixtures use local endpoints, not connected provider accounts.
- The worker-process test passed in the complete run and in isolation. A
  prior run timed out during concurrent rebuilding; no timeout was changed.
  Initial server test failures identified the missing `vak-tool-worker`;
  building both development binaries resolved that prerequisite. The bridge
  integration test caught notices being sent as embeds; restoring the explicit
  plain-status path fixed it, and all six bridge cases pass.
- Formatting, version stamps, record/index checks and document paths pass.
  Blueprint views were regenerated with no unclassified module/route findings.
- After the maintainer requested release and local install, `scripts/gate.sh`
  passed the full workspace gate: 3949 tests passed, 25 explicitly
  ignored, zero failures. Workspace Clippy, formatting, versions, records,
  doc paths and dependency advisories all pass. Log:
  `/tmp/vak-channel-full-gate.log`. Release/install qualification follows
  separately; no real provider/client visual acceptance is inferred.

Logs for this local run are `/tmp/vak-channel-tests-complete.log`,
`/tmp/vak-channel-clippy-complete.log`, and
`/tmp/vak-channel-build-final.log`. The debug linker reports its existing large
unwind-table warning when linking these large test/CLI executables.

## Still open

Qualify provider acceptance and appearance in the running Telegram, Slack and
Discord clients on desktop and phone, including source disclosure and overflow.
Local fixtures establish method/body construction and failure behavior, not a
provider's implementation or a client's visual rendering. Real outbound probes remain separate from source and installation acceptance.

## Local release and installation

The maintainer subsequently requested the version bump, release and local
installation. Version **8.23.0**, local tag `v8.23.0`, is built from
`9a6fb63fc`. `scripts/release.sh` passed the complete clean-room release gate:
3,949 tests passed, zero failed, 25 explicitly ignored, across 144 suites;
workspace Clippy and formatting passed, rebuilt frontends matched committed
sources, and version/record/advisory checks passed. The optimized native
`macos/aarch64` CLI, desktop and delivery worker are in `dist/8.23.0/`; all
three SHA256SUMS entries verify. Log: `/tmp/vak-8.23.0-release.log`. This is
a local release; no GitHub binary release was published.

The produced CLI installed all components and the desktop frontend through
`self install --force` into `/Applications/Vakyartha.app`, using `~/vak-home`.
`self verify` reports every component clean. The running gateway `/version`
reports `8.23.0 (9a6fb63fc)` and `/health` reports healthy with zero failures.
Service reconciliation restarted the gateway, desktop and Telegram bridge.
Telegram initially stood by while the stopped poller's 60-second cursor
liveness lease remained valid; no cursor or data file was changed by hand.
Provider/client appearance remains unqualified by this installation check.

After the lease elapsed, the Telegram launchd registration was unloaded and
loaded from its unchanged managed plist to clear the prior failed-launch
status. Final `self status` exits successfully: gateway, desktop and Telegram
are running with current managed binaries. `self verify` remains clean.
The displayed recent network warnings predate this final check.
