# 72 — Office documents (Open XML): design and implementation ledger

Status: **the core loop is shipped; everyday creation and extraction improvements reopened 2026-09-29 by owner direction and implementation is in progress.** The 2026-09-27 blank-file work is shipped. This stage adds everyday tables, charts, and images across Word, Excel, PowerPoint, and PDF, improves their structured Canvas views, and preserves the native file structure on download. See "Owner direction — everyday documents". Work is tracked below and is not complete until its local tests and review/download checks pass.

History: the 2026-09-23 plan aimed at a self-sufficient Office suite (own layout engine and PDF export, formula engine, a VBA interpreter, live multi-person co-editing, Visio editing, encryption and signatures). A review on 2026-09-24 corrected it against the tree, a second review fixed twelve defects in the first implementation, and the owner then narrowed the scope to the loop below. Work outside the loop is listed under "Deferred until asked" with the reason, not deleted.

## Owner direction

**2026-09-29 — everyday documents, structure, and extraction.** Build the
common daily tasks that were missing from the create/read loop: tables in
slides, simple charts, and images with descriptive text. Keep outputs in the
published Open XML formats (ECMA-376 / ISO/IEC 29500) and PDF (ISO 32000),
using no proprietary file format, service, or runtime dependency. Tables and
charts retain their rows, labels, and values as structured content. Images
retain their alt text, which is projected to `doc_read`; image pixels are
never presented as extracted text unless an OCR feature is separately added.
The Office/PDF reader must expose chart series, table cells, and image
descriptions as anchored data so search and RAG can consume them without
reverse-engineering screenshots. Generated and edited artifacts continue
through the same draft, Review, acceptance, and raw-byte download paths;
downloading an accepted file preserves its native package structure.
The Canvas is also in scope at phone and tablet widths: controls must stay
reachable, the page itself must not overflow horizontally, and workbook
horizontal scrolling stays inside the grid so cell columns remain usable.
Desktop and 390 × 844 browser checks are required before calling this stage
done.

**Progress — 2026-09-29.** Word and PowerPoint create native tables and
PNG/JPEG images with alternative text; Excel creates native filterable tables,
bar/line/pie charts and anchored images; PowerPoint charts are native Open XML
parts, and PDF charts are vector bars with searchable source rows. Readers
project table cells, chart categories and values, and image descriptions into
anchored units for `doc_read` and RAG. Excel table/chart/image changes appear
in Review. The real-worker integration test generates DOCX, XLSX, PPTX and PDF
samples and checks their projections and `doc_read` output. Separate server
tests exercise XLSX and PDF review, narrowing, promotion, reading and download.
The Excel Canvas now shows a scrollable blank-cell grid (with keyboard
selection), safe RGB font/fill colors and bold/italic from `styles.xml`, plus
native table ranges and row/column stripe hints. These are presentation-only
page metadata; `doc_read` still returns the same anchored values for search and
RAG. Worksheet drawings preserve their exact two-cell bounds or one-cell
pixel extents, so the Canvas sizes charts and images from the workbook instead
of assigning every object a fixed rectangle. The page projection also formats
common percentage, currency, and date values for the Canvas while retaining
stored cell values for `doc_read` and RAG, and returns custom/hidden row heights
and column widths as separate geometry. Verified against a real XLSX fixture:
the authenticated local worker endpoint returns `25.6%` and `1/1/24`, keeps
`0.256` and `45292` in its cell units, and reports custom widths/heights and
hidden dimensions separately. Unsupported number formats, conditional
formatting and exact table theme colors remain listed as not read.
The rendered workbook check found an overlay-origin bug even though placement
labels showed the correct cells. The Canvas now measures after render and
recalculates positions when the worksheet layout changes. It also exposed a
real collision: a chart at A4 covered an image at D2. An automatically placed
chart now checks every image cell in the request, including images added later,
and moves below nearby content using its native drawing extent; an explicitly
requested chart cell is kept. Review and narrowing carry that resolved anchor
through acceptance. The real dev app Canvas has been checked with generated
workbook and slide drafts; the server acceptance test confirms the accepted
XLSX keeps its chart clear of the D2 image and that the raw download equals the
accepted file bytes.
The workbook grid extends through the visible extent of lower anchored charts
and images, keeping overlays inside the sheet's scroll surface. The real Canvas
has also been checked at 1440 × 900 and 390 × 844: the page stays within the
phone width, horizontal spreadsheet navigation remains inside the grid, and
sheet tabs remain reachable.

**2026-09-29 — full-fidelity workbook preview reopened by owner request.** The
owner compared the Canvas with LibreOffice and asked why the spreadsheet view
does not render like the real workbook. The structured Canvas is an
accessibility and review projection: it shows a bounded HTML cell grid and
reconstructed chart cards, so it does not match spreadsheet layout or chart
rendering. The previous “Deferred until asked” decision no longer applies.
Plan and implement a page-faithful preview in Vakyartha while keeping the Open
XML package as the source of truth and preserving anchored cells, image alt
text, cached chart data and RAG extraction. Rendering must use Vakyartha's own
standards-based code. LibreOffice may be used only as a developer-side test
oracle; Vakyartha must never launch it or depend on it at runtime. Until the
native renderer lands, the Canvas is not an Office-equivalent visual preview.

**Progress — 2026-09-29.** Corrected the generic worksheet drawing
relationship serialization after comparing a real generated XLSX with
LibreOffice: drawing relationships now declare the relationship namespace at
the drawing reference, so Excel-compatible readers can resolve charts and
images even when tables are also present on the sheet. Added a combo-chart
reader fixture proving each series keeps its own type. Cached series and
categories remain structured table rows for extraction/RAG, and cached point
indices preserve category alignment when a middle point is missing. The Canvas
reconstructs common chart forms from those values, with a data table available
alongside the visual. This is still a structured preview, not full Office chart
fidelity. The live XLSX confirms its image at `Sheet1!D2` and chart at
`Sheet1!A10`. Result cards now use compact format icons and responsive actions;
390px viewport checks show no page overflow.

**Progress — 2026-09-29.** Spreadsheet drawing previews preserve OOXML
`from`/`to` marker offsets in pixels as visual metadata, and the Canvas keys
measured overlays by drawing identity. Two-cell image bounds account for both
marker offsets, and multiple drawing objects anchored to one cell no longer
overwrite each other's measured dimensions. Offsets stay out of searchable
unit text; image descriptions remain available to `doc_read` and RAG. Workbook
image IDs are scoped to their drawing part in the visual projection while
public citation/edit anchors stay unchanged, including when two sheets reuse
the same object ID and cell. Verified by drawing-reader and two-sheet
projection fixtures and the client TypeScript check. External workbook corpus
verification remains pending.

**Progress — 2026-09-29.** The workbook Canvas carries native merged-cell
ranges as visual-only sheet geometry, spans the source cells in the grid, and
normalizes keyboard navigation to the merged cell's top-left anchor. Arrow
navigation advances past the full merged range when moving out of it, so a
horizontal or vertical merge cannot trap focus. The metadata remains separate
from cell units and RAG text, is retained while pages load, and is scoped to
the visible worksheet. Reader and paged-projection fixtures cover merge
extraction, and `tests/office-cells.mjs` covers entry, exit and keyboard
movement; live browser verification remains pending.

**Placement contract.** The person names the desired place in the request;
the Agent writes the native anchor and shows it in Canvas and Review. In a
workbook, an image or chart has an exact top-left cell; a chart defaults to
the first source column immediately below its data and moves down if that
drawing area would cover another new image or chart. In a deck, tables, charts
and images go into a named layout placeholder; Canvas shows the slide,
placeholder and coordinates in inches. In Word, tables and images follow the
named paragraph or go at the end, and Canvas keeps them in document order.
PDF additions follow the named page or go at the end. A person can select a
workbook cell and ask the Agent to place an object there. Canvas does not yet
support dragging objects to reposition them directly. Downloaded Office files
keep their normal Open XML drawings and relationships; no proprietary format
or runtime is involved.

**Multiple objects and bounds.** Each add operation creates one Word table or
image, Excel table/chart/image, or PDF table/chart/image; PowerPoint can add
several objects to distinct placeholders on a slide and several slides in one
draft. There is no per-kind object count limit: OpenXML review offers
change-by-change choices for up to 100 edit operations per draft, and PDF
accepts up to 200. One Word table supports 1,000 rows by 63 columns; one Excel
table supports 10,000 data rows plus a header and 64 columns; one PowerPoint
table supports 100 rows by 32 columns. Excel charts support 2–1,000 data rows;
PowerPoint charts 1–100 categories; PDF charts 1–20. Each embedded image is at
most 1 MiB. The OpenXML Canvas now previews up to 8 MiB of images per file,
while all image descriptions remain available to extraction and RAG.

