# Integrity, hostile-prompt and external-link hardening

Status: historical — implementation and synthetic acceptance record for the
2026-10-20 working tree; final gate results are recorded below.

The maintainer requested remediation of all five findings in the
[baseline audit](security-integrity-2026-10-20.md), safe external media and
functioning external hyperlinks in Tauri and the browser. No real secrets,
provider inference, install, release, commit or upload is used for acceptance.
A public example.com browser smoke is separate from the synthetic probes.

## Changes

| Finding | Boundary now enforced | Regression evidence |
|---|---|---|
| SI-01 | Remote answer images/audio/video/thumbnails are links rather than automatic requests; CSP permits only same-origin/data/blob media; street-map tiles require a click. Workspace and supported self-contained media still render. | Real Markdown, settled presentation, media/link cards and maps in fresh Chrome, both widths/themes/preferences; no automatic collector requests. |
| SI-02 | Worker admission no longer depends on optional owner telemetry. File/grep checks and native Seatbelt exclusions protect `.env` variants and workspace `.vak`; shell-computed paths cannot bypass them. Unqualified backends refuse. | Real broker worker plus native computed-path, hardlink, rename and ordinary-file-write tests. |
| SI-03 | Unknown stored/file/command content is both private and untrusted; editable Core admission context starts conservative. Mutations as well as exits require mixed-context approval. | Real ReadTool/WriteTool in scripted-provider agent tests; remembered/preallowed network permission does not suppress the ask. |
| SI-04 | Owner requests start private in both the live loop and ledger-derived turns, and labels survive carried context. | Owner-secret plus web and later-turn exit regression. |
| SI-05 | Strict read-only seal-manifest and inventory checks include the first segment's own seal. Read/receipt failures prevent a healthy verdict. Coverage explicitly excludes object/key/index payloads. | Corrupt/torn manifests, sealed-prefix truncation, missing chain without directory creation, changed frame, erased conversation and malformed kept-receipt tests. |

