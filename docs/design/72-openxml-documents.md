# 72 — Office documents (Open XML): review and implementation ledger

Status: **proposal with an implementation ledger. Opened 2026-09-23; scope widened the same day by owner direction.** Everything under "What exists today" is in the tree. Nothing under the phases is built until its box is checked with evidence. Vak has zero users, so nothing here carries a compatibility path (invariant 29): when a phase replaces an existing mechanism, the older one is removed in the same change (invariant 30).

Review 2026-09-24: the plan was checked against the tree before P0 started. Corrections are folded in place: F1, F3 and F4 were restated from the code, F11 and F12 were added, the Excel and Word anchor stability claims were wrong and are fixed, the rule changes now land with the phase that enforces each one, and D9 (cryptography) was missing.

## Owner direction — 2026-09-23

- **Full Open XML support, done the most secure way.** Design and repository rules may change to allow it. There are no users to protect from a change.
- **Vak is self-sufficient.** A user may have neither Microsoft Office nor LibreOffice installed, and Vak must not need either. Reading, rendering, calculating, editing, creating, verifying and exporting all happen inside Vak.
- **Opening a file in another app is a courtesy, never a dependency.** When the user's machine has an application registered for the file type, Vak may offer **Open with…** and pick up the changes when the file is saved back. Nothing in Vak depends on it.

## Why this exists

Vak is not an Office replacement. People and organisations hold large amounts of knowledge in Word, Excel, PowerPoint and Visio files, and most of the work done to those files is small: read, find, cite, add a section, fix a table, update some cells, assemble a deck from a brief, comment, review and hand back. Agents and people need a light, reliable way to do this work together. They do not need a heavy suite.

| Use case | Shape of the work |
|---|---|
| Knowledge | A folder of existing Office files is searchable and quotable. An answer cites `file#anchor` (slide 7, `Budget!C12`, a paragraph) instead of paraphrasing a blob. |
| Human ⇄ Agent co-work | A spreadsheet or document is changed by both a person and an Agent. Every change is attributed, reviewable and reversible, and the file stays valid Open XML that any conforming application opens. |
| Human ⇄ Human, Agent ⇄ Agent | The same file moves between invited people (`69-shared-conversation-coworking.md`) or between Agents (researcher → writer). Handoff happens through versions and receipts, not "the latest file on disk". |
| Made on the fly | An Agent creates a deck, memo, workbook or diagram from a brief, often from the organisation's own template. A person previews it, comments and asks for a revision. |
| Adapting existing files | An existing file is changed while everything Vak does not edit is kept exactly: styles, themes, charts, SmartArt, macros, custom XML, and extensions that Office adds later. |
| Headless | The same operations work with no UI: CLI, HTTP, channels (a Telegram user sends a workbook and gets the updated one back) and scheduled jobs (update the weekly report every Monday). |

## Scope: the Open XML family

"Full support" means the ECMA-376 / ISO/IEC 29500 family, in both **Transitional** and **Strict** conformance, including the templates and macro-enabled variants:

| Vocabulary | Extensions |
|---|---|
| WordprocessingML | `.docx` `.docm` `.dotx` `.dotm` |
| SpreadsheetML | `.xlsx` `.xlsm` `.xltx` `.xltm` `.xlam` (add-in) |
| PresentationML | `.pptx` `.pptm` `.potx` `.potm` `.ppsx` `.ppsm` `.ppam` (add-in) |
| VisioML (MS-VSDX) | `.vsdx` `.vsdm` `.vstx` `.vstm` `.vssx` `.vssm` |
| Shared | OPC packaging, DrawingML (shapes, pictures, preset geometry, themes), DrawingML charts, SmartArt (diagram data plus its cached drawing), core, app and custom properties, custom XML parts, thumbnails, digital signatures, embedded packages, ECMA-376 Agile-encrypted packages, and **VBA projects** (MS-OVBA source and modules, MS-CFB storage, MS-OFORMS UserForms, form controls bound to macros) |

Outside the family, and reported as unsupported with an **Open with…** offer where the host has a handler: legacy binary formats (`.doc` `.xls` `.ppt` `.vsd`), `.xlsb` (Open XML packaging around binary parts), ODF, and IRM/RMS rights-managed files. Vak does not install or call an external converter for these, because that would bring back the dependency this direction removes.

## Engineering rules for this area

O1–O10 are the area invariants. Their `AGENTS.md` form is under "Rule changes", and they land there in the same change as P0, so each rule and its enforcement arrive together.

- **O1 — Lossless by default.** A part Vak did not edit is copied byte-for-byte, and its compressed ZIP entry is copied raw. An edited part keeps every element, attribute, namespace and `mc:AlternateContent` branch that Vak does not model. Edits are splices into the XML event stream, not a re-serialisation from a partial object model. Strict files stay Strict and Transitional files stay Transitional. A no-op round trip across the whole corpus must be byte-identical for every part.
- **O2 — Packages are hostile input.** Vak parses them only inside the broker worker (invariant 14), with explicit limits on entry count, per-part and total decompressed bytes, compression ratio, XML depth, attribute count and embedded-package nesting. Byte limits are enforced on the bytes actually decompressed (a bounded `Read`), never on the sizes the ZIP directory declares, because a hostile archive lies about those. It refuses `DOCTYPE` and entity declarations (`quick-xml` never expands entities, so this is an explicit refusal of markup no Office producer emits, not a patch over an expansion bug). Part names containing `..`, absolute paths, duplicates or case-insensitive collisions are rejected. `TargetMode="External"` is never followed. DDE, OLE, ActiveX, field code, Excel 4.0 (XLM) macro sheets, external data connections and Power Query are never executed or refreshed. VBA runs only in Vak's own runtime under O10, never in Office and never natively on the host.
- **O3 — One operation vocabulary.** Agents, people in the UI, the CLI, HTTP and channels all change a document through the same typed `OfficeOp` set, applied by one engine. There is no second write path.
- **O4 — Anchors are exact.** Every operation names its target by an anchor that Vak returned, bound to the base digest and the anchor's own revision. A stale or missing anchor produces a repairable error. The engine never guesses the "nearest" paragraph.
- **O5 — Evidence is observed.** After an operation, the engine re-reads the written package and checks the operation's postcondition, then schema-validates the result. Both are `Observed` evidence (invariant 33). Structural, calculated and rendered checks are reported separately, and none of them stands in for another.
- **O6 — Document content is data.** Text, comments, alt text, hidden runs, white-on-white or off-slide content, speaker notes, custom properties and macro source are untrusted model input. The reader labels hidden and off-canvas content so that a prompt injection hidden in a document is visible for what it is.
- **O7 — Vak needs no Office.** Reading, editing, creating, calculating, rendering, validating and exporting run on Vak's own code and pinned libraries. No Microsoft Office, LibreOffice, .NET or other external application is ever a runtime dependency. External tools may serve as **CI test oracles** only, and their results are never shown to users as checks.
- **O8 — Protection is honoured and secrets stay secret.** Document protection (read-only recommended, locked cells, protected sheets and ranges, restricted editing) limits ops unless the human owner explicitly overrides it with an op. A document password is a human-supplied secret: it is held in memory for one operation, is never model-visible or written to the ledger, and is never persisted unless the user stores it in the secret scope (invariants 8, 12 and 27).
- **O9 — Labels only narrow.** Sensitivity labels found in a package (for example `MSIP_Label_*` custom properties) are read, shown, and can only restrict egress: channel delivery, invitations and webhooks. A label never grants access. This is the same meet-only rule as invariant 32.
- **O10 — Macros are governed code.** VBA is supported: it is read, explained, indexed, edited and run. Every step is governed:
  - **Run:** only in Vak's own VBA runtime inside the broker worker. The only effects a macro can reach are `OfficeOp`s on a draft, brokered file tools under `PermissionEngine`, and prompts to a person. Win32 `Declare`, `Shell`, raw COM and network are unreachable unless a named, narrowly scoped emulation exists and permission grants it. Runs are budgeted and cancellable (invariant 5), and their results land in a draft for review, never directly in the file.
  - **Author:** an Agent may write or change VBA, but every VBA change requires a human review of the exact source diff. No envelope, schedule or auto-accept can pre-authorise it (invariant 32: irreversible work reaches a human). Adding an auto-run entry point, or turning a macro-free file into a macro-enabled one, needs its own explicit confirmation.
  - **Never, in any case:** DDE or other auto-executing fields, new external data connections, or hyperlinks with `javascript:`, `file:` or UNC targets.