The native-package, reader/RAG, review and raw-download tests remain gates,
alongside that rendered check. Unsupported extraction remains listed in
`not_read` rather than being treated as absence.

**2026-09-27 — create from scratch.** A person can ask for a new Word document, workbook or deck without bringing a file or a template. The Agent drafts it with the same `office_apply`, starting from Vak's own blank, and the new file reaches the workspace only through Review, like any other change. The loop is therefore:

> **a file or a request → the Agent reads and cites, or starts from a blank → the Agent proposes changes → a person reviews → accepts → file out.**

The organisation's template stays the better start when the workspace has one; the blank is for when it does not. Visio drawings from scratch come after Word, Excel and PowerPoint, because Visio has no editing engine yet. This replaces the 2026-09-24 deferral of blank packages ("making new decks waits for a request"): this is the request.

**2026-09-24 — build the loop, not a suite.** Office support is one experience:

> **file in → the Agent reads and cites → the Agent proposes changes → a person reviews a redline → accepts → file out.**

Anything that does not serve that loop waits until someone asks for it.

- **Self-sufficient where it matters.** Reading, citing, editing, creating from a template, verifying and reviewing need no Microsoft Office, LibreOffice or other external application (O7). Native page-faithful rendering is in progress; **Open with…** or **Download** remains a courtesy and never a runtime requirement.
- **Changes are always a redline, never a silent overwrite.** Every change lands in a draft, is attributed, and is reviewed before it reaches the file.
- **Macros are understood, never run.** VBA is read, explained and flagged. An Agent may rewrite what a macro does as a Vak automation, under review. Vak has no VBA runtime and does not plan one.
- **The headline creation feature is a deck, memo or workbook from the organisation's own template,** filled through its layouts and placeholders, never through free-positioned shapes.

2026-09-23 direction still in force: full Open XML reading in the most secure way, repository rules may change to allow it, and opening a file in another app is a courtesy, never a dependency.

## The experience

| Step | What the person sees | What makes it trustworthy |
|---|---|---|
| **File in** | Drop a file on the conversation, or send it on a channel. No import step. It appears as a quiet file card with a one-line summary and any security flags (macros, remote template, hidden text, sensitivity label). | The bytes never enter the prompt (F1). The file lands in `inbox/` in the Agent workspace and is parsed only in the worker. |
| **Create** | "Write a one-page memo on the Q3 results", "make a budget workbook", "a six-slide deck on the launch". The Agent drafts the new file, which appears as a draft marked new, with **Review draft**. | The draft starts from Vak's built-in blank (or the workspace's template), never a file fetched from elsewhere. Every op is checked by a re-read like any edit, and nothing reaches the workspace until a person accepts it. |
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

