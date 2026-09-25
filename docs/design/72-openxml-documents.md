# 72 — Office documents (Open XML): design and implementation ledger

Status: **proposal with an implementation ledger. P0 mostly landed and P1 started on branch `feat/openxml-p0`. Opened 2026-09-23; narrowed 2026-09-24 by owner direction to one loop (see "Owner direction").** Nothing under the phases is built until its box is checked with evidence. Vak has zero users, so nothing here carries a compatibility path (invariant 29): when a phase replaces an existing mechanism, the older one is removed in the same change (invariant 30).

History: the 2026-09-23 plan aimed at a self-sufficient Office suite (own layout engine and PDF export, formula engine, a VBA interpreter, live multi-person co-editing, Visio editing, encryption and signatures). A review on 2026-09-24 corrected it against the tree, a second review fixed twelve defects in the first implementation, and the owner then narrowed the scope to the loop below. Work outside the loop is listed under "Deferred until asked" with the reason, not deleted.

## Owner direction

**2026-09-24 — build the loop, not a suite.** Office support is one experience:

> **file in → the Agent reads and cites → the Agent proposes changes → a person reviews a redline → accepts → file out.**

Anything that does not serve that loop waits until someone asks for it.

- **Self-sufficient where it matters.** Reading, citing, editing, creating from a template, verifying and reviewing need no Microsoft Office, LibreOffice or other external application (O7). Page-accurate appearance is the one thing Vak does not reproduce: the structured views answer "what is in it and what changed", and **Open with…** or **Download** covers "what does page 4 look like" as a courtesy, never as the answer to a Vak feature.
- **Changes are always a redline, never a silent overwrite.** Every change lands in a draft, is attributed, and is reviewed before it reaches the file.
- **Macros are understood, never run.** VBA is read, explained and flagged. An Agent may rewrite what a macro does as a Vak automation, under review. Vak has no VBA runtime and does not plan one.
- **The headline creation feature is a deck, memo or workbook from the organisation's own template,** filled through its layouts and placeholders, never through free-positioned shapes.

2026-09-23 direction still in force: full Open XML reading in the most secure way, repository rules may change to allow it, and opening a file in another app is a courtesy, never a dependency.

## The experience

| Step | What the person sees | What makes it trustworthy |
|---|---|---|
| **File in** | Drop a file on the conversation, or send it on a channel. No import step. It appears as a quiet file card with a one-line summary and any security flags (macros, remote template, hidden text, sensitivity label). | The bytes never enter the prompt (F1). The file lands in `inbox/` in the Agent workspace and is parsed only in the worker. |
| **Read and cite** | The Agent answers and cites places: `Budget!B4`, `Slide 3`, a paragraph. Choosing a citation opens a structured view at that place. | Citations are anchors returned by the reader, bound to the file's digest. Hidden, deleted, white, off-slide and notes text is labelled, so a prompt injection hidden in the file is visible as hidden text. |
| **Propose changes** | "Update the Q3 numbers and add a summary slide." The Agent edits a draft. In Word the edits are real tracked changes under the Agent's name. | One tool (`office_apply`) with a small, typed op set; every op names an anchor, is checked after it runs, and fails with an error a small model can repair. |
| **Review** | A change list the way a person thinks: "Slide 3: title changed", "Budget: B4 100 → 120, 6 formulas will recalculate when opened", "§2 rewritten" as an inline redline. Accept all, or change by change. | The diff is computed from the op log and a re-read of the written package, never from the Agent's description. Checks are listed separately; a structural pass is never called "looks right". |
| **File out** | Accept writes the file (atomic, with undo). Download or **Open with…** hands it to any Office app; the Word redline survives there. A channel gets the updated file back with a short summary of what changed. | Unedited parts are copied byte for byte (O1). The written file is re-verified in the worker. |

### The views (one client for desktop and web)

The Office UI lives only in `crates/vak-client-ui`, built twice from one source (`48-web-client.md`). Desktop and web differ only through `host.can(...)`. Everything is drawn from the server's `OfficeProjection`; the browser never unzips a package and never mounts document HTML.

| # | View | What the person gets |
|---|---|---|
| U1 | **Document** | A reflowed reading view: outline rail of headings, paragraphs with their anchors, tables, comments in the margin, and tracked changes inline (insertions underlined, deletions struck, an author chip on each, never colour alone). |
| U2 | **Workbook** | Sheet tabs at the bottom and a virtualised grid of stored values and formulas; a formula bar shows the formula and its cached value, marked **stale** when an edit made it so. Hidden sheets and rows are shown and labelled. |
| U3 | **Deck** | A slide rail of titles, each slide as its shapes' text in reading order, speaker notes, and labels for hidden slides and off-slide shapes. |
| U4 | **Structure** | Parts, relationships (external ones marked, never followed), content types and flags. The transparency path, disclosed on request. |
| U5 | **Review** | The semantic change list, per-change accept, the checks panel, and conflict cards. |
| U6 | **File card** | In the conversation: type glyph, title, reader facts (`6 slides · 1,240 words`), draft state, and **Open**, **Review changes**, **Ask for a change**, **Download**, and **Open with…** where the host has it. A registry entry reusing the `File`/`Artifact` primitives, not a new card component. |

**Point, then act.** Selecting a cell range, paragraph or slide offers **Ask Agent** and **Comment**; either puts an anchor chip such as `About: Budget!B4:D20` into the composer. **Security is quiet but plain:** one banner line lists the flags, each opening Structure at its place.

## Why this exists

People and organisations hold large amounts of knowledge in Word, Excel, PowerPoint and Visio files, and most of the work done to them is small: read, find, cite, fix a table, update some cells, assemble a deck from a brief, review and hand back. Agents and people need a light, reliable way to do that work together. They do not need a suite.