## Review of the current implementation — 2026-09-23

### What exists today

- [x] `OpenXmlPackageVerifier` (`crates/vak-sandbox/src/lib.rs`, id `format.openxml`) opens DOCX, XLSX and PPTX candidates with `zip` and `quick-xml`. It checks the roots of `[Content_Types].xml`, `_rels/.rels` and the main part, parses the main part in full, and reports paragraph, worksheet or slide counts. It runs on the frozen draft before review and again from the applied workspace (`54-task-environments-and-promotion.md`).
- [x] `crates/vak-tools/src/artifact.rs` maps the three extensions to their Open XML MIME types.
- [x] `crates/vak-intent/src/outcome.rs` treats a request that names a `.docx`, `.pptx` or `.xlsx` file as a saved-file deliverable.
- [x] Reusable platform pieces this ledger builds on rather than re-implementing: frozen candidates, isolated revisions, crash-safe promotion and undo (`54`); anchored, attributed candidate comments and scoped invitations (`69`); Canvas and Review (`70`); the `DataGrid`, `Table`, `Chart`, `Graph`, `Document` and `File` primitives (`crates/vak-presentation/src/lib.rs`); `zip` and `quick-xml`, already pinned in the workspace manifest.

That is the whole Office implementation. Vak can confirm that three Office formats are structurally packages. It cannot read, edit, create, render, calculate or diff one.

### Findings

| # | Severity | Finding |
|---|---|---|
| F1 | **Bug** | `compose_prompt` in `crates/vak-server/src/gateway.rs` inlines every `document` attachment under 64 KiB with `String::from_utf8_lossy`. Any non-text file (not only Office: PDFs, archives, images sent as files) therefore lands in the model-visible prompt, and in the ledger, as replacement-character noise. The model sees garbage, tokens are wasted, and the file itself is not kept anywhere the Agent could read or edit it. The two caps also disagree: the Telegram bridge downloads documents up to 256 KiB (`DOCUMENT_MAX_BYTES` in `surfaces/telegram.rs`), and the gateway then drops anything over 64 KiB with a message telling the sender to send an excerpt. |
| F2 | Gap | `doc_read` (`crates/vak-core/src/doc_reader.rs`) uses `read_to_string`, so it fails on every Office file. `read` returns `[binary file]`. An Agent's only route to the contents is ad-hoc scripting in `bash` with whatever libraries the sandbox happens to have. That is ungoverned, unreproducible and not anchored. |
| F3 | **Boundary** | `doc_read` is registered in-process in `Core` (`scoped_tools`), not behind the worker (invariant 14). It is harmless while it only parses text, but Office parsing must not be added there. The worker cannot simply be pointed at it: `broker::worker_main` dispatches only `vak_tools::default_tools()`, and `doc_read` lives in `vak-core`, which the worker crate cannot depend on. It has to move into `vak-tools`. `data_query` is *not* a file parser today: it takes only an inline `data` string the model supplies and never opens a path, so it stays in-process until it gains a file source (P1), and moves to the worker in that same change. |
| F4 | **Boundary** | Target verifiers, including the Open XML one, run inside the `vak-server` process (the three `verifiers.verify` call sites in `crates/vak-server/src/lib.rs`). Every candidate's package is therefore unzipped and XML-parsed in the server that holds credentials and the control plane. Two of the three sites (candidate export and isolated-revision freeze) also run that synchronous parsing directly on the async runtime; only promotion uses `spawn_blocking`. Verifiers are not model tools, so moving them needs a verification request kind in the worker protocol, not a pseudo-tool. |
| F5 | Hardening | The verifier locates the main part by a hard-coded path instead of the package's `officeDocument` relationship. It has no ZIP entry-count or decompression bound (the PDF verifier has one), so a ZIP bomb in a candidate is decompressed during review. It dispatches on the file extension rather than the main part's content type. It does not distinguish macro-enabled, Strict, signed or encrypted packages. Visio is not covered. |
| F6 | Gap | Canvas `detectType` (`crates/vak-client-ui/src/components/ArtifactCanvas.tsx`) sends Office files to the `code` view, which tries to render ZIP bytes as source. |
| F7 | Gap | Review compares files as text, so an Office candidate has no meaningful diff. Accepting it is a blind decision, which contradicts `70`'s "show the actual candidate, exact changes". |
| F8 | Gap | The outcome evaluator can require a `.docx`, `.xlsx` or `.pptx` deliverable, but no governed tool can produce one. Office requests therefore reliably end `Unknown` or in improvised scripts. |
| F9 | Gap | Candidate comments anchor to a file plus an optional line range. A line number means nothing in a binary package, so a comment cannot point at a cell, paragraph, slide or shape. |
| F10 | Gap | `data_query` accepts inline CSV, JSON or Markdown text only and has no file source at all. A worksheet cannot be queried except by the model pasting its contents into the call. |
| F11 | Hardening | The broker worker is OS-sandboxed only in restricted permission modes. Under FullAccess it runs unsandboxed by design (invariant 13), and it has no wall-clock deadline and no memory limit (macOS has no working `RLIMIT_AS`, and `unsafe` is denied outside `bash.rs`). The worker is therefore process isolation plus in-code bounds, and O2's bounds are the primary control against hostile packages, not defence in depth. |
| F12 | Hygiene | `zip` and `quick-xml` are in the workspace manifest with semver ranges (`"4"`, `"0.41"`), not exact pins. Once they are the hostile-input boundary they are pinned exactly. |

## Architecture

Everything that touches package bytes is a pure crate with no dependency on other vak crates (the discipline `vak-intent` follows), so it can be tested and fuzzed without a server, model or sandbox. Every call site runs it inside the broker worker.

```
crates/vak-ooxml          the package and document engine
  L0 package    bounded OPC reader/writer: content types, relationship graph,
                part addressing by relationship, raw-copy writer, deterministic
                entry order, Agile encryption container, signature parts,
                security refusals (O2)
  L1 xml        quick-xml event streams, located ranges, splice editing that
                preserves unknown markup (O1), mc:Ignorable / AlternateContent,
                Strict <-> Transitional namespace mapping for reading
  L2 models     read projections with anchors for Word, Excel, PowerPoint,
                Visio, DrawingML, charts, SmartArt, properties, custom XML,
                VBA project (MS-CFB storage, MS-OVBA source and dir stream,
                references, UserForm structure, entry-point map)
  L3 ops        typed OfficeOp set, one apply engine, per-anchor revisions,
                postconditions (O3-O5, O8, O10)
  L4 diff       semantic diff and three-way merge at anchor granularity
  L5 project    typed, paged OfficeProjection for UI, cards and headless
  validate      schema conformance tables derived from the published ECMA-376
                and MS-VSDX schemas, plus semantic rules (relationship closure,
                index bounds, unique ids, sheet-name and defined-name syntax)
crates/vak-ooxml-calc     formula parser, dependency graph and evaluator (P3)
crates/vak-vba            VBA language (MS-VBAL) parser and interpreter with an
                          emulated object model that turns Range/Cells/
                          Worksheets/Selection/Documents/Slides calls into
                          OfficeOps; allowlisted library emulations
                          (Collection, Scripting.Dictionary, RegExp, a
                          workspace-confined FileSystemObject); MsgBox/
                          InputBox/UserForms bridged to Vak prompts and forms;
                          step, memory and time budgets (P9)
crates/vak-ooxml-layout   page and slide layout to a typed display list;
                          bundled metric-compatible fonts; PDF and PNG
                          writers; rendered-check measurements (P4)
```