**Edit and create** cover Word, Excel and PowerPoint through the op set in P2. A new file starts from a workspace template or from Vak's built-in blank ("Creating from scratch — 2026-09-27"). Visio is read-only until its creation step lands.

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
| F2 | Gap | `doc_read` (then vak-core's `doc_reader.rs`; it is now `crates/vak-tools/src/doc_read.rs`) used `read_to_string`, so it fails on every Office file. `read` returns `[binary file]`. An Agent's only route to the contents is ad-hoc scripting in `bash` with whatever libraries the sandbox happens to have. That is ungoverned, unreproducible and not anchored. |
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
- **`office_apply`** (brokered, built) takes `path` (the workspace file the draft is for), an optional `source` (the file or template to start from; defaults to `path`), `base_digest` (the sha256 `doc_read` printed for the source) and ordered ops. It returns the new digest and per-op results, each confirmed by a re-read. **Creation is the same tool:** a template as `source` and a new `path` (a template becomes a document by changing only its main part's content type), or a new `path` with no `source` and no `base_digest`, which starts from Vak's built-in blank for the path's format. It is a path-scoped write in the `PermissionEngine`, like `write` and `edit`. It writes a draft named `.vak/scratch/<agent>/<execution>/…` and kept in the space's execution root in the runtime directory (`vak_config::scope::draft_location`), never the workspace file; that changes only when a person accepts the draft in Review, whole or in part.
- **`data_query`** (P1) gains worksheet ranges and tables as sources, and moves to the worker in the same change.

**Tool guidance audit — 2026-09-30.** The shared `office_apply` description
stays below 2,000 bytes; format details live beside each operation. Its schema
includes executable creation examples for Word, Excel, PowerPoint and PDF.
It states the digest and draft chaining rules, atomic failure behaviour,
formula calculation limits, chart source shape and placement, table names
versus worksheet names, layout placeholders, base64 image inputs and PDF
writing limits. The chart schema requires a complete Excel or PDF variant
and rejects mixed fields. Tests execute every advertised example and read
its draft back (`every_advertised_creation_example_produces_a_readable_draft`).
Direct worker checks also found and corrected Excel chart lookup for quoted
sheet names, XML escaping of worksheet names in chart formulas, empty-string
cell postconditions, and chart cache labels split
by XML entities. Regression evidence:
`charts_accept_quoted_sheet_names_and_decimal_values` and
`empty_strings_can_be_written_and_clear_existing_excel_values`. These checks
verify file structure and extracted content; they do not claim every model
will plan correctly or that a particular Office application's rendering was
verified.

**P2 op set, v1.** Deliberately small, so small local models use it reliably:

| Vocabulary | Ops |
|---|---|
| Word | `replace_paragraph_text`, `add_paragraph` (style by id or name; after an anchor, or at the end), `add_table`, `delete_paragraph`; `set_table_cell`, `accept_changes` and `reject_changes` move to P3 with Review |
| Excel | `set_cells` (values or formulas over a range), `append_rows`, `add_sheet`, `rename_sheet`, `format_cells`, `set_column_widths` |
| PowerPoint | `add_slide_from_layout` (with its notes), `set_placeholder_text`, `set_notes`, `delete_slide`, `move_slide` |
| Shared | `set_title` |

`add_paragraph`, `add_table`, `rename_sheet`, `format_cells`, `set_column_widths` and slide notes were added on 2026-09-27 for creating from scratch; `add_paragraph` replaced `insert_paragraph_after`.

Agent edits to an existing Word document are written as native tracked changes under the frozen Agent identity (`71-agent-character-system.md`); new documents are written clean. A paragraph edit takes the paragraph's whole new text and marks only the words that differ, keeping every other run's formatting and everything the reader does not show (2026-09-26, below). An edited formula is written with `fullCalcOnLoad` and its cached value is reported as **stale**, never as current. The op set grows only when a real request needs an op it lacks.

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
- **Rewrite as a Vak automation**: an Agent reads the macro and builds the same behaviour as a Vak automation (a scheduled task, a flow, or a skill using `office_apply` and `data_query`). It is reviewed like any other change. The original macro stays in the file unless a person asks.
- **Never:** execute, edit, add or enable VBA; add auto-run entry points. A person who needs to run a macro opens the file in Office (**Open with…**).

## Implementation ledger

`[x]` means code exists in the tree **and** its stated evidence was observed. A passing build, a unit test on a synthetic fixture, or a model's claim does not close a journey item.

### Creating from scratch — 2026-09-27

Opened by the owner direction of the same date. Before it, `office_apply` could make a new file only from a template already in the workspace: asked for "a Word document about X" in an empty workspace, the Agent had no governed way to do it.

**One way to create.** `office_apply` with a `path` that is not in the workspace and no `source` starts the draft from Vak's built-in blank for that path's format. There is nothing to have read, so `base_digest` is left out; a call that gives one for a file that does not exist is refused, because it shows the model believed it was editing a file (usually a mistyped name). Creation never replaces a file: a name that exists is edited (read, then `base_digest`) or a new name is chosen. A macro-enabled name (`.docm`, `.xlsm`, `.pptm` and the add-ins) is refused (O10); a template name (`.dotx`, `.xltx`, `.potx`) creates a template. The draft is a new document, written clean (R7). Review replays its ops from the same blank: a draft's lineage starts either at a workspace file with its digest or at `blank`, never at a file the replay cannot reproduce, so a from-scratch draft offers change-by-change choices like any other. `vak office apply` takes the same shape (no source file and no digest: from the blank).

**Preview before Review.** An `office_apply` artifact is visible as soon as its execution reports it, before a candidate has been frozen. The Files panel and Canvas resolve that artifact through its owning session and execution, checking the reported path against that execution's scratch root inside the session workspace. Once Review freezes a candidate, its versioned candidate route serves the immutable bytes. A workspace-wide `/fs` lookup cannot establish ownership of an unaccepted Agent draft and is not the preview route for one.

**The blanks** (`vak_ooxml::blank`) are written for Vak, not copied from an application, and are byte-identical for a given version. Each passes the `format.openxml` verifier and reads back with no content. They are deliberately plain; an organisation's look comes from its own template.
- Word: A4 with 2.54 cm margins; Aptos body text and Aptos Display headings through the theme; the styles Normal, Title, Subtitle, Heading 1 to 3, List Bullet, List Number, Quote and the Table Grid table style, with bullet and number definitions; an empty body, as Word itself saves an empty document.
- Excel: one empty sheet, `Sheet1`, in Aptos Narrow 11.
- PowerPoint: 16:9, one master with the layouts Title Slide, Title and Content, Section Header, Two Content, Title Only and Blank, a notes master so slides can carry speaker notes, and no slides.

**Ops that fill a new file in one call.** A model cannot know the anchors an op mints, so every op a new file needs works without one. Each also works in an existing file, where a Word change is a tracked change as always.

| Vocabulary | Op | What it does |
|---|---|---|
| Word | `add_paragraph` (replaces `insert_paragraph_after`) | `text`, optional `style`, optional `after` (a paragraph anchor); without `after` the paragraph goes at the end of the document. A numbered-list paragraph continues the list directly above it, and otherwise starts a new list at 1. |
| Word | `add_table` | `rows` of cell text and optional `after`. The first row is a header row (bold, repeated on each page) unless `header` is false. The table spans the text width in equal columns with single borders, and every cell paragraph gets its own anchor. |
| Excel | `rename_sheet` | Refused while any formula, name, chart or other part refers to the sheet by name, so a reference is never broken: rename before writing formulas that name it. |
| Excel | `format_cells` | A `range` and any of `bold`, `italic`, `number_format` (an Excel format code such as `#,##0.00` or `0%`), `fill` (an RGB hex colour) and `wrap`. Every other part of each cell's existing format is kept. |
| Excel | `set_column_widths` | Widths in characters, by column letter. |
| PowerPoint | `add_slide_from_layout` with `notes` | The new slide's speaker notes. `set_notes` also creates a slide's notes page when it has none, from the deck's notes master. |

Text is plain: Markdown is not interpreted, and headings, lists and emphasis come from styles and ops. An op records every anchor it mints (a table mints one per cell paragraph), so Review's replay keeps a later edit tied to the paragraph it named.

### Everyday document improvements — 2026-09-29, in progress

- [x] Create native tables on PowerPoint slides, with table cells returned by `doc_read` and rendered as a table in Canvas.
- [x] Create native Excel and PowerPoint charts backed by explicit source values; return cached chart series as anchored table data so extraction and RAG do not depend on chart-image OCR. Excel charts use worksheet ranges and an exact top-left cell anchor (`cell`); without one, they start below the source range. Images use an exact cell anchor. The Workbook Canvas draws both objects over the worksheet cells named by their Open XML anchors and labels each `Sheet!Cell`, while chart category/value rows and image alt text remain extractable.
- [x] Embed PNG/JPEG images on PowerPoint slides with required alternative text; expose that text to `doc_read` and label pixels as unread image content. Image bytes are base64 in the worker op, limited to 1 MiB and 40 megapixels, signature and dimensions checked, and fitted within the chosen layout placeholder without changing aspect ratio.
- [x] Insert inline PNG/JPEG images in Word with alternative text; expose the text to `doc_read` and label pixels as unread image content. Existing-document insertions are tracked changes.
- [x] Add basic chart and image placement to PDFs where supported; preserve vector/data content and searchable text. Simple vector bar charts retain searchable source rows; bounded PNG/JPEG image insertion retains a searchable alternative-text caption.
- [x] Confirm acceptance, raw-byte download and round-trip structure for each generated format. Downloads retain the accepted DOCX/XLSX/PPTX package or PDF bytes, including their internal relationships; they are not flattened for export.

Progress: `add_slide_from_layout` accepts native tables, native charts and PNG/JPEG images in a layout placeholder, using that placeholder's bounds; Word accepts inline PNG/JPEG images after a paragraph or at document end. Both embed standard OpenXML media parts with relationships and content types, require meaningful alternative text, bound the data to 1 MiB and 40 megapixels, and re-read the authored alt text. Word image insertions in existing documents are tracked changes. The reader returns alternative text under slide/shape or image anchors and labels pixels unread. Word and PowerPoint table rows flow into Canvas from their structured cells. Excel charts use a two-column worksheet range, cached values projected as structured table data for RAG, and a cell anchor (default: immediately below the source range); workbook images use cell anchors too. Automatic placement checks image and chart extents across the complete operation list, including images added later, and resolved positions survive Review, narrowing and acceptance. The worker returns bounded PNG/JPEG previews to the human Canvas, never to model-visible `doc_read`; alt text and chart values remain the searchable/RAG projection. The Workbook Canvas uses shared column geometry for its headers and cells, honors hidden row and column sizes, and positions previews against the worksheet cell bounds with the resolved `Sheet!Cell` anchor. The agent can set a chart's exact `cell`; a person can select the visual's anchor in Canvas and ask the Agent to move it to a named cell, with the move appearing as a reviewable draft. PowerPoint shape positions are projected and rendered at native slide coordinates. PowerPoint charts use standard Open XML chart parts with explicit category/value caches, projected by `doc_read` as anchored table rows under their slide. LibreOffice's local Impress renderer displays the PowerPoint bars, category labels, values and title. A real CLI-generated slide with a 1×1 PNG imported and rendered in LibreOffice, showing the image contained in its content placeholder without distortion; `doc_read` returned its alt text and `office verify` passed. PDFs add bounded PNG/JPEG XObjects; PNG pixels are decoded with a size cap, JPEG bytes stay standard DCT streams, and alt text is printed as a searchable caption so `doc_read` and RAG retain the description. A real daily PDF with a table, vector chart and image rendered with Poppler; its reader reports one image and extracts the caption. The authenticated development app returns its content projection and downloads identical PDF bytes. The worker integration creates, extracts and downloads image-bearing PDFs; chart source rows remain searchable. The compact Office result card now uses format icons and a smaller preview. Evidence: native table, Excel and PowerPoint chart and PDF vector/image tests; chart/image anchor extraction tests; the brokered `real_office_drafts_are_downloadable_and_rag_readable` server test creates, extracts and downloads chart data across XLSX and PDF, native content plus a described image in DOCX, and a PPTX with table, chart and described image; it now also verifies two Excel tables, charts, and images through `doc_read` and byte-identical raw download; server acceptance tests exercise DOCX, XLSX and PDF review, narrowing, promotion and raw-byte download, including two tables, charts and images in one workbook; local authenticated app projection and SHA-256 matched PDF download; Poppler rendered the PDF; `vak-ooxml`, PDF, `vak-tools` and server tests, `git diff --check` and `cargo fmt --check`; and Office card checks plus the client TypeScript check. The live model-driven turn was canceled on 2026-09-29 at the user's request; it produced no reviewable result. Visual browser check on 2026-09-29 opened the real XLSX draft in the development app and confirmed its native table, image at `Sheet1!D2` and chart at `Sheet1!A10` appear at their cell locations without overlap; the solid blue image is the test's 1×1 PNG. Full page-faithful workbook rendering remains in progress. Excel charts use cached values; stale workbook formulas are rejected for authoring.

**Progress — 2026-09-29.** The model-facing `office_apply` description now
includes PDF image creation and states that a descriptive `alt_text` becomes a
searchable caption for `doc_read` and RAG. This closes a discoverability gap:
`add_image` already existed in the PDF schema and worker, but the overall tool
guidance did not name it for PDFs. A contract test pins the guidance to the
implemented worker behavior; the draft and Review boundary is unchanged.

**Checks.** The same as any draft: each op's postcondition against a re-read, the format verifier in the worker, and Review's change list (the new file's summary, and one choice per op). Developer evidence for the blanks also records how macOS Quick Look draws a file made from each; that is never a runtime check or something shown to a person as one (O7).

**Visio, next.** Visio has a reader and no editing engine. A drawing from scratch needs a blank drawing (one page, no masters) and three ops: `add_page`; `add_shape` (a label and a shape: rectangle, rounded rectangle, ellipse or diamond, with a `key` other ops name); and `connect` (from, to, an optional label). Shapes are laid out automatically in layers, so a model never places coordinates, and the reader gains connectors (`A → B`) so Review and the Drawing view can show them. Nothing on the machine that builds it can open a `.vsdx`, so its evidence will be the verifier, the reader and a structural check against the published schema.

- [x] Built-in blanks for Word, Excel and PowerPoint, deterministic, verified and read back empty. *Evidence: `crates/vak-ooxml/tests/create.rs` (`each_blank_is_an_empty_deterministic_package_of_its_format`, `a_blank_is_refused_for_macro_enabled_names_and_visio`, `a_template_name_makes_a_template_from_the_blank`); `a_file_is_created_from_scratch_verified_and_read_from_the_command_line` (`crates/vak/tests/office_cli.rs`) creates each format through the worker and passes `format.openxml` and a facts read with no flags. Developer evidence only (O7): macOS Quick Look, which uses Apple's own Office importer, drew a memo, a workbook and a six-layout deck made from the blanks as intended: title, headings, bullets, lists numbered from 1 in each list, a bordered table with a shaded header row, a bold filled header row with `#,##0` and `0.0%` formats, and every layout's placeholders in place. Quick Look ignores Word table widths and table styles (a table python-docx writes from Word's own template renders the same way), and falls back to a serif because Aptos is not installed on the machine.*
- [x] `office_apply` creation contract and the `blank` lineage origin through Review, narrowing and `vak office apply`. *Evidence: `a_new_file_is_created_from_scratch_as_a_draft` and `creating_is_refused_where_it_would_hide_a_mistake` (office_apply: an existing name, a digest for a missing file, a source without a digest, a macro-enabled name, Visio, a wrong-vocabulary op); `a_document_created_from_scratch_is_reviewed_narrowed_and_accepted` (vak-server, through the real worker: one choice per op replayed from the blank, a narrowed version that passes the format check, accepted clean); `a_from_scratch_draft_is_offered_change_by_change` (tests/create.rs, including a remapped table-cell anchor and a cell that needs the rename before it).*
- [x] `add_paragraph` (replacing `insert_paragraph_after`), `add_table`, `rename_sheet`, `format_cells`, `set_column_widths`, slide notes. *Evidence: tests/create.rs (a memo with every blank style, two numbered lists each starting at 1, a tracked table in an existing document, a table refused inside a cell; a workbook renamed, filled, formatted and widened, with a bold total row that keeps its number format; a rename refused while a formula names the sheet; a deck from all six layouts with notes, and `set_notes` making a notes page), the `names_sheet` unit test, and the op schema test in office_apply. Two defects found on the way and fixed: a paragraph added inside a table cell failed its own postcondition (a cell paragraph is read as part of its row, so the check now finds it by its anchor there too), and a second numbered list kept counting in Apple's importer, which ignores Word's per-instance start override, so each new list is now its own copy of the list definition.*
- [ ] Journey: in the web client against a live model, a Word document, a workbook and a deck each created from scratch in an empty workspace, reviewed and accepted, and viewed in the Canvas. *Progress, recorded live on 2026-09-27: a dev build in the web client, against the installed data home and its configured model, in an empty git workspace. The memo, the workbook and the deck were each drafted in one `office_apply` call with no source and no digest; Review showed each as a new file with one choice per op (a table as one choice with its rows; each cell of the workbook as its own choice, each saying it builds on the rename); each was accepted whole, and each accepted file passed `vak office verify`. The deck's first call was refused before writing anything because the model gave a Section Header a subtitle; it repaired the call from the error, and the description now lists each blank layout's placeholders. Opening a draft in the Canvas showed only a prompt to start a shared workspace, never the file (the shared Office workspace had replaced the view and showed it only inside a room): fixed, the file is shown and a saved version offers to start a workspace. Not closed because the Excel and PowerPoint views were checked in the Canvas but the grid shows formulas rather than values (Vakyartha does not calculate) and none of the new formatting (the reader does not read formats), the Document view draws lists without bullets or numbers and tables as text lines, the desktop shell was not run, and no file was opened in Office itself (none is installed).*
- [ ] Visio: blank drawing, `add_page`, `add_shape`, `connect`, automatic layout, connectors in the reader. *Seen live on 2026-09-27, in FullAccess: asked for a hiring flowchart as `.vsdx`, the Agent installed a third-party Visio package with pip into the user's Python, built the drawing with a script from that package's sample file, wrote it straight into the workspace with no draft or Review, and said it had verified the file opens, which it had only loaded with that package. FullAccess allows the commands by design (invariant 13); what was missing was the instruction. The `bash` and `office_apply` descriptions and the Visio refusal now say a Visio drawing cannot be made yet and must not be built with a command or script. Not re-checked live after that change.*

### Closing — 2026-09-25

The owner closed the work here: record what the running app does, fix what that exposes, and defer the rest with reasons. Every run below was live in the web client against `gpt-6-luna` on the `openai-responses` route, in a throwaway git workspace, on macOS with no Office application installed. The files were authored for the run with python-docx, python-pptx and openpyxl, whose default templates carry parts Word and PowerPoint saved; the workbook was then given the cached formula values and the `calcPr` Excel writes. They are not files saved by Office, and none is added to the corpus.

- **Journey 1, a cited answer (bar item 1).** One question over a contract, a workbook and a deck. The model read each file with `doc_read` and answered with four citations. Each opened its file at the quoted place: the Word paragraph with its section heading above it, the workbook with the cited range selected and its header row in view, and the deck at the slide.
- **Journey 3, a Word redline partly accepted (bar item 3).** The model made both requested changes as tracked changes with `office_apply`. In Review one change was left out, a version kept the other, and it was accepted. The accepted file's digest is the reviewed version's; its kept change is still a tracked change authored by the Agent, and the left-out paragraph is unchanged. Opening it in Word was not checked: no Word.
- **Journey 4, a workbook edited and reviewed change by change (bar item 4).** Two cell corrections drafted with `set_cells`. Review listed each cell with its before and after, and said that the formulas would show old values until Excel recalculates. One change was accepted, one left out. The accepted file is the reviewed version, sets `fullCalcOnLoad`, and reads every cached value as stale.

Defects the runs exposed, each fixed with a test:
- The model copied a draft out of `.vak/scratch/` into the workspace with a shell command, and in another run rewrote a workbook in place with a script. Both skip Review. In a restricted mode every shell command already asks first, so a person sees either; in FullAccess nothing did. The agent loop now refuses a shell command that names a draft this turn delivered for review (`Not run:`), and the `bash` and `office_apply` descriptions say that only `office_apply` changes an Office file and that a draft is never copied out.
- `doc_read` shows a digest's first 16 characters; the model filled in the rest, and `office_apply` rightly refused it. Its description and its refusal now say that the shown prefix is enough.
- A citation whose sheet name has a space, wrapped in a link to itself (``[`book.xlsx#'Q3 by region'!B2`](…)``), rendered as raw brackets. A code span linked to exactly itself is now read as the code span.
- A cited page began at the cited unit, so a workbook opened without its header row and a paragraph without its heading; it now begins at the start of the section when that fits (`projection::page_start`). A workbook citation selected the whole row, not the cited cells, and a formula with no calculated value showed as an empty cell.
- Review said nothing about formulas left showing old values. The diff now reports them as a `recalculation` impact.

Not addressed, because it is not Office work: every answer whose completion was not judged is marked "partial" with a caution, a plain cited answer included (the outcome runtime, doc 52).

**2026-09-26 — a revision asked for from a review comment failed for every Office draft.** Reproduced live against `gpt-6-luna`: a comment on version 1 of a Word draft, turned into a revision request, ended with "revision did not change candidate files". Four causes, each fixed:
- The revision runs in a task copy whose tool list (`TASK_COPY_TOOLS`, `vak-core`) predated the Office tools, so `doc_read` and `office_apply` were refused as outside the run's scope, and `read`, `write` and `edit` refuse a package. Both are now in the list; they run in the worker, confined to the copy, and `office_apply` writes only under the copy's `.vak/scratch/`.
- `office_apply` never writes the file it edits, and a candidate never includes `.vak/`, so its draft could not become the new version. After the revision turn, the newest successful `office_apply` draft of each file takes that file's place in the task copy (`vak_sandbox::adopt_revision_drafts`), located from the child ledger by the call id through the tool's own `office_apply::draft_dir`, and the copy is frozen as before. The new candidate keeps the draft's execution, so its card reads version 2.
- The stop guard counted `write` and `edit` as modifications by name but not a draft-delivering call, so an Office edit read as "modify" was told no file had changed. A call that delivers a file (`Tool::delivered_file`) now counts.
- With the first three fixed, the live model drafted `reunion-letter-v2.docx` beside the file: the revision request said to "keep existing candidate files" and make "a new version", which it read as "do not overwrite". The request now names the draft's files and says to change them under their own names, never a new one.

*Evidence: `office_revision_is_the_next_version_of_the_draft` (vak-server; it fails with the reported message when the adoption step is removed), `adopted_office_draft_becomes_the_next_version` (vak-sandbox). Live on 2026-09-26, against the installed data home, `gpt-6-luna` and a throwaway workspace: the Agent drafted the lunch change to a letter, Review saved version 1, a comment asking for a line about a vegetarian table was turned into a revision, and the revision read the copy with `doc_read` and drafted `reunion-letter.docx` with `office_apply`. Version 2 is that file alone, with version 1's tracked change and the new tracked paragraph and the original baseline; its format check passed; Review opened on Version 2 and listed both changes; the card read "Draft, version 2, waiting for your review"; and the workspace file was unchanged. Seen and not fixed: the card's facts line (words, tracked changes) still describes version 1, and `find_tools` is offered to the revision though its task-copy scope refuses it.*

**2026-09-26 — a Word edit redlined the whole paragraph, and accepting it damaged the paragraph.** Found by a probe through the worker (`vak office apply`) on a contract clause with a bold lead-in, a bold defined term, a link and a footnote mark. Changing "twelve" to "twenty-four" marked all seven runs deleted and inserted the clause again as one run, which carried the first run's bold, sat outside the link, and left the footnote mark inside the deletion. Accepting it made the whole clause bold, unlinked "the schedule" and removed the footnote mark; the reader does not show footnotes, so neither the model nor Review could say so. A paragraph holding a field (a cross-reference) or another author's tracked change was refused outright, so most clauses of a contract under negotiation could not be edited, and a revision of a draft could not touch a paragraph the Agent had already changed. `replace_paragraph_text` now writes a redline of only what changed (`crates/vak-ooxml/src/edit/redline.rs`, compared by `crates/vak-ooxml/src/edit/textdiff.rs`):
- The paragraph is read as `doc_read` shows it and compared with the requested text word by word. An unchanged stretch no longer than the changes on both sides of it folds into one change, so a rewritten sentence reads as one change rather than as confetti. Typographic quotes, no-break spaces and hyphens compare equal to their plain forms, and the document keeps its own.
- A run the change splits keeps its formatting on every piece. New text looks like the text it replaces (the run most of it came from), or like the text before it; text added after a link goes outside the link, and text added after a sentence goes after its footnote mark.
- A field's result, another author's tracked change and the text of an equation or drawing never change: a change to one is refused naming its text, and the rest of the paragraph can change around it. Content the reader does not show (footnote and endnote marks, images, bookmarks, comment ranges, field codes, hidden and white text, another author's deletions) is never removed. A paragraph that starts inside a field begun in an earlier paragraph (a table of contents) is guarded to that field's end.
- The Agent's own earlier tracked changes are revised, never stacked: the comparison is made against the paragraph as it read before them, so revising a draft's change leaves one change, asking for the original text withdraws it, and asking for what it already reads writes nothing.
- The postcondition re-reads the written paragraph: with the Agent's changes accepted it reads the requested text, with them rejected it reads exactly what it read before, and it still holds everything the reader does not show.

*Evidence: in `crates/vak-ooxml/tests/edit.rs`, `a_changed_word_is_the_only_change_and_everything_around_it_is_kept`, `replaced_text_looks_like_the_text_it_replaces`, `new_text_goes_after_a_link_and_after_a_footnote_mark`, `fields_and_other_authors_changes_stay_and_a_change_to_them_is_named`, `the_authors_own_change_is_revised_not_stacked`, `a_paragraph_added_in_a_draft_is_edited_as_one_insertion`, `hidden_text_and_images_are_never_removed`, `an_empty_paragraph_is_filled_in_its_marks_formatting`, `a_rewritten_sentence_is_one_change` and `a_paragraph_inside_a_field_begun_earlier_is_guarded_to_its_end`; in `crates/vak-ooxml/tests/review.rs`, `a_word_edit_is_offered_and_kept_as_its_changed_words`; the comparison's unit tests in `textdiff.rs`; and the same clause through `vak office apply`, whose paragraph now holds one deletion and one insertion with every other run byte for byte. Live on 2026-09-26, a dev build in the web client against the installed data home and its configured model, in a throwaway git workspace holding that clause, a cross-reference clause and a clause the counterparty had redlined: asked to change the term to twenty-four months, the Agent drafted it with `office_apply`; Review listed one change whose only marks were "twelve" struck and "twenty-four" inserted, the format check passed, and the accepted file held one deletion and one insertion with the bold lead-in, the defined term, the link and the footnote mark byte for byte. Asked next for a sixty-day notice period and fees due within 60 days "of receipt of invoice", it drafted both paragraphs the old op refused: Review offered each as "Edit paragraph", only "thirty" to "sixty" and "receipt of" were marked, and the cross-reference and the counterparty's change stayed byte for byte. Not checked: opening the result in Word (no Word on this machine), the desktop shell, and Strict documents.*

**2026-09-26 — tables, the other side's changes, new documents, word counts and the Workbench.** Five gaps left by the redline work, closed the same day:
- **A Word table's cells can be changed.** A table row was one line with no anchors for the paragraphs in its cells, so the model could not name a cell, and an edit by a cell paragraph's own anchor failed its postcondition. A row's line now names each cell's paragraphs (`[tbl@1/r2] [p@8] North | [p@9] 120`, from `Unit::row_cells`), `replace_paragraph_text` changes a cell like any paragraph, and a citation of a cell's paragraph opens its row (`projection::locate`).
- **Another author's tracked insertion can be countered.** Striking text the other side inserted is a deletion nested inside their insertion, as Word writes it; new text beside or inside it closes their insertion and opens it again, with a fresh id, around the new text, so an insertion is never nested in theirs. Their deletions and moves, and an insertion of theirs holding anything but plain text, stay fixed, and a change to them is refused as before. The postcondition's rejected view still reads exactly the text before.
- **A new document is written clean (R7).** `EditContext::tracked` decides. `office_apply` writes clean when its `path` is not yet in the workspace (as when creating from a template); `vak office apply` writes clean when its source is a template; Review's replay reads the lineage's `new_file`; and a revision runs in a task copy that holds the new file, so the server names the draft's new documents and the broker hands that list to the worker (`ToolContext::new_documents`), never through the model. In a new document replaced text simply goes, a new paragraph is plain, and a deleted paragraph is removed, unless that would break a section, the mark of a comment or note, or a field, bookmark or comment range that continues elsewhere. Removing a `p@N` paragraph renumbers those after it, so one call refuses a later op that names a renumbered paragraph (`edit::check_renumbering`, run by `apply_checked`), and Review makes such an op depend on the removal. A paragraph-count postcondition covers inserts and removals.
- **Word counts count words.** The facts line counted the reader's markers (`[deleted by`, author names) and table separators as words. Words now come from the text as it reads, without markers or deleted and hidden text.
- **The Workbench shows Office files.** Its file panel said "This format cannot be previewed here yet" for a Word, Excel or PowerPoint result; it now shows the Canvas's Document, Workbook or Deck view, with Download.

*Evidence: `a_table_cell_is_edited_by_its_paragraphs_anchor`, `fields_and_other_authors_changes_stay_and_a_change_to_them_is_named` (the counter to "60"), `another_authors_insertion_is_split_around_new_text_never_nested`, `a_new_document_is_written_clean`, `removing_a_paragraph_from_a_new_document_renumbers_what_follows` and `a_new_documents_paragraph_is_not_removed_when_that_would_break_it` (tests/edit.rs); `in_a_new_document_an_edit_that_names_a_renumbered_paragraph_needs_the_removal` (tests/review.rs); the word count in `word_projection_anchors_and_labels_hidden_content` (tests/package.rs); `a_new_document_is_written_clean_and_an_existing_one_tracked` (office_apply); and `the_worker_writes_a_new_document_clean_when_the_runtime_says_so` (vak-server, through the real worker). LIVE_FOLLOWUPS*

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
- [x] Journey evidence: a real Agent conversation answers questions over a real document, workbook and deck and cites anchors that resolve to the quoted text. *Evidence: "Closing — 2026-09-25", journey 1; files authored for the run, not saved by Office.*

### P2 — Edit and create

*Progress 2026-09-24:* `crates/vak-ooxml/src/splice.rs` (L1), `src/edit/` (L3: `mod.rs`, `word.rs`, `sheet.rs`, `deck.rs`) and `crates/vak-tools/src/office_apply.rs`. Evidence: `crates/vak-ooxml/tests/edit.rs` (13 tests), the `splice` unit tests, `office_apply`'s tests in `vak-tools`, `crates/vak-permission/tests/document_tools.rs`, and `office_apply_edits_through_the_worker_as_the_calling_agent` in `crates/vak-server/tests/doc_read_worker.rs`, which runs the real worker. Two defects found on the way were fixed: the broker never forwarded the Agent id to the worker, so inside the worker `bash` always used the `vak` scratch directory whichever Agent ran it; and `doc_read` was missing from the permission engine's read tools, so read-only mode denied it and workspace-write asked for approval.

- [ ] L1 splice editor: edits replace located element ranges in the event stream and keep everything else byte-identical; an unknown-markup preservation test for every op (O1). *Progress: built. Raw-copy identity of every untouched part is tested for Word, Excel and PowerPoint ops, and byte identity before the edited paragraph for Word. `markup_vak_does_not_model_survives_inside_edited_elements` puts unknown elements and attributes inside the edited elements for the three Word ops, `set_cells` (which `append_rows` uses) and `set_placeholder_text` (whose text replacement `set_notes` shares). Not yet covered by that test: `add_slide_from_layout`, `delete_slide`, `move_slide`, `add_sheet` and `set_title`.*
- [x] `OfficeOp` v1 schema and one apply engine with per-op postconditions checked by re-reading the written package (O5); stale or missing anchors are repairable errors (O4). *Evidence: `tests/edit.rs`; unknown fields are refused, every refusal names the op and the repair, and `base_digest` must match the source's sha256.*
- [x] `office_apply` writing to a draft that freezes as a candidate. *Evidence: `an_edit_is_a_draft_and_the_workspace_file_is_untouched` (the draft lands in `.vak/scratch/<agent>/<execution>/`, the workspace file is byte-identical, the Workbench receives `ExecutionStarted` and `ExecutionFinished` with the draft as its artifact), `drafts_chain_and_a_template_creates_a_new_file`, and the server test `an_office_draft_is_reviewed_by_meaning_and_accepted_through_promotion`.*
- [x] Word edits as native tracked changes under the runtime's Agent id (`w:ins`/`w:del` on runs and on the paragraph mark, new paragraphs with a fresh `w14:paraId`, `w14` declared when missing). *Evidence: `tests/edit.rs`, and the worker test showing the author is the calling Agent's id.*
- [x] Excel edits keep cell styles, set `fullCalcOnLoad`, drop the calculation chain when formulas change, and report cached values as stale. *Evidence: `tests/edit.rs`.*
- [x] PowerPoint slides from the template's layouts and placeholders, placeholder text, notes, move and delete (including section-list slide ids). *Evidence: `tests/edit.rs`.*
- [x] Creation from a workspace template: a `.potx`/`.dotx`/`.xltx` becomes the document named by `path`, theme, styles and layouts kept. *Evidence: `a_template_becomes_a_document_but_never_changes_macro_state`, `creates_a_deck_from_a_template_with_the_agent_as_author`.*
- [x] Built-in blank packages for Word, Excel and PowerPoint. *Evidence: "Creating from scratch — 2026-09-27".*
- [x] O8 protection enforcement and the O10 refusals, each with a test that the refusal happens before any byte is written. *Evidence: `protection_is_honoured_before_any_byte_is_written` (Word read-only and comments-only protection refused, tracked-changes-only protection allowed because every op is a tracked change, PowerPoint password to modify refused), `excel_refusals` (protected sheet), the workbook-structure check in `add_sheet`, the macro-state refusals, and `office_apply`'s test that a refused op leaves the file byte-identical with no temporary file behind.*
- [ ] `prepare_to_share`.
- [ ] Journey evidence: an Agent creates a six-slide deck from a brief and the workspace's template, freezes it as a candidate, and passes Vak's verifier. The CI oracles pass on that file. A manual PowerPoint open is recorded as developer evidence, not a runtime check.

### P3 — Review

*Progress 2026-09-24:* `crates/vak-ooxml/src/diff.rs` and `src/review.rs`, the `OfficeReview` and `OfficeNarrow` worker tasks, `GET /sessions/{id}/sandbox/candidates/{cid}/office-review` and `POST …/office-narrow`, `office_apply` drafts, and in the client `src/officeRedline.ts`, `src/components/OfficeChangeList.tsx` and the Review wiring in `WorkbenchPanel.tsx`.

- [x] L4 semantic diff for Word, Excel and PowerPoint: paragraphs changed or redlined and grouped under their heading, cells old → new (a cached value turning stale is not counted as a change), slides added, removed, moved or retitled, and a new file summarised. It compares two re-reads rather than the op log, so it describes an external edit the same way. *Evidence: `crates/vak-ooxml/tests/diff.rs`; the endpoint runs it in a worker under the read-only sandbox (server test above).*
- [x] Per-change acceptance (R6) by op replay. `vak_ooxml::review` offers a draft's changes as choices (one per op; one per cell of a `set_cells` op), each with the changes it makes (attributed by replaying the ops one at a time and diffing each step) and the earlier choices it builds on (a paragraph or slide an earlier op minted, a sheet an earlier op added). The server assembles the draft's lineage from the session ledger: the `office_apply` call whose id is the candidate's execution, then each call whose `source` was an earlier draft, back to the file the chain started from and its `base_digest`. Narrowing replays only the kept ops against that file, remapping minted anchors (`p:<paraId>`, `slide:<id>`) to the ones the replay mints, so a kept op still lands where it was written; the result is a new candidate version (`narrowed`, parent = the full draft) that is verified, reviewed and accepted like any other, and the full draft stays. A draft the lineage does not reproduce (a command edited it, or the source changed) is offered only whole, and narrowing it is refused, so a hand edit is never silently dropped. Once one version of a result is accepted, the others stop being pending. *Evidence: `crates/vak-ooxml/tests/review.rs` (4, including the minted-id remap that a naive replay gets silently wrong), the server tests `an_office_draft_is_reviewed_by_meaning_narrowed_and_accepted_through_promotion` (a two-call chain, narrowed in the sandboxed worker, accepted, the left-out change absent) and `a_draft_changed_after_office_apply_is_offered_only_whole`, `tests/office-choices.mjs`, and the component harness checks in the browser at desktop and 375 px phone width. Not checked: the full Review modal against a live server and a live model. The replay found a P2 bug, now fixed: two slide-list ops in one call failed their postconditions, because an earlier op's slide order was checked against the final deck (`several_slide_list_ops_in_one_call_are_checked_against_the_order_they_leave`).*
- [ ] Review (U5) shows the change list first and lists checks and signature and label impact (F7). *Progress: for an Office file, Review shows the semantic change list in place of the raw before/after text columns, with tracked changes drawn as `<ins>`/`<del>` elements parsed from the reader's markers (never mounted HTML), the file's flags, and whether it is compared with the workspace or is new; with choices, each change has a checkbox and Accept waits until the kept changes are made into a version. "Open saved version in Canvas" is hidden for Office files until P4 gives Canvas a real view (F6). Signature and label impact (D4, O9): an edit to a signed file drops the package's signature origin, signature parts, their relationships and content types, and the `office_apply` result records it; Review states before acceptance what accepting does to signatures (removed, or a signature a changed draft still claims but no longer holds) and to sensitivity labels (kept, added, changed or removed, with removal and change marked as warnings). *Evidence: `an_edit_removes_the_signatures_it_invalidates_and_keeps_the_label` (tests/edit.rs), `accepting_states_what_happens_to_signatures_and_labels` (tests/diff.rs), `editing_a_signed_file_records_that_its_signature_was_removed` (office_apply), the harness in the browser. Signatures are still not verified (D9).* Checked live on 2026-09-24: a dev build served a disposable workspace (`/tmp/vak-live/workspace`, port 8931) with the owner's configured provider; the Agent, asked to raise two budget cells as a draft, called `doc_read` and `office_apply` (two `set_cells` ops), leaving the workspace file untouched; Review showed the two cell changes as choices; leaving one out made a narrowed version in the worker (format check passed); accepting it wrote only the kept cell, with untouched parts copied byte for byte; and undoing the acceptance restored the original exactly. The run found and fixed three client faults: the Review dialog squeezed the change list to a few pixels on narrow or short windows (it now scrolls as one page with header and actions pinned), and two places still treated a draft whose narrowed version was accepted as pending (one round-based rule, `src/candidateVersions.ts`, now decides for every place, with `tests/candidate-versions.mjs`). Not yet exercised live: Word and PowerPoint drafts, signed or labelled files, and anchored comments.*
- [x] Office anchor variant for candidate comments (F9). A comment on an Office file carries an `anchor` (`Budget!B4`, `'Q4 plan'!A1`, `p:1A2B3C4D`, `slide:256/shape:3`) instead of line numbers, which the server refuses for a package; the anchor's shape is checked (`vak_ooxml::is_anchor`), it is stored with the comment and listed, and a revision request names it ("file budget.xlsx, at Budget!B4"). In Review each change has a Comment action that points the comment form at its anchor. Diff anchors for sheets whose names need quotes now match the reader's (`'Q4 plan'!A1`, was `Q4 plan!A1`), so a comment or citation on such a cell resolves. *Evidence: `anchors_are_recognised_by_shape` (tests/package.rs), `a_cell_on_a_sheet_whose_name_needs_quotes_is_anchored_as_the_reader_anchors_it` (tests/diff.rs, fails without the fix), the anchored-comment steps of the server narrowing test, `a_revision_request_names_the_cell_or_lines_a_comment_points_at`, and the harness in the browser.*
- [x] Journey evidence: a person reviews a real Agent-edited workbook in the running UI, accepts some changes and rejects others, and the accepted file matches the reviewed derived digest. *Evidence: "Closing — 2026-09-25", journey 4, in the web client; the desktop shell was not run.*

### P4 — Views and file card

- [ ] `OfficeProjection` schema (paged: document blocks, sheet ranges, slides) and authenticated projection routes shared unchanged by desktop and web. *Progress: `vak_ooxml::projection` pages the reader's own units under a byte budget with the outline (each entry names its first unit) and the Structure listing (parts, content types, sizes, relationships, external targets recorded); `GET /fs/office` and `GET /sessions/{id}/sandbox/candidates/{cid}/office` answer through the `OfficeProject` worker task. Evidence: `tests/projection.rs`, `the_canvas_reads_an_office_file_as_pages_and_structure_through_the_worker`, the web client live. Not checked in the desktop shell.*
- [ ] U1 Document, U2 Workbook, U3 Deck and U4 Structure replace the Code fallback (F6), with the flag banner and point-then-act anchor chips. *Progress: the Canvas opens a workspace file or a draft in its view: Document (headings, redline, labels), Workbook (sheet tabs, grid, formula bar with the stale-value marker), Deck (slide outline, one card per slide, a shape's paragraphs on their own lines — the reader separates them with `PARAGRAPH_BREAK`, " ¶ ", since a slash is ordinary text), and Structure; a selection offers "Ask Vak about this", which puts the anchor in the composer. Checked live on 2026-09-25 in the web client: a Workbook draft, a Word draft and a PowerPoint draft opened in their views. Not checked: the desktop shell, a file saved by Office, and comments drawn in the views.*

  Citations (the "Read and cite" row of the experience): `doc_read` tells the model to cite a place as `` `path#anchor` ``; the chat turns such a span into a link, in the live answer (`md.ts`) and the settled one (`PresentationRenderer`), and choosing it opens the file's view at the page that holds the cited unit and selects it (`view=at`, `vak_ooxml::projection::locate`, which also finds a cell's row and a range's first row). `is_anchor` and its client mirror (`src/officeFiles.ts`) now accept the reader's table and comment anchors (`tbl@1/r2`, `p@2/comment:0`), which they had refused. Evidence: `a_citation_finds_the_unit_it_names` (tests/projection.rs), the `at` assertions in the projection route test, `tests/office-files.mjs`; checked live on 2026-09-25: both a table-row and a heading citation opened the Word view at that unit. The 2B local model under test did not cite anchors on its own; it read the file and answered in prose, or tried a table card it could not form.

  Live runs with a small local model also showed what the loop must not leave to the model's reading of a result: it edited a `.docx` with the text `edit` tool (text tools now refuse an Office file before any approval is asked, `Tool::refusal`), named a paragraph by its text (a missed anchor now suggests the paragraphs whose text matches), wrote malformed slide ops (the op schema is a tagged `oneOf` whose errors name the branch), answered a delivered draft with HTML preview cards, and repeated the same `office_apply` call until three identical drafts existed. `Tool::delivered_file` now names the file a call delivers: the presentation check stands down for the run, an identical call gets the first call's result without a second draft or a second approval, and a card whose `artifact_path` previews that file is not shown. Evidence: `crates/vak-agent/tests/tool_refusal.rs`; checked live on 2026-09-25, where a repeated call was answered "Already drafted" and the turn ended on one sentence.
- [ ] U6 file card for deliverables and inbound files; desktop and web drop-in through one HTML5 intake and upload route into the inbox (`dragDropEnabled: false` in `crates/vak-desktop/tauri.conf.json`). *Progress: a file dropped anywhere on the conversation, or picked with Attach, is saved to the workspace inbox through `POST /fs/inbox` (the gateway's own `save_to_inbox`, moved to `vak-server/src/inbox.rs`, so a channel and a client save the same way); a run names the saved paths (`files`), and the model gets the note that says where the file is and which reader understands it, never its bytes, while the message records `MessageMeta::attachments`, so the chat draws a file card instead of the note (typed, not sniffed). Before this, the composer read any non-image file as text into the prompt, a `.docx` included. Images still go as vision blocks and small text files are still quoted. A file named in a message (an attached file, or a file a document points to) is the `Artifact` file row, and a file the Agent produced is the `ResultCard` (docs/design/75 §6.1, V3.14): a preview, its status from the server's records (`ArtifactRef.status`) and Review changes, Open, Download and Ask for changes; both carry the reader's facts from a `view=facts` projection (kind, headline counts, flags; cached per path so a re-rendered timeline does not read the file again) and, where the host has it, Open with. `dragDropEnabled` is false, so the desktop webview gets the same HTML5 drop. Evidence: `a_dropped_file_reaches_the_model_as_a_note_and_the_chat_as_a_file` (http_e2e), the facts assertions in `the_canvas_reads_an_office_file_as_pages_and_structure_through_the_worker`, `tests/office-facts.mjs`, `tests/attach-files.mjs`; checked live on 2026-09-25 in the web client (a deck and a Word file dropped, their cards, the model reading the inbox copy with `doc_read`, Download). The first live run found a request storm (one facts read per card per render, over 20,000 requests until the browser refused) and a model that could not reach `doc_read`, which declared only `documents` and so was deferred when `read` refused the file; `doc_read` now also serves `filesystem`, and names the inbox copy when it is asked for a file by the name it was sent with. Not checked: the desktop shell's drop.*
- [ ] Host port: `saveText` replaced by `saveFile(name, bytes, mime)` with its call sites migrated (invariant 30); `open-with` as a desktop-only `HostFeature` backed by a scoped opener limited to workspace files. *Progress: `Host.saveFile(name, bytes, mime)` replaced `saveText`, and its one call site moved (the desktop's `export_file` takes the bytes as a raw IPC body, not a JSON array); `open-with` is a desktop `HostFeature` whose `open_workspace_file` command opens only a regular Word, Excel, PowerPoint or Visio file inside the open workspace, refuses a macro-enabled one, and hands it to the system opener without reading it. Evidence: type-checked and built; the web download checked live. Not checked: the desktop save dialog and Open with, which need the desktop app run by hand.*
- [ ] Journey evidence, in both the desktop app and the web client on a machine without Office: a person drops a real deck, asks a question, follows a citation to the slide, asks for a change, reviews and accepts it, and downloads the file.

### P5 — Headless and channels

- [ ] `vak office read|apply|diff|verify` CLI verbs over the same crates and op schema, JSON in and out, no logic of their own (invariant 19). *Progress: built. Each verb parses its arguments and prints the worker's JSON: `read` is `office_project` (content from a unit or a cited anchor, `--structure`, `--facts`), `diff` is `office_review` (the change list with signature and label impact), `verify` is the `format.openxml` target verifier (exit 1 when it fails), and `apply` is a new `OfficeApply` worker task that shares `office_apply`'s one checked apply (`apply_checked`: the digest check, the engine, the re-read) and writes a new file at `--out`, never the source and never over an existing file, with Word changes attributed to `--agent`'s tracked-change author. Nothing is parsed in the CLI process. Evidence: `crates/vak/tests/office_cli.rs`; run by hand on the live workspace's budget workbook.*
- [ ] Channel round trip: an inbound workbook is updated and returned under the bound bot identity (invariant 24) with a change summary, subject to label narrowing. Raise the `/gateway/inbound` body limit so channel files larger than 1 MiB can arrive. *Progress: a channel turn's reply now carries each file it drafted with `office_apply` (the latest draft per path, found from the turn's successful calls by the scratch convention Review uses), under the name it was sent with and captioned with the worker's change summary, to a bridge that declares `accepts_files`; the Telegram bridge declares it and sends each with `sendDocument` under its own bot token after the text. A draft that carries a sensitivity label is held with a line saying so (labels only narrow where a file goes), and a bridge without files, or a draft over 20 MiB, gets a line saying the draft is in Vak. A channel document may now be up to 20 MiB (Telegram's download limit), with `/gateway/inbound`'s body limit raised to match, and the words sent with a file (Telegram's `caption`) now reach the Agent; before, a document's caption was dropped. Evidence: `crates/vak-server/tests/channel_office_round_trip.rs`: a workbook through the gateway, a labelled draft held, a bridge without files, and the whole Telegram path against a Bot API double, all in workspace-write mode. Not checked: a real Telegram chat.*
- [ ] Scheduled jobs: a scheduled task that updates a named workbook produces a candidate, or auto-accepts only inside an explicit envelope, with a run record as its receipt. *Waiting on the data architecture plan's M4 (runs and schedules), which gives every trigger a run record and a scheduled run a working environment outside git; `AgentSchedule`, which was never run, was removed in M0.*
- [ ] Journey evidence: a real Telegram round trip and one real scheduled run, each with receipts, on a machine with no Office application installed.

### Deferred until asked

Each of these needs a real request before it is started. The first rows were in the 2026-09-23 plan; the rest are the ledger's unchecked boxes at the 2026-09-25 close.

| Item | Why deferred |
|---|---|
| Office page thumbnails and per-sheet print-to-PDF export (`vak-ooxml-layout`, bundled fonts) | The requested workbook Canvas is being improved as a spreadsheet view. A separate thumbnail and print-export workflow still needs its own design; **Open with…** and **Download** preserve the original file meanwhile. |
| Formula calculation engine (`vak-ooxml-calc`; IronCalc was the candidate) | `fullCalcOnLoad` plus honest "stale" labels are correct without it. Needed only if people must see recalculated values inside Vak. |
| Live multi-person co-editing (op sequencing, presence, conflict streaming) | Draft plus review already gives attributed, reversible collaboration. |
| Visio editing | Reading covers the knowledge use case; no editing request exists. |
| Agile decryption and XML-DSig signature verification | Needs its own cryptography dependencies (decision D9) and a password flow. Refused with a reason until asked. |
| A knowledge index across a folder of Office files | `doc_read` per file serves the loop. An index waits for a real "search my folder" need. |
| A VBA interpreter, macro runs, VBA editing, trusted macros | Cut, not deferred (O10). |
| Macro understanding: the CFB reader, MS-OVBA decompression, modules and entry points (bar item 6), and with the CFB reader, telling an encrypted package from a legacy binary file | Macros are detected and flagged today. Explaining one needs a new reader of a second hostile format; it waits for a real macro workbook to explain. |
| `prepare_to_share` | Waits for a request to clean a file before it leaves Vak. Creating from templates and blank packages, deferred here at the 2026-09-25 close, was reopened on 2026-09-27 ("Creating from scratch"). |
| A live Telegram round trip (bar item 5) | Built and tested; the owner parked Telegram work on 2026-09-25. |
| Scheduled Office jobs | Wait on the data architecture plan's M4 (runs and schedules). |
| Application-authored corpus, the byte-identical no-op round trip over it, and the CI oracles (Open XML SDK, LibreOffice) | Need files saved by Office and a CI container; neither exists yet. Generated fixtures and the adversarial set cover the reader and writer meanwhile. |
| Fuzz targets for L0 and L1 | Need a nightly job; the 1,200-package mutation smoke test runs in the normal suite. |
| Schema-conformance validator | Needs the schemas' redistribution terms confirmed first. |
| `data_query` over ranges and tables | `doc_read` answered the loop's questions in journey 1. |
| Read coverage not yet built: headers and footers, footnotes, charts, SmartArt, number formats, pivot tables, PowerPoint comments, layout and master inheritance, Visio masters | Each reader lists what it does not read, so an omission is never read as absence. |
| The inbox intake's reader summary and recorded digest entry | The saved file, its digest-prefixed name and the reader note already reach the model. |
| Unknown-markup preservation tests for `add_slide_from_layout`, `delete_slide`, `move_slide`, `add_sheet` and `set_title` | Raw-copy identity of untouched parts is tested for every op; the in-element test covers the others. |
| The desktop shell: Canvas views, the save dialog and Open with | Checked in the web client; the desktop app needs a person to run it. |
| Opening an accepted Word redline in Word (bar item 3's last step) | No Office application on the machine that recorded the journeys. |

### Shared Office workspace — 2026-09-26

The built-in Artifact Canvas and shared-conversation Office view now use one first-party workspace over saved Office candidates. Explicit invitation `edit` grants enable typed Word paragraph, Excel cell, and PowerPoint placeholder edits. Each save is a new worker-applied, verified candidate revision with an expected-head check. Named branches can be created and merged when operation anchors are disjoint; stale writes and overlapping edits return conflicts. Agent candidates can be imported only when their recorded base digest matches the target branch. Owner Review remains the only path to workspace promotion, with a direct action from the owner's workspace into the existing Review panel. Presence carries the active room and selection anchor; saved-candidate comments remain attributed. Verified: `npm run build` (desktop and web bundles), `cargo check -p vak-server`, the focused workspace conflict tests, the invitation route-scope test, and the Office client file/cell/facts tests. The shared invitation entry route loaded in the in-app browser without an error overlay. A live two-client edit/review, full Office workspace screenshot audit, desktop shell, and 390px layout audit have not been run. Full-page fidelity, formula recalculation, and character-level simultaneous editing are not implemented.

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
| R9 | Rendering | Structured, standards-based views with page-faithful rendering phased in natively; external office applications are never a runtime dependency (owner clarification, 2026-09-29). **Open with…** / **Download** remain available. |
| R10 | Inbox writes on read-only channels | Allowed: a received file is the sender's, not an Agent write; the inbox never overwrites (2026-09-24). |
| D4 | Signature handling on edit | Drop invalid signature parts and record it, announced before acceptance. |
| D7 | CFB reader | Needed to tell encrypted packages from legacy binaries and to read VBA source for P1 understanding. Evaluate the `cfb` crate, pinned, else write a bounded reader. |
| D9 | Cryptography | Deferred with decryption and signature verification. When asked: pinned RustCrypto crates and a bounded exclusive C14N; `ring` covers Ed25519 only. |

## Completion bar

The loop is complete when, **on a machine with no Office application installed**, the running desktop app and web client (one shared UI) and the channel path demonstrate with real files, a real Agent and recorded evidence:

1. a cited answer over a real document, workbook and deck, whose citations open at the quoted place;
2. an Agent-created deck, workbook and document, from the workspace's template or from scratch, reviewed and accepted;
3. a Word redline by an Agent, partly accepted in Vak and then opened in Word with the remaining changes still tracked (developer evidence);
4. a workbook edited by an Agent with formulas marked stale, reviewed change by change;
5. a Telegram round trip returning the updated file with a change summary;
6. a macro-enabled workbook read, its macro explained and flagged, and its behaviour rewritten as a Vak automation under review.

Every produced file passes Vak's verifier and the CI oracles. Record what was not checked.

At the 2026-09-25 close: items 1, 3 and 4 were recorded in the web client, except opening the redline in Word; item 2 (templates), item 5 (Telegram, parked) and item 6 (macros) are deferred; every produced file passed Vak's verifier, and the CI oracles do not exist yet.

Related: `54-task-environments-and-promotion.md` (candidates and verifiers), `69-shared-conversation-coworking.md` (principals and comments), `70-calm-agent-experience-implementation.md` (Canvas and Review), `24-agent-security.md` (threat model).