Both client entry pages disable speculative hyperlink DNS in metadata, and
embedded web/Admin HTML responses send `X-DNS-Prefetch-Control: off`.
[Chromium documents](https://www.chromium.org/developers/design-documents/dns-prefetching/)
that hyperlink hostnames can be resolved before a click and can carry private
information. Response/bundle tests verify this opt-out alongside media CSP;
the loopback HTTP collector is not a measurement of DNS traffic or every
browser vendor's speculative-navigation behavior.

The scheme allowlist is not authority to send data. In-app external media
requires a governed fetch into a local artifact before viewing; no new
arbitrary-URL media proxy or automatic provider iframe is introduced here.
Downloaded bytes remain untrusted decoder input. Unsupported active SVG/HTML
must not be promoted to trusted application content.

External hyperlinks use one delegated handler, including streamed anchors.
A trusted click invokes the Tauri system-browser command; a web click keeps
normal browser navigation in a separate tab. Synthetic clicks are blocked,
app routes/downloads retain their own behavior, and links carry no referrer
or opener access. The native boundary rejects arbitrary schemes, credentials,
control characters and overlong addresses; the opener receives one URL
argument with no shell interpretation. Native command validation and the
shared desktop dispatch are tested; an installed Tauri release was not
launched or replaced during this work.

Fetch's connector DNS resolution rejects private/special-use answers and pins
the sockets actually connected. Ambient HTTP proxies are disabled. Chromium
uses a private profile, a screened SOCKS proxy and OS egress confinement;
destination screening applies to each CONNECT, including redirects and
subresources. Proxy teardown also aborts accepted tunnels. SOCKS tests cover
private IPv4/IPv6, mapped addresses and localhost DNS destinations.

Comment-driven revisions now forward mutation approvals to the parent
conversation's existing HTTP approval channel. Their child remains Workspace
Write in an isolated copy; no automatic allow or Full Access fallback is added.
The revised text/Office tests answer the real parent approval and retain their
assertions that the original workspace does not change. The learning lifecycle
fixture explicitly approves its two intended mutations and verifies that the
editable admission context raised both asks before any file read.

## Additional output finding: WebRTC preview egress

A fresh Chrome probe with the prior `allow-scripts` opaque-origin document
and `connect-src 'none'` sent **five UDP/STUN packets** to a synthetic loopback
collector. No user data left the machine. Static previews now block scripts
at both iframe and CSP boundaries. Origin-served document previews retain
only the hash-authorized, fixed theme bridge; arbitrary scripts and event
handlers cannot run. Workbench/result thumbnails and popout wrappers preserve
these restrictions. Repeating the probe against static and origin policies
observed **zero packets and no attack script execution**. The fixed theme
bridge still accepted the parent theme update. The source probe
is retained with this audit. Hash tests bind the allowed script to the actual
server bridge bytes. Script-powered HTML behavior is a compatibility loss;
layout, native inputs and local media remain. Explicit live dev-server
previews are a separate user-started capability, not an offline document.

Channel attachment round-trip fixtures now configure the existing forwarded
approval mode and answer from its synthetic approver chat before editing.
Library write/Office comparison HTTP fixtures answer the real pending API
request and assert exactly one approval. This preserves the transport and
version checks while respecting the new mutation policy; unattended deny
mode is not relaxed. Sandbox fixtures distinguish denied operations from
write allowances, and the shell control-file stress case uses the real native
sandbox with a computed path rather than a command-text match. All 30 tool
integration tests and tool Clippy pass after those fixture corrections.

## Acceptance

- Browser fixtures: `crates/vak-client-ui/tests/security-boundaries.html`
  and `crates/vak-admin-ui/tests/data-integrity.html` exercise real renderers.
  Client: 1440 × 900 and 390 × 844, light/dark, media setting on/off; supported
  inline image preserved, map consent present, eight explicit external links,
  zero horizontal overflow and zero automatic marker requests. Trusted native
  dispatch, synthetic click refusal and a separate web tab without a referrer
  are separately exercised. Trusted clicks on rejected schemes and credential-bearing
  addresses are also refused without falling through to default navigation.
- Admin: both viewport sizes and themes, complete/incomplete reports; incomplete
  verification is never presented as healthy, and excluded coverage is visible.
- Client and Admin production builds pass; 37 frontend pure-test scripts pass
  (the client-event fixture checker requires generated input and is separate).
  Saved runners, result JSON and representative screenshots are under
  `docs/audits/security-hardening-2026-10-20/`. Launch the Vite fixture servers
  with `VAK_HOST=web node_modules/.bin/vite --host 127.0.0.1 --port 39492`
  in the client package and the corresponding Vite command on port 39494 in
  Admin, then run the saved browser-check scripts with Node. The runners use
  the locally installed macOS Chrome and disposable profiles/loopback collectors.
- Native browser IPC permits only the private profile's Unix sockets and the
  browser's per-process Mach rendezvous namespace; direct TCP remains denied
  except for the explicit screened proxy. Native private-socket/outside-socket
  and direct-TCP refusal regression passes. macOS's hermetic temp override is
  set from the private profile, following [Chromium's implementation](https://chromium.googlesource.com/chromium/src/+/refs/tags/146.0.7680.180/base/files/file_util_apple.mm).
  Read-only extensions are confined to Chrome's already-readable bundle for
  its nested child sandbox; no user-profile write allowance is added.
- Live rendered-browser smoke fails under the tightened native policy on this
  Mac: Chromium exits before producing a page. This is a recorded compatibility
  failure, not a passing browser acceptance result. The production tool returns
  an error and recommends ordinary fetch; it does not retry without confinement.
  Keep the ignored public-page smoke as the executable acceptance criterion.
- Full `scripts/gate.sh` passes: formatting, workspace Clippy, 3,883 passing
  tests with zero failures and 24 ignored, versions, documentation paths,
  records and advisories. Workspace tests took 885 seconds. The final run used
  the corrected fixtures throughout; the previous run used stale tool-test
  binaries and is not counted as a passing gate. The ignored public-browser
  smoke remains a separately measured compatibility failure as described above.
  See `security-hardening-2026-10-20/verification.txt` for the compact result.

## Limits

The secure rendered browser is currently unavailable in the measured local
Chrome setup. Private-IPC tests and proxy destination tests do not establish
successful page rendering. This is tracked separately from closing automatic
browser egress and must not be described as complete rendering support.

Full Access is still an explicit unsandboxed trust decision and bypasses the
mixed-provenance ask. A person may authorize a harmful mutation or follow a
misleading external link; no prompt wording proves that action safe. Models
cannot elevate the mode automatically. Restricted Linux Landlock and Docker
workers/parsers/MCP currently refuse because their backends do not yet enforce
the mandatory control-file exclusions; this is a compatibility restriction,
not Linux parity. Browser-native/OS vulnerabilities, descriptor-relative
filesystem races, independent reference-reader and multi-process storage
assurance, complete chain deletion/rollback detection without an external
root, and object/key/index payload verification remain outside this closure.