## Scope

**Read** covers the whole ECMA-376 / ISO/IEC 29500 family in Transitional and Strict conformance, including templates, macro-enabled variants and add-ins: `.docx .docm .dotx .dotm`, `.xlsx .xlsm .xltx .xltm .xlam`, `.pptx .pptm .potx .potm .ppsx .ppsm .ppam`, and Visio `.vsdx .vsdm .vstx .vstm .vssx .vssm`.

**Edit and create** cover Word, Excel and PowerPoint through the op set in P2. Visio is read-only until asked.

**Unsupported, reported with a reason** (and **Open with…** where the host has a handler): legacy binary (`.doc .xls .ppt .vsd`), `.xlsb`, ODF, IRM/RMS, and, until the deferred decryption lands, Agile-encrypted packages. Vak never installs or calls an external converter.

## Engineering rules for this area

O1–O10 are the area invariants. Their enforced part is `AGENTS.md` invariant 39, which grows as each phase lands.

- **O1 — Lossless by default.** A part Vak did not edit is copied as its raw compressed ZIP entry. An edited part keeps every element, attribute, namespace and `mc:AlternateContent` branch Vak does not model: edits are splices into the XML event stream, never a re-serialisation from a partial model. Strict stays Strict.
- **O2 — Packages are hostile input.** Parsed only in the broker worker (invariant 14), under in-code bounds on file size, entries, per-part and total inflated bytes (counted as inflated, never as declared), compression ratio, XML depth and attributes. `DOCTYPE`, traversing, absolute or duplicate part names, and relationship targets that escape the package are refused. External targets are recorded, never followed. DDE, OLE, ActiveX, fields, XLM macro sheets, external data and Power Query are never executed or refreshed.
- **O3 — One operation vocabulary.** Agents, people in the UI, the CLI, HTTP and channels change a document only through the typed `OfficeOp` set, applied by one engine.
- **O4 — Anchors are exact.** Every op names an anchor Vak returned, bound to the base digest. A stale or missing anchor is a repairable error; the engine never guesses the nearest paragraph.
- **O5 — Evidence is observed.** After an op, the engine re-reads the written package and checks the op's postcondition. That check is `Observed` evidence (invariant 33); structural and content checks are reported separately.
- **O6 — Document content is data.** Text, comments, alt text, hidden runs, white or off-slide text, notes, properties and macro source are untrusted input, labelled for what they are.
- **O7 — Vak needs no Office.** No Microsoft Office, LibreOffice, .NET or other external application is a runtime dependency. External tools serve only as CI test oracles, never as user-facing checks.
- **O8 — Protection is honoured.** Locked cells, protected sheets and restricted editing limit ops unless the human owner removes protection with an explicit op that Review shows. Protection is never cracked.
- **O9 — Labels only narrow.** Sensitivity labels are read and shown and can only restrict egress (channels, invitations, webhooks), the meet-only rule of invariant 32.
- **O10 — Macros are understood, never run.** VBA and XLM are preserved byte for byte, read, explained and flagged. Vak has no macro runtime. An op never adds, edits or enables macros, never adds an auto-executing field, an external data connection, or a hyperlink with a `javascript:`, `file:` or UNC target. Rewriting a macro's behaviour as a Vak automation is ordinary Agent work under review.

## Review of the tree before P0 — 2026-09-23

Before P0, Vak could confirm that three Office formats were structurally ZIP packages (`OpenXmlPackageVerifier`) and map their MIME types. It could not read, edit, create or diff one.

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

Everything that touches package bytes lives in `crates/vak-ooxml`, a pure crate with no dependency on other vak crates, so it can be tested and fuzzed without a server, model or sandbox. Every call site runs it in the broker worker.

```
crates/vak-ooxml
  L0 package   bounded OPC reader, content types, relationship graph,
               detection by main-part content type, security inspection,
               raw-copy writer                                    (built)
  L1 xml       bounded event walk that refuses DOCTYPE and resolves
               mc:AlternateContent for reading                    (built);
               splice editor that preserves unknown markup        (P2)
  L2 read      anchored projections for Word, Excel, PowerPoint
               and Visio with O6 labels                           (built, P1 continues)
  L3 ops       typed OfficeOp set, one apply engine, per-op
               postconditions                                     (P2)
  L4 diff      semantic diff from the op log plus a re-read      (P3)
```

### Anchors