The display list is typed data: positioned glyph runs, paths, fills and image references. The client draws it with Vak's own SVG/canvas code, the PDF writer prints it, and the visual verifier measures it. All three read one layout. No surface mounts HTML from a document, and no second layout engine exists in TypeScript.

### Anchors

| Vocabulary | Anchor | Stability |
|---|---|---|
| Word | `w14:paraId` when present; otherwise Vak adds a paraId in the first op that touches the paragraph (which also declares the `w14` namespace and lists it in the root's `mc:Ignorable`). A read never adds one: reading must leave the package byte-identical (O1). Read-only anchors are a structural path plus a content hash valid for one digest. | A paraId usually survives editing in Word, but Word may reassign one (duplicates, some paste paths). A paraId that no longer resolves after an external edit is stale (O4), never matched to the nearest paragraph. A path-and-hash anchor never outlives its base. |
| Excel | `Sheet!A1`, `Sheet!A1:D20`, defined names, table names | **Not stable by construction:** inserting or deleting rows or columns moves every address below or to the right. The op engine rebases cell and range anchors through each structural op in the draft's op log. An external edit cannot be rebased, so anchors against the old digest become stale. Defined names and table names are the stable anchors. The sheet is resolved by name, with `sheetId` recorded so a rename is detected. |
| PowerPoint | slide `p:sldId@id`, shape `p:cNvPr@id` within the slide | Stable across reordering. A shape id is unique only within its slide, and copy and paste can renumber it, so a shape anchor always carries its slide id. |
| Visio | page ID, shape ID | Stable. |
| Charts and SmartArt | the owning anchor plus the chart or diagram part relationship | Stable while the owner exists. |

Each anchor carries a revision counter held in the draft's op log. An op states the revision it expects (compare-and-set), which lets live co-editing (P6) stay exact without guessing.

### Tool surface (one way each)

- **`doc_read`** (brokered) serves the whole family through the existing `text`, `summary`, `outline` and `table` views and the `section` parameter, with anchors on every unit and explicit labels for hidden content, comments, tracked changes, macros, signatures and labels.
- **`office_apply`** (brokered, a write claim on the path, and a normal `PermissionEngine` decision) takes a base digest and ordered `OfficeOp`s. It returns the new digest, per-op postcondition results, validation results and a compact semantic diff. **Creation is the same tool:** `create: {kind, template?}` starts from a built-in blank package or a workspace template.
- **`data_query`** (brokered) accepts worksheet ranges and tables as sources.
- **`office_run_macro`** (brokered, `Ask` by default) runs a named macro from a file's VBA project in the Vak runtime against a draft. It returns the ops produced, the prompts shown, the run receipt, and any point where the runtime stopped because a feature is unsupported. VBA source edits go through `office_apply` as ordinary ops (`vba_set_module`, `vba_add_module`, `vba_remove_module`), so there is still one write path.
- Op families, pinned per phase: Word text, paragraphs, styles, lists, tables, sections, headers and footers, footnotes, images, comments, tracked-change accept/reject, content controls and fields (non-executing only, O10); Excel cells, formulas, rows and columns, sheets, number formats, styles, merges, tables, defined names, data validation, conditional formats, comments, charts; PowerPoint slides from layouts, placeholders, shapes, text, tables, images, notes, order, charts, comments; Visio pages, shapes from stencil masters, text, shape data, connectors; shared: properties, `prepare_to_share` (removes personal metadata, hidden text and, optionally, comments).
- Agent edits to an existing Word document are written as **native tracked changes** authored as the frozen Agent identity (`71-agent-character-system.md`). New documents the Agent creates are written clean.
- Formulas are calculated by Vak (P3). Before P3 lands, an edited formula is written with `fullCalcOnLoad` and its cached value is reported as **stale**, never as current.

### How it joins the existing platform

- **Candidates** (`54`): every write happens in a draft, freezes as a candidate, runs the verifiers in the worker, goes through Review and is accepted atomically. Office files gain no private path.
- **Review** (`70`): the semantic diff replaces the blind binary comparison.
- **Comments** (`69`): candidate comment anchors gain an Office anchor variant alongside file plus line.
- **Canvas**: the Office views below replace the Code fallback.
- **Channels**: inbound Office attachments are saved into the Agent workspace inbox and summarised through the reader (fixes F1). Outbound deliverables go as document attachments with provenance, subject to label narrowing (O9).
- **Knowledge** (P8): a digest-keyed, rebuildable index of workspace Office files with anchors.

## Security model

Threat model context: `24-agent-security.md`. Office files arrive from strangers through channels and shared folders, and they are among the most abused document formats in the world. The design assumes every package is hostile.

| Surface | Handling |
|---|---|
| Parsing | Only in the broker worker, a disposable process, with the O2 bounds. The worker carries the session's OS sandbox in restricted modes and none under FullAccess (F11), so the O2 bounds hold on their own. Verification runs in a worker under a dedicated read-only, network-denied sandbox rooted at the tree being verified, whatever the session's mode, with a wall-clock deadline. A verification that cannot run fails closed as a failed check. |
| Fuzzing | `cargo-fuzz` targets for OPC reading, XML splicing, each L2 model, the op engine, the encryption container, the VBA decompressor, the formula parser and layout. They run on every change and nightly with a growing corpus. A crash is a release blocker. |
| XML | No DTD, no entity expansion, depth and attribute bounds, namespace allowlists for the parts Vak models, and unknown parts preserved but never interpreted. |
| External relationships | Recorded and shown (linked images, remote templates such as `attachedTemplate`, external workbook links, hyperlinks). Never fetched or resolved. A remote template is flagged as a known phishing vector. |
| VBA | Governed by O10 and the "Macros and automation" section below. Preserved byte-for-byte unless a reviewed op edits it. Static analysis runs on every read and flags risky calls, auto-run entry points and external dependencies. Execution happens only in Vak's runtime. |
| Excel 4.0 (XLM) macro sheets | Preserved, shown and flagged. Never executed: this legacy format is chiefly used today for malware. |
| ActiveX controls | Preserved and shown as labelled placeholders. Never instantiated. Form controls (buttons bound to macros) are supported through the runtime. |
| Fields, DDE, OLE, embedded packages | Preserved and never activated. DDE and include-type fields are flagged. Embedded Open XML packages are read recursively up to the nesting bound. Other OLE objects show as labelled placeholders. |
| External data | Connections, Power Query, query tables and pivot caches are preserved and shown, and never refreshed. Pivot tables display their saved cache, labelled as such. |
| Digital signatures | Verified (XML-DSig over the signed parts, with certificate chain shown as observed). An edit that would invalidate a signature is announced in Review before acceptance. The edited output drops the now-invalid signature parts and records that in the receipt, rather than shipping a file that claims a signature it no longer has. |
| Encryption | Agile-encrypted packages are decrypted in the worker with a human-supplied password (O8) and re-encrypted with the same password on save. IRM/RMS files are reported as unsupported. |
| Sensitivity labels | Read and shown. They narrow egress only (O9). A labelled file cannot go to a channel or invitation whose policy the label excludes. |
| Document protection | Honoured by ops (O8). Protection hashes are never cracked. The owner can remove protection by an explicit op, which Review shows. |
| Images and fonts | Images are decoded in the worker and served to clients as re-encoded PNG or WebP through an authenticated per-part route. EMF/WMF are converted by Vak's bounded converter or shown as placeholders. Embedded (including obfuscated) fonts are never loaded into a client; rendering uses bundled fonts. |
| Prompt injection | O6 labelling. Document text is tool output, never instructions, and hidden content is marked in both the tool result and the UI. |
| Privacy | `prepare_to_share` removes personal metadata, revision identifiers, hidden text and optionally comments. Review offers it before any external delivery. |

## Macros and automation

VBA macros hold two decades of real automation and business logic: monthly closes, report builders, data cleaners, templates that fill themselves in. Treating them as inert blobs would throw that knowledge away. Vak therefore supports macros at four levels. Each level is independently useful, and each is governed by O10.

| Level | What a person or Agent can do | How it stays safe |
|---|---|---|
| **Understand** | See every module, procedure, reference and UserForm. See what triggers each macro (a button, `Workbook_Open`, a ribbon or keyboard binding, a worksheet event). Get a plain-language explanation. Search macros as knowledge across the whole library (P8). | Parsing happens in the worker. Static analysis is `Observed` evidence. An Agent's explanation is labelled as the Agent's reading (`Asserted`) and never presented as verified fact (invariant 33). |
| **Run** | Press a macro's button in U2, choose Run in the Automations panel, or have an Agent call `office_run_macro`. The macro's effects appear live in a draft and are reviewed like any other change. `MsgBox`, `InputBox` and UserForms appear as Vak prompts and forms in both the desktop app and the web client. | The runtime reaches only its emulated object model, permission-gated brokered tools and prompts. Before a run, Vak shows what the macro **may** do from static analysis ("change cells in Budget, read files in the workspace, ask you 2 questions"). The runtime then refuses anything beyond what was granted. Unsupported features stop the run at an exact line with a clear reason. A run never half-applies to the real file, because everything lands in a draft. |
| **Author** | An Agent fixes, extends or writes a macro. A person edits source in the Automations panel. | A VBA change always needs human review of the source diff with its analysis flags. It can never be pre-authorised. New auto-run entry points and macro-free-to-macro-enabled conversions need separate confirmation. An edit invalidates the VBA project signature, and Review says so before acceptance. |
| **Migrate** | Turn a macro into a Vak automation (an `AgentSchedule`, a flow, or an Agent skill using `office_apply` and `data_query`) that runs headless, on channels and on a schedule. | Parity check: both the macro in the Vak runtime and the migrated automation run on the same sample input, and Review compares their op lists. Migration never removes the original macro unless a person asks. |

**Trusted macros.** An owner can mark one macro as trusted, bound to its source digest and a declared capability set. It then runs without a prompt, within exactly those capabilities, and only while the source is unchanged. A changed digest drops the trust automatically. This is a narrow, revocable grant like any other permission rule. It never implies FullAccess (invariant 13), and it never extends to files from an untrusted source (a channel upload, a shared folder), which always prompt.

**Provenance.** Every run appends a receipt to the ledger: the file digest, the macro name and source digest, who invoked it (a person or an Agent) and under which grant, the answers given to prompts, the ops produced, and the stop reason. A run can therefore be replayed and audited, and the draft it produced reviewed with its full history.

**Locked projects.** A VBA project locked with a password (the VBA editor's view lock) is honoured under O8. Vak can run it (as Office would) and statically analyse it for flags, but it does not display or edit the source without an explicit owner-override op. The lock is never cracked, and Vak never displays or edits the source without that owner override.

**Runtime coverage is measured, not claimed.** VBA and the Office object models are large. Coverage is tracked the same way as the support matrix: language features and object-model members, each with tests, plus a corpus measure (the share of real macros in the corpus that run to completion with results matching their expected output). The Excel object model comes first, because macro-enabled workbooks hold most real automation, then Word, PowerPoint and Visio. Where coverage ends, Vak says so at the exact line, and **Open with…** remains the courtesy route on desktop for people who have Office.

## UI and UX

Office files follow the same calm experience as everything else (`70-calm-agent-experience-implementation.md`): outcome first, machinery one deliberate step away, and the composer always available. Office views should feel familiar to someone who uses Word, Excel or PowerPoint (sheet tabs at the bottom, a slide rail on the left, redline markup) without copying a suite's ribbon. A person reads, points at something, comments, asks the Agent, makes an edit, reviews and accepts, all without leaving Vak. For heavy formatting work, **Open with…** is an optional courtesy (U10).

### Rules for every Office view

- **Everything comes from the projection, the layout or the ledger.** Views render the typed `OfficeProjection`, the display list and recorded receipts. Counts, change lists, author chips and checks are never inferred on the client. The browser never unzips a package.
- **No mounted HTML.** Document content is drawn by Vak's components (the rule in `AGENTS.md`). SVG produced from Vak's own display list is Vak's own markup, not document content.
- **Two ways to look, one set of anchors.** *Pages* is the faithful laid-out view from `vak-ooxml-layout` (pages, slides and diagram pages as they will print). *Flow* is a reflowed reading view for narrow screens and long documents. Both expose the same anchors, so selecting, commenting and editing work in either.
- **Point, then act.** One gesture everywhere: select something (a text range, a cell range, a slide or shape, a diagram shape) and a small floating bar offers **Comment**, **Ask Agent** and, when the viewer has edit authority, **Edit**. Choosing one puts an anchor chip into the composer or comment box, for example `About: Budget!B4:D20` or `About: Slide 3 · Title`.
- **Security and hidden content are shown quietly but plainly.** One banner line collects what the reader flagged: macros (with their entry points and flags), external links (not followed), hidden text in N places, a signature and its state, a sensitivity label, protection, and encryption. Each entry opens the Structure view at the exact location.
- **Presence and authorship are observed facts.** Author chips use the frozen Agent identity and character mark (`71-agent-character-system.md`) or the verified principal (`69`). A presence avatar appears only for an observed stream. The Agent's "editing here" marker appears only while an op for that anchor is being applied.

### Desktop and web: one client

Desktop and web are one product. The Office UI lives only in `crates/vak-client-ui`, which builds twice from one source: `dist/` for the Tauri shell and `dist-web/` for `/app` (`48-web-client.md`). Every Office view, card, Review surface and Canvas mode is a shared component. No component checks which host it is running on. The only differences come from `host.can(...)` answers on the `Host` port (`crates/vak-client-ui/src/host/port.ts`), and an absent capability renders differently rather than brokenly.

The same boundary keeps the two identical. The desktop shell embeds the same `secured_router` the web client talks to, so projection, display lists, calculation, validation, diff, export and the op engine are the **same server routes with the same responses** on both. Neither client contains Office logic or native code for Office. It draws what the server returns and sends ops back.

| Concern | One path for both | Where the host differs |
|---|---|---|
| Rendering | The server's display list, drawn by shared client code. Bundled fonts are served from an authenticated asset route as web fonts, never taken from the operating system, so pages look the same on every host. | None. Every webview and browser in the test matrix below must pass. |
| Getting a file in | HTML5 file input and drag-and-drop post bytes to one authenticated upload route into the Agent inbox. | Tauri 2 intercepts OS file drops by default. Set `dragDropEnabled: false` in `crates/vak-desktop/tauri.conf.json` so the webview gets the same HTML5 drop as a browser, and verify on each OS. No host gets a second intake path. |
| Getting a file out (a version, a PDF) | Server-produced bytes. | The `Host` port's `saveText` becomes `saveFile(name, bytes, mime)`. It opens a native save dialog on desktop and downloads on web. It **replaces** `saveText` and its existing call sites move over in the same change (invariant 30), rather than a second save method being added. |
| Open with… | Optional (O7). | A new `HostFeature`, `open-with`, answered only by the desktop shell. It is backed by a narrowly scoped opener capability limited to workspace files, with the server re-checking path confinement. The web client never has it and shows **Download** instead. |
| Coming back after editing elsewhere | Server-side detection. The workspace file's digest changes (desktop Open with…), or a re-uploaded file carries a lineage id (either host). Both lead to the same external-change card and three-way merge. | None in the flow; only how the bytes arrive. |
| Live ops, presence, comments | The existing reconnectable SSE and EventBus, with `Last-Event-ID` gap replay. | None. |
| Password prompt | One shared component posting to the worker-bound route. Never stored by the client. | None. |
| Present mode | The DOM Fullscreen API on the Canvas element. | If a desktop webview refuses fullscreen, add a `Host` method for window fullscreen rather than a component branch. |
| Clipboard | The `paste` and `copy` events. A pasted Excel or Word range (TSV or HTML table) is parsed as data into ops and never mounted. | None. Event-based clipboard access needs no permission prompt in either host. |
| Keyboard | One shortcut map. Canvas claims browser-conflicting keys (for example Cmd/Ctrl+S) only while it has focus. | The web client must not break browser navigation keys outside Canvas. |
| Remote web (headless box) | The same routes, with upload and export sizes bounded by `[server]` settings and the audience guards (invariant 34). | Open with… is structurally absent. Everything else is identical. |

**Test matrix:** every U journey is recorded in the desktop app on macOS (WKWebView), and on Windows (WebView2) and Linux (WebKitGTK) where the shell ships, and in the web client on current Chrome, Firefox and Safari, plus at 390 px phone width. A pass on one host does not close a U item. Both client bundles are rebuilt and the freshness manifests verified for every UI change (`AGENTS.md`).

### The views

| # | View | What the person gets |
|---|---|---|
| U1 | **Document** | Pages and Flow modes, an outline rail of headings, a comment margin anchored to paragraphs, and Word's markup modes (*All markup*, *Simple*, *No markup*, *Original*). Tracked changes appear inline with insertions underlined, deletions struck and an author chip on each, so no change is signalled by colour alone. Headers, footers, footnotes, fields' last results and content controls are shown. |
| U2 | **Workbook** | Sheet tabs at the bottom and a virtualised grid that honours frozen panes, merges, column widths, styles, conditional formats and number formats. A formula bar shows the formula and the value Vak calculated, and marks cells whose functions Vak cannot evaluate. Comment indicators, data-validation dropdowns, charts drawn from the chart part, and a range selection with sum, count and average plus **Ask Agent**, **Comment** and **Chart this**. Defined names and tables can be jumped to. |
| U3 | **Deck** | A slide rail of thumbnails from the layout engine, a slide stage with theme, masters, preset shapes, charts and SmartArt drawings, a speaker notes panel, and **Present** mode (arrow keys, Escape). Comments are pinned to shapes, and slides can be reordered by dragging when the viewer can edit. |
| U4 | **Diagram** | Page tabs, shapes and connectors from master geometry, and shape data on click. With edit authority: move, add from the file's stencils, connect, and edit text and data. |
| U5 | **Structure** | The parts tree, relationships (external ones marked), content types, signature, encryption and label state, flagged items, and a read-only XML viewer for one part. This is the transparency path, disclosed on request and never the default. |
| U13 | **Automations** | For a macro-enabled file, the list of macros with their triggers, the Agent's plain explanation (labelled as the Agent's reading) and the static-analysis capability statement. **Run** shows that statement first and then streams the run into the open draft. The source view has syntax highlighting, line-anchored comments and diff. It also offers the run history with receipts, **Trust this macro** (digest-bound), and **Migrate to Vak automation**. Buttons in U2 and U3 that are bound to macros run through the same path. |

The Canvas header for an Office file shows the file name, a version label (`Version 2 · Draft · by Mira`), Pages or Flow, observed presence, a comment count, a **Changes** toggle, **Review**, **Download** (a native save dialog on desktop), **Export PDF**, and **Open with…** only where `host.can("open-with")` is true. Split and focused modes and the return-to-result link keep working as they do today (`66`). Office views use their own zoom (fit width, fit page, 100%) instead of the HTML device switcher.

### In the conversation

- **Result card (U6).** An Office deliverable is a quiet file result. It shows a type glyph, a title (from `docProps/core.xml` or the first heading, otherwise the file name), facts taken from the reader (`6 slides · 1,240 words`, `3 sheets · 412 rows`), a first-page, slide or sheet thumbnail from the layout engine, the draft state, and the actions **Open**, **Review changes**, **Ask for a change** and **Download**. It is a registry entry reusing the `File`/`Artifact` primitives, not a new card component.
- **Inbound files.** A file dropped onto the conversation, or received from a channel, is saved to the Agent workspace inbox and appears as the same card with a reader summary and any security flags. Its bytes never enter the prompt (F1). An encrypted file asks the person, not the model, for its password.
- **While the Agent is working (U7).** When the Agent applies ops to a draft that is open in Canvas, the view updates op by op. The Agent's mark sits briefly beside the anchor being edited, and one quiet line says what it is doing ("Mira is editing slide 4"). Scrolling does not follow the Agent unless the person turns *Follow* on. Motion respects reduced-motion settings. Pause and Stop go through the existing run-control bar.

### Review for Office candidates (U8, Screen 3)

- A **semantic change list** comes first, grouped the way a person thinks: "Slide 3: title changed", "Budget: 14 cells changed, column F added, 6 values recalculated", "§2.3 rewritten". Each entry opens the view at that anchor with a before and after: inline redline for documents, changed cells with old value, new value, author and recalculation effects, and a side-by-side or overlay comparison for slides and diagram pages.
- **Per-change decisions for every format.** Because every change is a sequenced op, a person can accept a subset. Vak replays the chosen ops (with their dependencies) onto the base and yields a derived candidate, which the existing exact-scope acceptance then promotes.
- The **checks panel** lists each check separately: package and schema conformance, op postconditions, calculation (N cells recalculated, M not evaluable), rendered checks (page or slide count, overflowed text, blank pages), signature impact, and label impact on delivery. A structural pass is never labelled "looks right".
- A **conflict card** states both sides at one anchor, for example "You set B4 to 120 · Mira set B4 to `=SUM(B1:B3)`". The person picks one or keeps both as a comment. There is no silent winner.

### Editing and collaboration in the UI (U9)

- **Edit mode is explicit** (a toggle or the floating bar), never click-to-type by accident. Editing grows by phase: text, cells and formulas, and slide text first; then tables, styles, images, charts and diagram shapes. Each committed edit becomes `OfficeOp`s, is attributed, and can be undone.
- **Live drafts.** People and Agents edit one draft at the same time. The server sequences ops, ops on different anchors merge automatically, and a stale anchor revision returns a conflict to the later author (P6).
- The **owner's own edits** can be saved to the file directly with **Save to file**, which uses the existing atomic promotion and undo receipt. **Participant edits** (a new `edit_draft` capability alongside `read`/`comment`/`message` in `69`) always stay in the draft until the owner accepts them.
- **Shared participants** see the same views with only the controls their grant allows. Absent authority means the control is absent, not a button that fails after a click.

### Open with another app (U10)

This is optional and not needed. There are two routes to the same result:

- **Desktop** (`host.can("open-with")`): the file opens in whatever application the OS has registered for it. When it is saved back, the workspace file's digest changes.
- **Either host:** **Download** writes a copy. Vak stamps that copy with a lineage custom property (`vak:lineage`, recorded in the download receipt, and preserved by conforming applications). When the file is later dropped back in, Vak recognises which version it came from.

Both routes lead to the same external-change card, for example "`report.docx` changed outside Vak: 3 paragraphs changed, 1 comment added", with **Review** and **Tell Mira**, and the same three-way merge. No Vak feature depends on this path, and it is never offered as the answer to something Vak cannot do.

### Library and knowledge (U11, with P8)

A quiet Files view lists the workspace's Office files with thumbnails, titles, author metadata, label and flag indicators, and when each last changed. Search results are anchors, not just files. Choosing one opens Canvas at the paragraph, cell, slide or shape with a brief highlight. Citations in Agent answers open the same way.

### Narrow screens, keyboard and accessibility (U12)

- **Phone:** Canvas is a full-screen sheet. Documents default to Flow. The workbook grid keeps headers frozen with a zoom control. Decks show one slide with swipe, and the rail becomes a bottom strip. A long press opens Comment / Ask Agent. On phone Review, the change list comes first.
- **Keyboard:** arrows and Tab move through the grid; Enter or F2 edits a cell (with edit authority); Page Up and Page Down move between slides; a single shortcut adds a comment; Escape leaves Present, Edit and focus in that order.
- **Screen readers:** the grid is an ARIA grid that announces headers and values plus formula or comment presence. Slides read in shape order with alt text, and missing alt text is flagged. Redline is announced as insertion or deletion by author.
- **Performance budgets:** a 50-page document's first page in under one second; a 100,000-row sheet scrolls through range-paged projection without jank; an 80-slide deck loads thumbnails lazily; PDF export of a 50-page document in under five seconds. A budget miss is a ledger item.

## Rule changes (land with P0)

The owner authorised rule changes for this work. The following edits to `AGENTS.md` land in the same change as P0, so each rule arrives together with its enforcement and no invariant describes something the tree does not yet do.

Each amendment lands with the phase that enforces it, not all at once: an invariant must never describe something the tree does not do.

1. **Amend invariant 14** (P0) to name the document parsers. `doc_read` and every target verifier execute in the broker worker; `data_query`, `office_apply` and Office projection join them as each gains a file source or exists. A parser of untrusted file formats never runs in the server, desktop or gateway process.
2. **Amend invariant 38** (P1) so that `doc_read`'s format list includes the Open XML family, subject to invariant 39.
3. **Add invariant 39 — Office documents are lossless, hostile, anchored and self-sufficient** (P0, growing with each phase), stating O1–O10 in `AGENTS.md` form as each becomes enforceable. It includes the self-sufficiency clause (no external office application is a runtime dependency, and external tools are CI oracles only) and the macro clause: until Vak's runtime exists no macro is ever executed, and once it exists VBA runs only there, inside the worker.
4. **Amend invariant 12** (with Agile decryption) so that a document password is a recipient-scoped secret, held in memory for one operation, never model-visible, and never in the ledger.
5. **Amend invariant 32's irreversible-work sentence** (with VBA authoring, P9). A VBA source change is not irreversible in itself, because it lands in a draft; what it grants is future code execution. The sentence therefore names VBA source changes, new auto-run entry points and macro-enabling conversions as work that always reaches a human, whatever was delegated, alongside irreversible work.
6. **Amend the Layout map** with `crates/vak-ooxml`, `crates/vak-ooxml-calc`, `crates/vak-ooxml-layout` and `crates/vak-vba` as each exists (the map is path-checked).

## Implementation ledger

`[x]` means code exists in the tree **and** its stated evidence was observed. A passing build, a unit test on a synthetic fixture, or a model's claim does not close a journey item.

### Support matrix

This is the tracking instrument for "full support". Each cell moves from `—` to **Preserve** (lossless round trip proven by the corpus), **Read** (projected with anchors), **Edit** (ops with postconditions) and **Create** (from blank or template). Preserve must be complete for every feature before any Edit cell is marked, because an edit that damages something else is not support.

| Feature | Word | Excel | PowerPoint | Visio |
|---|---|---|---|---|
| Package, properties, custom XML, thumbnails | — | — | — | — |
| Text, paragraphs, runs, styles, lists | — | — | — | — |
| Cells, formulas, number formats, names | n/a | — | n/a | n/a |
| Tables | — | — | — | n/a |
| Sections, headers, footers, footnotes | — | n/a | n/a | n/a |
| Slides, layouts, masters, notes | n/a | n/a | — | n/a |
| Pages, shapes, masters, connectors, shape data | n/a | n/a | n/a | — |
| DrawingML shapes, pictures, themes | — | — | — | — |
| Charts | — | — | — | n/a |
| SmartArt | — | n/a | — | n/a |
| Comments and threaded comments | — | — | — | — |
| Tracked changes | — | n/a | n/a | n/a |
| Content controls, fields (non-executing) | — | n/a | n/a | n/a |
| Validation, conditional formats, merges | n/a | — | n/a | n/a |
| Protection | — | — | — | — |
| Macros: preserve, read, analyse | — | — | — | — |
| Macros: author (reviewed) | — | — | — | — |
| Macros: run in Vak runtime (see coverage in P9) | — | — | — | — |
| UserForms and form controls | — | — | — | n/a |
| Signatures (verify; invalidation announced) | — | — | — | — |
| Agile encryption | — | — | — | — |
| Strict conformance | — | — | — | n/a |

### P0 — Foundation, security and rules

- [ ] Stop inlining binary document attachments (F1). A document that is valid UTF-8 text keeps today's inline path. Anything else is saved to `inbox/` in the Agent workspace (so every file tool can reach it under invariant 10), under a sanitised, digest-prefixed name that can never traverse or overwrite, and the prompt carries only a note naming the saved path and the reader to use. Until P1 lands, the note says plainly that the file was received and cannot yet be read. One cap governs both the bridge and the gateway.
- [ ] Move `doc_read` into `vak-tools` and serve it from the broker worker (F3); remove the in-process registration in the same change.
- [ ] Add a verification request kind to the worker protocol and run every target verifier through it under a read-only sandbox with a deadline (F4, F11). Worker unavailable or timed out means every planned check fails, never passes.
- [ ] Create `crates/vak-ooxml` L0: bounded reader (O2), relationship graph, main-part resolution through `officeDocument`, content-type validation, and detection of macro-enabled, Strict, signed, encrypted and labelled packages.
- [ ] L0 raw-copy writer with deterministic entry order. No-op round trip byte-identical across the corpus.
- [ ] Replace the verifier's hand-rolled logic with `vak-ooxml` (F5). Keep `format.openxml` as the verifier id and cover the whole family.
- [ ] Fuzz targets for L0 and L1. `cargo-fuzz` needs a nightly toolchain and CI is stable-only, so fuzzing runs as its own nightly job; a deterministic mutation test in the normal suite is a smoke check, not a substitute.
- [ ] Test corpus under `crates/vak-ooxml/tests/corpus/` with a provenance manifest. P0 lands with generated fixtures and the adversarial set; files authored in the real applications are added as they are made and are tracked by the manifest, and Preserve cells in the matrix wait for them. Only self-authored files: Word, Excel, PowerPoint and Visio (macOS and Windows), LibreOffice, a Google Workspace export, and python-docx/openpyxl/python-pptx output, in both Strict and Transitional. An adversarial set: ZIP bombs, traversal names, `DOCTYPE` entities, external relationships and remote templates, DDE fields, a macro package renamed to `.docx`, mismatched content types, duplicate entries, deep nesting, and malformed encryption headers.
- [ ] CI oracles (developer tooling only, O7): Open XML SDK validation and a LibreOffice round-trip open over the corpus and over every op test's output, in a CI container. They never ship and are never shown to users.
- [ ] Apply the P0 rule changes above to `AGENTS.md` (1, 3 as far as enforced, 6).
- [ ] Pin `zip` and `quick-xml` exactly (F12).

### P1 — Read (knowledge)

- [ ] L2 read models with anchors for Word, Excel, PowerPoint and Visio, including charts (series data), SmartArt text, properties and custom XML. Strict is read through namespace mapping.
- [ ] `doc_read` serves the family through the existing views, with O6 labels. Comments and tracked changes are shown as such, never merged silently into the body.
- [ ] `data_query` over worksheet ranges and tables (F10).
- [ ] VBA understanding: CFB reader, MS-OVBA decompression, modules, references, UserForm structure, entry-point map, and static analysis flags. Signature verification. Agile decryption with the O8 password flow. Label reading.
- [ ] Channel and drop-in intake: saved to the inbox, summarised through the reader, path and digest recorded.
- [ ] Schema-conformance validator from the published schemas plus the semantic rules. Confirm the schemas' redistribution terms before vendoring them.
- [ ] Support matrix: Preserve and Read columns filled with evidence.
- [ ] Journey evidence: a real Agent conversation answers a question over a folder of mixed Office files and cites `file#anchor` values that resolve to the quoted text.

### P2 — Edit and create

- [ ] L1 splice editor with an unknown-markup preservation test for every op (O1).
- [ ] `OfficeOp` schema pinned per op family; `office_apply` brokered and permission-gated, with repairable validation errors that small local models can act on.
- [ ] Built-in blank packages for every vocabulary. Creation from workspace templates copies theme, styles, layouts and stencils.
- [ ] Word tracked-change authoring under the frozen Agent identity; accept and reject ops.
- [ ] PowerPoint slides from template layouts and placeholders; charts from data; Visio shapes from stencil masters and connectors.
- [ ] O8 protection enforcement and the O10 "never" refusals, each with a test that the refusal happens before any byte is written.
- [ ] `prepare_to_share` op.
- [ ] Per-op postconditions plus schema validation recorded as `Observed` evidence (O5).
- [ ] Support matrix: Edit and Create cells filled with evidence.
- [ ] Journey evidence: an Agent creates a six-slide deck from a brief and the workspace's template, freezes it as a candidate, and passes Vak's validator and rendered checks. The CI oracles also pass on that file. A manual PowerPoint open is recorded as developer evidence and is not a runtime check.

### P3 — Calculation

- [ ] Decide D2 (below). Formula parser, dependency graph and evaluator in `crates/vak-ooxml-calc`, run in the worker, with a function coverage list published in this document.
- [ ] Recalculation after ops. Cached values written back through the splice editor (O1). Cells using unsupported functions, external references or volatile network functions are marked "not evaluable", never presented as current.
- [ ] A differential test against cached values in real Excel-authored corpus files.
- [ ] Journey evidence: a person changes an input cell in the running UI and sees dependent cells recalculate. The file saved from Vak shows the same values when opened by the CI oracle.

### P4 — Rendering and export

- [ ] `crates/vak-ooxml-layout`: Word pagination (sections, headers and footers, tables, floating images, footnotes), PowerPoint slides (masters, preset geometry, text autofit, charts, SmartArt drawings), Visio pages, and Excel print areas for export.
- [ ] Bundled metric-compatible fonts under open licences (for example Carlito, Caladea and Liberation), with licence files shipped. Text shaping and line breaking in pure Rust.
- [ ] The client draws the display list with Vak's own code. Thumbnails come from the same path.
- [ ] PDF and PNG export from the display list; the PDF output is re-verified by the existing PDF verifier.
- [ ] A visual verifier: page and slide counts, overflowed text, off-slide content, blank pages and missing fonts.
- [ ] Journey evidence: the same deck renders identically in the running web UI, the desktop UI and headless PDF export, with the rendered checks recorded.

### P5 — Review

- [ ] L4 semantic diff for all four vocabularies, including recalculation effects.
- [ ] Review shows the semantic diff, supports per-change acceptance by op replay, and lists signature and label impact (F7).
- [ ] Journey evidence: a person reviews a real Agent-edited workbook in the running UI, accepts some changes and rejects others, and the accepted file matches the reviewed derived digest.

### P6 — Collaboration

- [ ] Office anchor variant for candidate comments (F9). Comments stay attached to the version they addressed.
- [ ] Live drafts: a server-sequenced op log per draft, per-anchor compare-and-set, automatic merging of ops on different anchors, conflict cards for the same anchor, and op streaming to every observed participant.
- [ ] Human edits emitted as `OfficeOp`s under the principal's grant (`69`), with the new `edit_draft` capability.
- [ ] External-change detection by digest, with a semantic diff and three-way merge against the last known version.
- [ ] Export of Vak comments as native comments in the file, so that people without Vak see them.
- [ ] Agent-to-Agent handoff through versions (the existing isolated-revision path).
- [ ] Journey evidence: two people and an Agent edit one workbook live. Changes on different cells merge, a deliberate same-cell edit produces a conflict card, and Review shows all three authors.

### P7 — Headless

- [ ] `vak office read|apply|diff|verify|render` CLI verbs over the same crates and op schema, with JSON in and out and no logic of their own (invariant 19).
- [ ] HTTP routes for projection, display list, diff and export, under the same audience guards as the other candidate reads.
- [ ] Channel round trip: an inbound workbook is updated and returned under the bound bot identity (invariant 24), subject to label narrowing.
- [ ] Scheduled jobs: an `AgentSchedule` that updates a named workbook produces a candidate, or auto-accepts only inside an explicit envelope, with receipts in `agents_runs.jsonl`.
- [ ] Journey evidence: a real Telegram round trip and one real scheduled run, each with receipts, on a machine with no office application installed.

### P8 — Knowledge corpus

- [ ] An incremental workspace Office index keyed by digest, with anchors, rebuildable from the files and never authoritative over them.
- [ ] Search and recall return `file#anchor` citations, and the answer path shows them.
- [ ] Journey evidence: a folder of at least 200 mixed real files is indexed, a changed file is reindexed alone, and a cited anchor opens the right place in Canvas.

### P9 — Macros and automation

- [ ] CFB writer and MS-OVBA compression for VBA project writing, with forced recompilation so Office rebuilds its cache from source. Round-trip tests: an unedited project stays byte-identical, and an edited project opens and compiles in the CI oracle.
- [ ] VBA authoring ops (`vba_set_module`, `vba_add_module`, `vba_remove_module`) with mandatory human source review and separate confirmation for auto-run entry points and macro-enabling conversion. Tests prove that no envelope, schedule or auto-accept path can skip either.
- [ ] `crates/vak-vba`: lexer and parser for the full MS-VBAL grammar (parse coverage over the whole corpus comes before execution work), then an interpreter with budgets and cancellation.
- [ ] Emulated Excel object model mapped to `OfficeOp`s, first; then Word, PowerPoint and Visio. Library emulations as listed under "Architecture". Every other `CreateObject`, `Declare` and `Shell` stops the run with its line.
- [ ] Prompt and form bridge: `MsgBox`, `InputBox` and UserForms rendered by one shared component in both clients, and answered in the run receipt. Headless runs fail closed at the first prompt unless the answer was supplied up front.
- [ ] Pre-run capability statement from static analysis. Runtime enforcement of the granted capabilities. Digest-bound trusted macros with automatic invalidation.
- [ ] `office_run_macro` tool, run receipts in the ledger, form-control buttons in U2 and U3.
- [ ] Coverage report: language features, object-model members and the corpus completion rate, published in this document and updated with each change.
- [ ] Migration: an Agent turns a macro into a Vak automation, with a parity check of both op lists on the same input.
- [ ] Macro corpus: self-authored workbooks and documents with representative automation (report builders, cleanup loops, event handlers, UserForms), plus adversarial macros (auto-run downloaders, `Shell` and `Declare` abuse, obfuscated strings, XLM sheets). Every adversarial case must be flagged by analysis and refused by the runtime.
- [ ] Journey evidence, in both the desktop app and the web client on a machine without Office: a person opens a real month-end workbook, reads the Agent's explanation of its macro, runs it through its button, answers its `InputBox`, reviews the resulting cell changes and accepts them. An Agent then fixes a bug in that macro, the source diff is reviewed and accepted, and the macro is migrated to a scheduled Vak automation whose parity check passes.

### U — Office UI and Canvas (runs alongside P1–P7)

Each U item closes only with screenshots or recorded browser evidence of the **running** UI against real files (the `70` rule). A renderer fixture proves coverage, not a journey.

- [ ] Visual references for U1–U4, U6 and U8, agreed with the owner before building, stored beside the four `70` screens and following the same honesty rules.
- [ ] `OfficeProjection` and display-list schemas (paged: document block windows, sheet ranges, lazy slides) and the authenticated projection, display-list and part routes, shared unchanged by the desktop shell and the web client.
- [ ] Host port changes: `saveText` replaced by `saveFile(name, bytes, mime)` with its call sites migrated; the `open-with` feature with a scoped desktop opener capability; `dragDropEnabled: false` with one HTML5 intake and upload route; the bundled-font asset route.
- [ ] U5 Structure view replaces the Code fallback for Office files (F6), with the flag banner.
- [ ] U1 Document view (Flow in P1, Pages with P4), outline, comment margin and markup modes.
- [ ] U2 Workbook view: virtualised grid, frozen panes, formats, formula bar, calculated values (P3), charts.
- [ ] U3 Deck view: rail, stage (P4 geometry), notes, Present mode.
- [ ] U4 Diagram view, read in P1 and editable in P2.
- [ ] Point-then-act selection bar and anchor chips in all four views.
- [ ] U6 Office result card and inbound-file card, including the password prompt that bypasses the model.
- [ ] U7 live op streaming into an open draft, with the Agent-at-anchor marker and opt-in Follow.
- [ ] U8 semantic Review with per-change acceptance, checks panel and conflict card.
- [ ] U9 explicit edit mode, live drafts, owner Save to file with undo, participant `edit_draft`.
- [ ] U10 optional **Open with…** (desktop, `open-with`), download lineage stamping, and the external-change card, which is the same on both hosts.
- [ ] U11 Files library and anchor deep links.
- [ ] U13 Automations panel: macro list, triggers, explanation, capability statement, Run with live draft, source view with comments and diff, run history, trust, migrate.
- [ ] U12 phone layouts, keyboard map and screen-reader pass for U1–U4 and U8, recorded at 390 px and with a screen reader.
- [ ] Cross-host matrix recorded for U1–U4, U8 and U9 (desktop webviews and browsers listed under "Desktop and web: one client").
- [ ] Journey evidence: in **both** the desktop app and the web client, on a machine without Office or LibreOffice, a person opens a real deck, comments on slide 3, the Agent revises it in isolation while the draft updates live, Review shows the slide-level change, the person accepts, and exports a PDF.

## Decisions

Resolved by the 2026-09-23 owner direction:

| # | Decision | Resolution |
|---|---|---|
| R1 | Package layer | Build our own thin OPC and splice layer on `zip` + `quick-xml`. Model-based crates drop markup they do not model (breaks O1), and a second parser would break invariant 30. |
| R2 | Office or LibreOffice for rendering, recalculation, validation or conversion | **Never at runtime** (O7). Vak ships its own calculation, layout, export and validation. External tools are CI oracles only. |
| R3 | Scope | The full Open XML family in Strict and Transitional, including templates, macro-enabled variants and Visio. Legacy binary, `.xlsb`, ODF and IRM files are reported as unsupported, with **Open with…** when available. |
| R4 | Macros | **Supported and governed** (owner direction, 2026-09-23): understand, run in Vak's own sandboxed runtime, author under mandatory human review, and migrate (O10). XLM and ActiveX are preserved and never executed. |
| R5 | Co-editing | Live drafts with a server-sequenced op log and per-anchor compare-and-set. The workspace file changes only through acceptance or the owner's Save to file. |
| R6 | Per-change acceptance | For every format, by op replay. |
| R7 | Agent edits to existing Word documents | Native tracked changes under the Agent's identity. New documents are written clean. |
| R8 | Where documents are parsed | In the worker only. The browser never unzips a package. |
| R9 | Owner's in-app edits | Save to file directly through atomic promotion with undo. Participants go through acceptance. |

Still open (recommendation first):

| # | Decision | Recommendation | Why |
|---|---|---|---|
| D1 | Layout engine location | **One Rust engine (`vak-ooxml-layout`) producing a typed display list**, drawn by the client and printed to PDF headless. | One layout for UI, export and visual checks; no TypeScript second implementation to drift. |
| D2 | Formula engine | **Evaluate adopting IronCalc** (an open-source, pure-Rust spreadsheet engine) as the evaluator only, with our splice writer keeping ownership of the file. Build our own evaluator if its licence, function coverage or API does not fit. | Full Excel semantics are large. Reuse is sound if the library can evaluate without rewriting the package. Pin the exact version and justify it per the dependency rule. |
| D3 | PDF and text dependencies | Pure-Rust candidates (for example `krilla` for PDF, `rustybuzz` for shaping), pinned with one-line justifications when P4 starts. | Keeps O7 and avoids C toolchains in the worker. |
| D4 | Signature handling on edit | **Drop invalid signature parts and record it**, announced before acceptance. | A file should not claim a signature it no longer has. |
| D5 | Lineage stamp on downloaded copies | **Stamp `vak:lineage` in the downloaded copy only**, recorded in the receipt. The workspace file is never stamped silently. | Lets the web client, which has no Open with…, round-trip external edits into a proper three-way merge. The cost is that the downloaded copy's digest differs from the stored version, and the receipt records that. |
| D6 | VBA runtime: build or adopt | **Build `vak-vba` in Rust.** No embeddable, sandboxable VBA engine exists to adopt that fits O7, and the object-model emulation, which is the valuable part, is Vak-specific anyway. | Keeps execution inside the worker, with effects expressed only as ops. |
| D7 | VBA storage library | **Evaluate the `cfb` crate** for compound files, shared with Agile encryption, and pin it. Otherwise write a bounded reader and writer. | Both encryption and VBA need MS-CFB, so one implementation serves both. |
| D8 | Signing edited VBA projects | **Not now.** Vak drops the invalid VBA signature and says so. Re-signing with a user's certificate is a later decision. | Signing keys are high-value secrets and need their own design. |
| D9 | Cryptography for Agile encryption and XML-DSig | **Pinned RustCrypto crates** (AES, SHA-1/SHA-512, HMAC, RSA and ECDSA verification, X.509 parsing), each pinned exactly with a one-line justification, and a bounded exclusive XML C14N written in `vak-ooxml`. Decide before the P1 security slice. | Nothing in the tree covers this: `ring` is pinned for Ed25519 verification only and has no X.509 or AES-CBC. |

## Completion bar

This work is complete when the running desktop app and web client (one shared UI, verified across the host test matrix), and the headless CLI and channel paths, demonstrate these journeys **on a machine with no office application installed**, with real files, real Agents and runtime evidence: cited knowledge answers over a real folder; an Agent-created deck from a template, commented on, revised live and exported to PDF; a Word redline by an Agent reviewed and partly accepted; a spreadsheet co-edited live by two people and an Agent with correct recalculation and a surfaced conflict; a channel round trip; a Visio diagram read, edited and answered from; a real VBA automation understood, run in Vak's runtime, fixed under review and migrated; and an encrypted, signed, labelled file handled per the security model. The support matrix must be filled with evidence. Every produced file must pass Vak's validator and rendered checks, and the CI oracles must pass on it. Manual checks in native applications are developer evidence and are recorded as such. Record evidence in this ledger as each item lands, and state what was not checked.

Related: `54-task-environments-and-promotion.md` (candidates and verifiers; its "rendered document inspection" adapter is P4 here), `69-shared-conversation-coworking.md` (principals, grants and comments), `70-calm-agent-experience-implementation.md` (Canvas and Review; its "document/data result" workflow depends on P1, P2 and P5 for Office files), `24-agent-security.md` (threat model for the security section).