| Vocabulary | Anchor | Stability |
|---|---|---|
| Word | `w14:paraId` when present; otherwise Vak adds a paraId in the first op that touches the paragraph (which also declares the `w14` namespace and lists it in the root's `mc:Ignorable`). A read never adds one: reading must leave the package byte-identical (O1). Read-only anchors are a structural path plus a content hash valid for one digest. | A paraId usually survives editing in Word, but Word may reassign one (duplicates, some paste paths). A paraId that no longer resolves after an external edit is stale (O4), never matched to the nearest paragraph. A path-and-hash anchor never outlives its base. |
| Excel | `Sheet!A1`, `Sheet!A1:D20`, defined names, table names | **Not stable by construction:** inserting or deleting rows or columns moves every address below or to the right. The op engine rebases cell and range anchors through each structural op in the draft's op log. An external edit cannot be rebased, so anchors against the old digest become stale. Defined names and table names are the stable anchors. The sheet is resolved by name, with `sheetId` recorded so a rename is detected. |
| PowerPoint | slide `p:sldId@id`, shape `p:cNvPr@id` within the slide | Stable across reordering. A shape id is unique only within its slide, and copy and paste can renumber it, so a shape anchor always carries its slide id. |
| Visio | page ID, shape ID | Stable. |
| Charts and SmartArt | the owning anchor plus the chart or diagram part relationship | Stable while the owner exists. |

Each anchor carries a revision counter held in the draft's op log. An op states the revision it expects (compare-and-set), so an Agent's edit and a person's edit to the same draft never silently overwrite each other.


### Tool surface (one way each)

- **`doc_read`** (brokered, built) serves the family through the `text`, `summary`, `outline` and `table` views and the `section` parameter, with anchors, the file's digest, and labels for hidden content, comments, tracked changes and flags.
- **`office_apply`** (brokered, built) takes `path` (the workspace file the draft is for), an optional `source` (the file or template to start from; defaults to `path`), `base_digest` (the sha256 `doc_read` printed for the source) and ordered ops. It returns the new digest and per-op results, each confirmed by a re-read. **Creation is the same tool:** a template as `source` and a new `path`; a template becomes a document by changing only its main part's content type. It is a path-scoped write in the `PermissionEngine`, like `write` and `edit`. It writes a draft under `.vak/scratch/<agent>/<execution>/`, never the workspace file; that changes only when a person accepts the draft in Review, whole or in part.
- **`data_query`** (P1) gains worksheet ranges and tables as sources, and moves to the worker in the same change.

**P2 op set, v1.** Deliberately small, so small local models use it reliably:

| Vocabulary | Ops |
|---|---|
| Word | `replace_paragraph_text`, `insert_paragraph_after` (style by id or name), `delete_paragraph`; `set_table_cell`, `accept_changes` and `reject_changes` move to P3 with Review |
| Excel | `set_cells` (values or formulas over a range), `append_rows`, `add_sheet` |
| PowerPoint | `add_slide_from_layout`, `set_placeholder_text`, `set_notes`, `delete_slide`, `move_slide` |
| Shared | `set_title` |

Agent edits to an existing Word document are written as native tracked changes under the frozen Agent identity (`71-agent-character-system.md`); new documents are written clean. An edited formula is written with `fullCalcOnLoad` and its cached value is reported as **stale**, never as current. The op set grows only when a real request needs an op it lacks.

### How it joins the existing platform

- **Candidates** (`54`): every write happens in a draft, freezes as a candidate, is verified in the worker, goes through Review and is accepted atomically with undo. Office files gain no private path.
- **Review** (`70`): the Office change list replaces the blind binary comparison (F7).
- **Comments** (`69`): candidate comment anchors gain an Office anchor variant (F9).
- **Canvas**: the U1–U4 views replace the Code fallback (F6).
- **Channels**: inbound files are saved to the inbox (F1, done). Outbound deliverables go as document attachments with a change summary, subject to label narrowing (O9).

## Security model

Threat model context: `24-agent-security.md`. Office files arrive from strangers and are among the most abused document formats; every package is assumed hostile.

| Surface | Handling |
|---|---|
| Parsing | Only in the broker worker, with the O2 bounds. The worker carries the session's OS sandbox in restricted modes and none under FullAccess (F11), so the bounds hold on their own. Verification runs in a worker under a read-only, network-denied sandbox rooted at the tree being verified, whatever the session's mode, within a deadline, and fails closed. |
| Fuzzing | `cargo-fuzz` targets for the OPC reader, the XML walk, each read model, the splice editor and the op engine, run nightly with a growing corpus. A crash is a release blocker. |
| External relationships | Recorded and shown (linked images, remote templates, external workbook links, hyperlinks). Never fetched. A remote template is flagged as a known phishing vector. |
| Macros (VBA, XLM), ActiveX, fields, DDE, OLE, embedded packages, external data | Preserved byte for byte, shown and flagged, never executed, activated or refreshed (O10). |
| Signatures | Detected and reported as "not verified". An edit to a signed file is announced in Review, and the output drops the now-invalid signature parts and records that in the receipt, rather than shipping a file that claims a signature it no longer has. |
| Encryption | Agile-encrypted and IRM files are refused with a reason until decryption is asked for (deferred). |
| Sensitivity labels | Read and shown; narrow egress only (O9). |
| Protection | Honoured by ops (O8). |
| Prompt injection | O6 labelling in the tool result and the UI. |
| Privacy | A `prepare_to_share` op (P2) removes personal metadata, revision identifiers, hidden text and optionally comments; Review offers it before external delivery. |

## Macros

VBA macros hold real business logic, so they are made legible rather than treated as blobs, but they are never run. Running VBA safely would mean a language interpreter plus emulated Office object models, measured in years, and it would put untrusted code execution inside the product for little return.

- **Understand** (P1): every module, procedure and reference, what triggers each macro (a button, `Workbook_Open`, a worksheet event), static flags for risky calls (`Shell`, `Declare`, `CreateObject`, network, file writes, auto-run entry points), and a plain explanation from the Agent labelled as the Agent's reading (`Asserted`, invariant 33).
- **Rewrite as a Vak automation**: an Agent reads the macro and builds the same behaviour as a Vak automation (an `AgentSchedule`, a flow, or a skill using `office_apply` and `data_query`). It is reviewed like any other change. The original macro stays in the file unless a person asks.
- **Never:** execute, edit, add or enable VBA; add auto-run entry points. A person who needs to run a macro opens the file in Office (**Open with…**).

## Implementation ledger

`[x]` means code exists in the tree **and** its stated evidence was observed. A passing build, a unit test on a synthetic fixture, or a model's claim does not close a journey item.

### Progress — 2026-09-24, branch `feat/openxml-p0`

P0 is mostly landed and P1 is started. Everything checked below was observed in `cargo test` on macOS: `crates/vak-ooxml/tests/package.rs`, the `format.openxml` verifier test in `vak-sandbox`, `doc_read`'s Office tests in `vak-tools`, `crates/vak-server/tests/verification_worker.rs` and `doc_read_worker.rs` (both through the real `vak-tool-worker` binary, under the Seatbelt read-only verification sandbox), the gateway attachment tests, and the existing candidate export and promotion tests, which now verify through the worker. Not checked: Linux (Landlock verification sandbox), any file authored by a real Office application, and any live-model journey.

**Second review, same day.** The first implementation was reviewed line by line against real-file behaviour, and these defects were fixed, each with a regression test: percent-encoded part names (`a%20b.xml` in the ZIP) failed to resolve because targets were decoded and ZIP names were not; a scheme target (`mailto:`) without `TargetMode="External"` failed the whole read instead of being recorded as external; the Excel grid was a dense rows-by-columns matrix, a memory bomb for one cell at column XFD (grids now show at most 64 used columns and say how many are left out, while the anchored lines keep every cell); `mc:AlternateContent` was read in both `Choice` and `Fallback`, doubling every Word text box; field instructions were concatenated per paragraph, so a second DDE field was missed; formatting inside `w:rPrChange` (the properties before a tracked change) was applied as current; one huge paragraph or a large page could exceed the worker's 2 MiB protocol limit and surface as a bare exit code (units longer than 16,000 characters now continue on numbered part lines, pages stop at 1 MiB and say where to continue, and an oversized worker result is a readable error); `doc_read` output carried no digest, so anchors were not bound to a base (O4); parts without a content type were not detected (they are now flagged, and the verifier fails them); the archive comment was dropped on rewrite; the whole file was read into memory before any bound (the file size is now checked against the total bound first); and the synthetic fixtures compiled into shipped binaries (they are behind a `fixtures` feature that only test builds enable).

Verification for the branch: `cargo fmt --check`, `cargo clippy --workspace --exclude vak-desktop --all-targets -D warnings` and `cargo test --workspace --exclude vak-desktop` pass on macOS. `vak-desktop` was not built because this checkout has no `vak-client-ui/dist` bundle; no desktop code changed.

Decided 2026-09-24: the gateway saves a received file into the workspace inbox whatever the channel's permission mode, including read-only. It is the human sender's file arriving, not an Agent write; losing it would be worse, and the inbox never overwrites. The UI shows it under "Received files".

### P0 — Foundation, security and rules

- [x] Stop inlining binary document attachments (F1). A document that is valid UTF-8 text keeps today's inline path. Anything else is saved to `inbox/` in the Agent workspace (so every file tool can reach it under invariant 10), under a sanitised, digest-prefixed name that can never traverse or overwrite, and the prompt carries only a note naming the saved path and the reader to use. Until P1 lands, the note says plainly that the file was received and cannot yet be read. One cap governs both the bridge and the gateway. *Evidence: `gateway::tests` (inline text, large text saved, Office file saved and named with no package bytes in the prompt, other binary described honestly, traversal and NUL filenames stay in the inbox, a planted inbox symlink is refused, same bytes reuse one file, over-cap not received). The cap is 1 MiB because the gateway's JSON body limit is 2 MiB; larger Office files over channels need a raised body limit on `/gateway/inbound`.*
- [x] Move `doc_read` into `vak-tools` and serve it from the broker worker (F3); remove the in-process registration in the same change. *Evidence: `doc_read_worker.rs` reads a workbook through the real worker, and proves `Core` exposes exactly one `doc_read`, which fails with "tool broker unavailable" when the worker is missing instead of falling back in-process. Remaining gap outside this area: `vak-eval`'s runner still builds `default_tools()` in-process, so an eval case that calls `doc_read` parses in the eval process.*
- [x] Add a verification request kind to the worker protocol and run every target verifier through it under a read-only sandbox with a deadline (F4, F11). Worker unavailable or timed out means every planned check fails, never passes. *Evidence: `verification_worker.rs` (Office fixtures verified in the worker, a disguised macro package fails, a missing worker fails closed, a path outside the root fails); the existing candidate export, revision and promotion tests pass through the worker. Protocol version 2, `WorkerTask::VerifyTargets`, 120 s deadline. The timeout path is not exercised by a test.*
- [x] Create `crates/vak-ooxml` L0: bounded reader (O2), relationship graph, main-part resolution through `officeDocument`, content-type validation, and detection of macro-enabled, Strict, signed, encrypted and labelled packages. *Evidence: `tests/package.rs` over the four generated vocabularies and the adversarial set. Byte bounds count inflated bytes: a directory that claims 100 bytes for a 2 MiB part is refused with `PartTooLarge`.*
- [ ] Tell an encrypted Open XML package apart from a legacy binary file. Both are OLE compound files and are refused today with one message naming both; separating them needs the CFB reader (D7).
- [x] L0 raw-copy writer with deterministic entry order. *Evidence: a no-op rewrite of every generated fixture keeps each entry's name, CRC and raw compressed bytes in order; an edit changes only the edited entry, and the same edits produce identical output bytes.*
- [ ] No-op round trip byte-identical across application-authored corpus files (none exist yet).
- [x] Replace the verifier's hand-rolled logic with `vak-ooxml` (F5). Keep `format.openxml` as the verifier id and cover the whole family. *Evidence: `openxml_verifier_covers_the_family_and_refuses_disguises` (all four vocabularies pass; a macro-enabled package named `.docx`, a wrong main-part root and a non-package fail; `.doc` and `.xlsb` are not routed to it). Its evidence string says schema conformance and rendering were not checked.*
- [ ] Fuzz targets for L0 and L1. `cargo-fuzz` needs a nightly toolchain and CI is stable-only, so fuzzing runs as its own nightly job; a deterministic mutation test in the normal suite is a smoke check, not a substitute. *Progress: the smoke check exists (`mutated_packages_never_panic`, 1,200 mutated packages); no fuzz target yet.*
- [ ] Test corpus under `crates/vak-ooxml/tests/corpus/` with a provenance manifest. P0 lands with generated fixtures and the adversarial set; files authored in the real applications are added as they are made and are tracked by the manifest, and Preserve cells in the matrix wait for them. *Progress: `tests/corpus/MANIFEST.md`, `src/fixtures.rs` and the adversarial cases in `tests/package.rs` exist; no application-authored file yet.* Only self-authored files: Word, Excel, PowerPoint and Visio (macOS and Windows), LibreOffice, a Google Workspace export, and python-docx/openpyxl/python-pptx output, in both Strict and Transitional. An adversarial set: ZIP bombs, traversal names, `DOCTYPE` entities, external relationships and remote templates, DDE fields, a macro package renamed to `.docx`, mismatched content types, duplicate entries, deep nesting, and malformed encryption headers.
- [ ] CI oracles (developer tooling only, O7): Open XML SDK validation and a LibreOffice round-trip open over the corpus and over every op test's output, in a CI container. They never ship and are never shown to users.
- [x] Apply the P0 rule changes to `AGENTS.md` (invariants 14, 38, 39 and the Layout map).
- [x] Pin `zip` and `quick-xml` exactly (F12): `=4.6.1` and `=0.41.0`.

### P1 — Read (knowledge)

- [ ] L2 read models with anchors for Word, Excel, PowerPoint and Visio, including charts (series data), SmartArt text, properties and custom XML. Strict is read through namespace mapping. *Progress: Word body paragraphs with heading levels from built-in style names, `w14:paraId` or path anchors, tables, comments, tracked insertions and deletions, hidden and white runs, text boxes and risky fields; Excel sheets, shared and inline strings, booleans, errors, formulas with cached values, hidden sheets and rows, defined names; PowerPoint slides, titles, shapes, tables, notes, hidden slides and shapes, off-slide shapes; Visio pages and shape text; core title. Each projection lists what it does not read (headers and footers, footnotes, charts, SmartArt, number formats, pivot tables, PowerPoint comments, layout and master inheritance, Visio masters and shape data). Reading matches local names, so Strict parts should read, but no Strict document body is tested yet.*
- [x] `doc_read` serves the family through the existing views, with O6 labels. Comments and tracked changes are shown as such, never merged silently into the body. *Evidence: `doc_read` Office tests (text, section, outline, table with paging, summary with flags and external links, DOCTYPE refused, legacy and binary files refused with a reason). Every view states the content is data and lists what was not read.*
- [ ] `data_query` over worksheet ranges and tables (F10).
- [ ] Macro understanding (read-only, O10): CFB reader (D7), MS-OVBA decompression, modules, references, entry-point map (buttons, `Workbook_Open`, events) and static flags for risky calls. *Progress: the presence of a VBA project, XLM sheets and ActiveX parts is detected and flagged; sensitivity labels from custom properties are read.*
- [ ] Channel and drop-in intake: saved to the inbox, summarised through the reader, path and digest recorded. *Progress: channel files are saved to `inbox/` with a digest-prefixed name and the prompt names the reader (F1). Not done: a reader summary in the prompt, a recorded digest entry, and desktop or web drop-in.*
- [ ] Schema-conformance validator from the published schemas plus the semantic rules. Confirm the schemas' redistribution terms before vendoring them.
- [ ] Journey evidence: a real Agent conversation answers questions over a real document, workbook and deck and cites anchors that resolve to the quoted text.

### P2 — Edit and create

*Progress 2026-09-24:* `crates/vak-ooxml/src/splice.rs` (L1), `src/edit/` (L3: `mod.rs`, `word.rs`, `sheet.rs`, `deck.rs`) and `crates/vak-tools/src/office_apply.rs`. Evidence: `crates/vak-ooxml/tests/edit.rs` (13 tests), the `splice` unit tests, `office_apply`'s tests in `vak-tools`, `crates/vak-permission/tests/document_tools.rs`, and `office_apply_edits_through_the_worker_as_the_calling_agent` in `crates/vak-server/tests/doc_read_worker.rs`, which runs the real worker. Two defects found on the way were fixed: the broker never forwarded the Agent id to the worker, so inside the worker `bash` always used the `vak` scratch directory whichever Agent ran it; and `doc_read` was missing from the permission engine's read tools, so read-only mode denied it and workspace-write asked for approval.

- [ ] L1 splice editor: edits replace located element ranges in the event stream and keep everything else byte-identical; an unknown-markup preservation test for every op (O1). *Progress: built. Raw-copy identity of every untouched part is tested for Word, Excel and PowerPoint ops, and byte identity before the edited paragraph for Word. `markup_vak_does_not_model_survives_inside_edited_elements` puts unknown elements and attributes inside the edited elements for the three Word ops, `set_cells` (which `append_rows` uses) and `set_placeholder_text` (whose text replacement `set_notes` shares). Not yet covered by that test: `add_slide_from_layout`, `delete_slide`, `move_slide`, `add_sheet` and `set_title`.*
- [x] `OfficeOp` v1 schema and one apply engine with per-op postconditions checked by re-reading the written package (O5); stale or missing anchors are repairable errors (O4). *Evidence: `tests/edit.rs`; unknown fields are refused, every refusal names the op and the repair, and `base_digest` must match the source's sha256.*
- [x] `office_apply` writing to a draft that freezes as a candidate. *Evidence: `an_edit_is_a_draft_and_the_workspace_file_is_untouched` (the draft lands in `.vak/scratch/<agent>/<execution>/`, the workspace file is byte-identical, the Workbench receives `ExecutionStarted` and `ExecutionFinished` with the draft as its artifact), `drafts_chain_and_a_template_creates_a_new_file`, and the server test `an_office_draft_is_reviewed_by_meaning_and_accepted_through_promotion`.*
- [x] Word edits as native tracked changes under the runtime's Agent id (`w:ins`/`w:del` on runs and on the paragraph mark, new paragraphs with a fresh `w14:paraId`, `w14` declared when missing). *Evidence: `tests/edit.rs`, and the worker test showing the author is the calling Agent's id.*
- [x] Excel edits keep cell styles, set `fullCalcOnLoad`, drop the calculation chain when formulas change, and report cached values as stale. *Evidence: `tests/edit.rs`.*
- [x] PowerPoint slides from the template's layouts and placeholders, placeholder text, notes, move and delete (including section-list slide ids). *Evidence: `tests/edit.rs`.*
- [x] Creation from a workspace template: a `.potx`/`.dotx`/`.xltx` becomes the document named by `path`, theme, styles and layouts kept. *Evidence: `a_template_becomes_a_document_but_never_changes_macro_state`, `creates_a_deck_from_a_template_with_the_agent_as_author`.*
- [ ] Built-in blank packages for Word, Excel and PowerPoint.
- [x] O8 protection enforcement and the O10 refusals, each with a test that the refusal happens before any byte is written. *Evidence: `protection_is_honoured_before_any_byte_is_written` (Word read-only and comments-only protection refused, tracked-changes-only protection allowed because every op is a tracked change, PowerPoint password to modify refused), `excel_refusals` (protected sheet), the workbook-structure check in `add_sheet`, the macro-state refusals, and `office_apply`'s test that a refused op leaves the file byte-identical with no temporary file behind.*
- [ ] `prepare_to_share`.
- [ ] Journey evidence: an Agent creates a six-slide deck from a brief and the workspace's template, freezes it as a candidate, and passes Vak's verifier. The CI oracles pass on that file. A manual PowerPoint open is recorded as developer evidence, not a runtime check.

### P3 — Review

*Progress 2026-09-24:* `crates/vak-ooxml/src/diff.rs` and `src/review.rs`, the `OfficeReview` and `OfficeNarrow` worker tasks, `GET /sessions/{id}/sandbox/candidates/{cid}/office-review` and `POST …/office-narrow`, `office_apply` drafts, and in the client `src/officeRedline.ts`, `src/components/OfficeChangeList.tsx` and the Review wiring in `WorkbenchPanel.tsx`.

- [x] L4 semantic diff for Word, Excel and PowerPoint: paragraphs changed or redlined and grouped under their heading, cells old → new (a cached value turning stale is not counted as a change), slides added, removed, moved or retitled, and a new file summarised. It compares two re-reads rather than the op log, so it describes an external edit the same way. *Evidence: `crates/vak-ooxml/tests/diff.rs`; the endpoint runs it in a worker under the read-only sandbox (server test above).*
- [x] Per-change acceptance (R6) by op replay. `vak_ooxml::review` offers a draft's changes as choices (one per op; one per cell of a `set_cells` op), each with the changes it makes (attributed by replaying the ops one at a time and diffing each step) and the earlier choices it builds on (a paragraph or slide an earlier op minted, a sheet an earlier op added). The server assembles the draft's lineage from the session ledger: the `office_apply` call whose id is the candidate's execution, then each call whose `source` was an earlier draft, back to the file the chain started from and its `base_digest`. Narrowing replays only the kept ops against that file, remapping minted anchors (`p:<paraId>`, `slide:<id>`) to the ones the replay mints, so a kept op still lands where it was written; the result is a new candidate version (`narrowed`, parent = the full draft) that is verified, reviewed and accepted like any other, and the full draft stays. A draft the lineage does not reproduce (a command edited it, or the source changed) is offered only whole, and narrowing it is refused, so a hand edit is never silently dropped. Once one version of a result is accepted, the others stop being pending. *Evidence: `crates/vak-ooxml/tests/review.rs` (4, including the minted-id remap that a naive replay gets silently wrong), the server tests `an_office_draft_is_reviewed_by_meaning_narrowed_and_accepted_through_promotion` (a two-call chain, narrowed in the sandboxed worker, accepted, the left-out change absent) and `a_draft_changed_after_office_apply_is_offered_only_whole`, `tests/office-choices.mjs`, and the component harness checks in the browser at desktop and 375 px phone width. Not checked: the full Review modal against a live server and a live model. The replay found a P2 bug, now fixed: two slide-list ops in one call failed their postconditions, because an earlier op's slide order was checked against the final deck (`several_slide_list_ops_in_one_call_are_checked_against_the_order_they_leave`).*
- [ ] Review (U5) shows the change list first and lists checks and signature and label impact (F7). *Progress: for an Office file, Review shows the semantic change list in place of the raw before/after text columns, with tracked changes drawn as `<ins>`/`<del>` elements parsed from the reader's markers (never mounted HTML), the file's flags, and whether it is compared with the workspace or is new; with choices, each change has a checkbox and Accept waits until the kept changes are made into a version. "Open saved version in Canvas" is hidden for Office files until P4 gives Canvas a real view (F6). Signature and label impact (D4, O9): an edit to a signed file drops the package's signature origin, signature parts, their relationships and content types, and the `office_apply` result records it; Review states before acceptance what accepting does to signatures (removed, or a signature a changed draft still claims but no longer holds) and to sensitivity labels (kept, added, changed or removed, with removal and change marked as warnings). *Evidence: `an_edit_removes_the_signatures_it_invalidates_and_keeps_the_label` (tests/edit.rs), `accepting_states_what_happens_to_signatures_and_labels` (tests/diff.rs), `editing_a_signed_file_records_that_its_signature_was_removed` (office_apply), the harness in the browser. Signatures are still not verified (D9).* Checked live on 2026-09-24: a dev build served a disposable workspace (`/tmp/vak-live/workspace`, port 8931) with the owner's configured provider; the Agent, asked to raise two budget cells as a draft, called `doc_read` and `office_apply` (two `set_cells` ops), leaving the workspace file untouched; Review showed the two cell changes as choices; leaving one out made a narrowed version in the worker (format check passed); accepting it wrote only the kept cell, with untouched parts copied byte for byte; and undoing the acceptance restored the original exactly. The run found and fixed three client faults: the Review dialog squeezed the change list to a few pixels on narrow or short windows (it now scrolls as one page with header and actions pinned), and two places still treated a draft whose narrowed version was accepted as pending (one round-based rule, `src/candidateVersions.ts`, now decides for every place, with `tests/candidate-versions.mjs`). Not yet exercised live: Word and PowerPoint drafts, signed or labelled files, and anchored comments.*
- [x] Office anchor variant for candidate comments (F9). A comment on an Office file carries an `anchor` (`Budget!B4`, `'Q4 plan'!A1`, `p:1A2B3C4D`, `slide:256/shape:3`) instead of line numbers, which the server refuses for a package; the anchor's shape is checked (`vak_ooxml::is_anchor`), it is stored with the comment and listed, and a revision request names it ("file budget.xlsx, at Budget!B4"). In Review each change has a Comment action that points the comment form at its anchor. Diff anchors for sheets whose names need quotes now match the reader's (`'Q4 plan'!A1`, was `Q4 plan!A1`), so a comment or citation on such a cell resolves. *Evidence: `anchors_are_recognised_by_shape` (tests/package.rs), `a_cell_on_a_sheet_whose_name_needs_quotes_is_anchored_as_the_reader_anchors_it` (tests/diff.rs, fails without the fix), the anchored-comment steps of the server narrowing test, `a_revision_request_names_the_cell_or_lines_a_comment_points_at`, and the harness in the browser.*
- [ ] Journey evidence: a person reviews a real Agent-edited workbook in the running UI, accepts some changes and rejects others, and the accepted file matches the reviewed derived digest.

### P4 — Views and file card

- [ ] `OfficeProjection` schema (paged: document blocks, sheet ranges, slides) and authenticated projection routes shared unchanged by desktop and web. *Progress: `vak_ooxml::projection` pages the reader's own units under a byte budget with the outline (each entry names its first unit) and the Structure listing (parts, content types, sizes, relationships, external targets recorded); `GET /fs/office` and `GET /sessions/{id}/sandbox/candidates/{cid}/office` answer through the `OfficeProject` worker task. Evidence: `tests/projection.rs`, `the_canvas_reads_an_office_file_as_pages_and_structure_through_the_worker`, the web client live. Not checked in the desktop shell.*
- [ ] U1 Document, U2 Workbook, U3 Deck and U4 Structure replace the Code fallback (F6), with the flag banner and point-then-act anchor chips. *Progress: the Canvas opens a workspace file or a draft in its view: Document (headings, redline, labels), Workbook (sheet tabs, grid, formula bar with the stale-value marker), Deck (slide outline, one card per slide, a shape's paragraphs on their own lines — the reader separates them with `PARAGRAPH_BREAK`, " ¶ ", since a slash is ordinary text), and Structure; a selection offers "Ask Vak about this", which puts the anchor in the composer. Checked live on 2026-09-25 in the web client: a Workbook draft, a Word draft and a PowerPoint draft opened in their views. Not checked: the desktop shell, a file saved by Office, and comments drawn in the views.*

  Live runs with a small local model also showed what the loop must not leave to the model's reading of a result: it edited a `.docx` with the text `edit` tool (text tools now refuse an Office file before any approval is asked, `Tool::refusal`), named a paragraph by its text (a missed anchor now suggests the paragraphs whose text matches), wrote malformed slide ops (the op schema is a tagged `oneOf` whose errors name the branch), answered a delivered draft with HTML preview cards, and repeated the same `office_apply` call until three identical drafts existed. `Tool::delivered_file` now names the file a call delivers: the presentation check stands down for the run, an identical call gets the first call's result without a second draft or a second approval, and a card whose `artifact_path` previews that file is not shown. Evidence: `crates/vak-agent/tests/tool_refusal.rs`; checked live on 2026-09-25, where a repeated call was answered "Already drafted" and the turn ended on one sentence.
- [ ] U6 file card for deliverables and inbound files; desktop and web drop-in through one HTML5 intake and upload route into the inbox (`dragDropEnabled: false` in `crates/vak-desktop/tauri.conf.json`).
- [ ] Host port: `saveText` replaced by `saveFile(name, bytes, mime)` with its call sites migrated (invariant 30); `open-with` as a desktop-only `HostFeature` backed by a scoped opener limited to workspace files.
- [ ] Journey evidence, in both the desktop app and the web client on a machine without Office: a person drops a real deck, asks a question, follows a citation to the slide, asks for a change, reviews and accepts it, and downloads the file.

### P5 — Headless and channels

- [ ] `vak office read|apply|diff|verify` CLI verbs over the same crates and op schema, JSON in and out, no logic of their own (invariant 19).
- [ ] Channel round trip: an inbound workbook is updated and returned under the bound bot identity (invariant 24) with a change summary, subject to label narrowing. Raise the `/gateway/inbound` body limit so channel files larger than 1 MiB can arrive.
- [ ] Scheduled jobs: an `AgentSchedule` that updates a named workbook produces a candidate, or auto-accepts only inside an explicit envelope, with receipts in `agents_runs.jsonl`.
- [ ] Journey evidence: a real Telegram round trip and one real scheduled run, each with receipts, on a machine with no Office application installed.

### Deferred until asked

Each of these was in the 2026-09-23 plan. None is started; each needs a real request before it is.

| Item | Why deferred |
|---|---|
| Page-accurate rendering, thumbnails, Present mode and PDF export (`vak-ooxml-layout`, bundled fonts) | Months of work for appearance only. Structured views answer the loop's questions; **Open with…** and **Download** cover appearance. |
| Formula calculation engine (`vak-ooxml-calc`; IronCalc was the candidate) | `fullCalcOnLoad` plus honest "stale" labels are correct without it. Needed only if people must see recalculated values inside Vak. |
| Live multi-person co-editing (op sequencing, presence, conflict streaming) | Draft plus review already gives attributed, reversible collaboration. |
| Visio editing | Reading covers the knowledge use case; no editing request exists. |
| Agile decryption and XML-DSig signature verification | Needs its own cryptography dependencies (decision D9) and a password flow. Refused with a reason until asked. |
| A knowledge index across a folder of Office files | `doc_read` per file serves the loop. An index waits for a real "search my folder" need. |
| A VBA interpreter, macro runs, VBA editing, trusted macros | Cut, not deferred (O10). |

### Support matrix

Each cell moves from `—` to **Preserve** (lossless round trip proven by application-authored corpus files), **Read** (projected with anchors), **Edit** (ops with postconditions) and **Create**. Preserve must be complete for a feature before its Edit cell is marked. No cell is marked yet: read coverage exists on generated fixtures, but no application-authored corpus file exists.

| Feature | Word | Excel | PowerPoint | Visio |
|---|---|---|---|---|
| Package, properties, custom XML | — | — | — | — |
| Text, paragraphs, runs, styles, lists | — | — | — | — |
| Cells, formulas, names | n/a | — | n/a | n/a |
| Tables | — | — | — | n/a |
| Slides, layouts, notes | n/a | n/a | — | n/a |
| Pages, shapes | n/a | n/a | n/a | — (read only) |
| Comments | — | — | — | — |
| Tracked changes | — | n/a | n/a | n/a |
| Macros: preserve, read, flag | — | — | — | — |
| Strict conformance | — | — | — | n/a |

## Rule changes

Each amendment to `AGENTS.md` lands with the phase that enforces it; an invariant never describes something the tree does not do.

1. **Invariant 14** (done, P0): a parser of untrusted file formats never runs in the server, desktop or gateway process; `doc_read` is a worker tool and verifiers run in the worker's `VerifyTargets` task. `data_query` and `office_apply` join as each gains a file source or exists.
2. **Invariant 38** (done): `doc_read` reads the Open XML family, subject to invariant 39.
3. **Invariant 39** (done for P0 and P1, grows per phase): O1–O10 as enforced, including that Vak never executes macros and has no macro runtime.
4. **Layout map** (done): `crates/vak-ooxml`.

## Decisions

| # | Decision | Resolution |
|---|---|---|
| R1 | Package layer | Our own thin OPC and splice layer on `zip` + `quick-xml`, pinned exactly. Model-based crates drop markup they do not model (breaks O1). |
| R2 | External office applications | Never at runtime (O7). CI oracles only. |
| R3 | Scope | Read the whole family; edit and create Word, Excel and PowerPoint through a small op set; everything else deferred until asked (2026-09-24). |
| R4 | Macros | Understood, never run; rewritten as Vak automations under review (2026-09-24, replaces "run in Vak's runtime"). |
| R5 | Collaboration | Draft plus review; live co-editing deferred (2026-09-24). |
| R6 | Per-change acceptance | By op replay onto the base, for every editable format. |
| R7 | Agent edits to existing Word documents | Native tracked changes under the Agent's identity; new documents written clean. |
| R8 | Where documents are parsed | In the worker only. The browser never unzips a package. |
| R9 | Rendering | Structured views; page fidelity through **Open with…** / **Download** (2026-09-24). |
| R10 | Inbox writes on read-only channels | Allowed: a received file is the sender's, not an Agent write; the inbox never overwrites (2026-09-24). |
| D4 | Signature handling on edit | Drop invalid signature parts and record it, announced before acceptance. |
| D7 | CFB reader | Needed to tell encrypted packages from legacy binaries and to read VBA source for P1 understanding. Evaluate the `cfb` crate, pinned, else write a bounded reader. |
| D9 | Cryptography | Deferred with decryption and signature verification. When asked: pinned RustCrypto crates and a bounded exclusive C14N; `ring` covers Ed25519 only. |

## Completion bar

The loop is complete when, **on a machine with no Office application installed**, the running desktop app and web client (one shared UI) and the channel path demonstrate with real files, a real Agent and recorded evidence:

1. a cited answer over a real document, workbook and deck, whose citations open at the quoted place;
2. an Agent-created deck from the workspace's template, reviewed and accepted;
3. a Word redline by an Agent, partly accepted in Vak and then opened in Word with the remaining changes still tracked (developer evidence);
4. a workbook edited by an Agent with formulas marked stale, reviewed change by change;
5. a Telegram round trip returning the updated file with a change summary;
6. a macro-enabled workbook read, its macro explained and flagged, and its behaviour rewritten as a Vak automation under review.

Every produced file passes Vak's verifier and the CI oracles. Record what was not checked.

Related: `54-task-environments-and-promotion.md` (candidates and verifiers), `69-shared-conversation-coworking.md` (principals and comments), `70-calm-agent-experience-implementation.md` (Canvas and Review), `24-agent-security.md` (threat model).
