# 77 — PDF documents: reader, writer, review and shared drafts

Status: **shipped 2026-09-29; everyday authoring and extraction improvements continue.** The reader, writer, Review with choices, shared drafts, verifier, inbox, citations, CLI, vector charts and bounded PNG/JPEG authoring are shipped. Multi-page authored tables now repeat their header on continuation pages, and `doc_read(view="table")` exposes conservatively recognized aligned text tables.

## Owner direction

**2026-09-27.** Add PDF beside the Open XML pipeline as its own crate, written fresh rather than as a wrapper around a PDF library, and wired the way `crates/vak-ooxml` is. Then give it a writer, and the same loop Office files have: drafting, review, preview and collaboration.

## Why

A PDF is the document people most often send an Agent. Before this, a PDF that arrived on a channel or was dropped in a client was saved with the note that no reader understood it, and the only PDF code in the tree was a structure check built on a third-party library.

## Decisions

- **D1 — our own engine.** `crates/vak-pdf` is written from ISO 32000-2 with no PDF library beneath it and no other vak crate as a dependency, so it can be tested and fuzzed alone. Its outside dependencies are `flate2` for zlib and `png` for bounded authoring decode; JPEG image bytes use the standard DCT stream and are not decoded. The old verifier moved onto it and `lopdf` left the workspace (invariant 30).
- **D2 — worker only.** Every call site runs the engine inside the broker worker (invariant 14): `doc_read` and `office_apply` are worker tools, and review, narrowing, projection, apply and verification are worker tasks. A defect in the engine on some input is caught and reported as a damaged file, never a crash of the worker.
- **D3 — one loop, not a second one.** A PDF goes through the Office loop's own surfaces: `office_apply` takes a `.pdf` path, the candidate review, narrowing and projection endpoints dispatch on the file's kind, shared drafts hold PDF ops, and `vak office` reads, applies, compares and verifies PDFs. The review, change-list and projection shapes are the Office ones, so one client view draws both.
- **D4 — bounded.** `vak_pdf::Limits` caps the file (128 MiB), the object count, each decoded stream (64 MiB) and the whole document's decoding (512 MiB, counted as bytes produced), pages read (2,000), lines per page, line length, form nesting, bookmarks and annotations; nesting depths and reference chains are bounded, and cycles are visited once.
- **D5 — nothing runs or is followed.** JavaScript, automatic, launch, submit and remote actions, embedded files, XFA forms, rich media and links are counted and named in the flags. Signatures are counted, never verified or made.
- **D6 — labelled text.** Invisible text (rendering mode 3 or 7, often a scan's recognised-text layer), white, tiny (under 1pt) and off-page text carries a label, so a hidden instruction never reads as body text. Comments, highlights (with the text under them) and form fields follow a page's text, labelled with their kind and author.
- **D7 — anchors.** Every line is `page:<n>/line:<m>`, counted from 1 as a viewer counts pages, valid for the file's digest. Every anchor in one `office_apply` call names the read that call started from, whatever the ops before it did, so each op stands alone.
- **D8 — clean writes.** Every write produces the whole file again, keeping only what the catalog reaches, renumbered: content an edit removes (a deleted line, a deleted page, the old text of a rewritten line) is gone from the file, not left in an earlier revision. A stream no op touched keeps its encoded bytes exactly. Earlier saved revisions are dropped, which Review states before acceptance.
- **D9 — confirmed by a re-read.** A written file is read again and every op's change must show (the new text on its page, the comment, the rotation, the title), or the call fails and nothing is written.
- **D10 — what an edit may be.** A replaced line is drawn in Helvetica, at the old text's size and place, with the page's own text state put back after it; the old text operations become moves of the same length, so the rest of the page does not shift. Hidden text may be deleted, never rewritten as visible text. A line drawn inside a form XObject is refused, since an edit of the page cannot reach it. New content is set on new A4 pages (the new-document size Word uses) in the styles a new Word document offers, in the standard Helvetica faces with their real widths, so nothing is embedded; text a WinAnsi Helvetica cannot draw is refused by name, never dropped.
- **D11 — detection by bytes.** `doc_read` routes a file named `.pdf`, or any file that starts `%PDF-`, to the reader; a file named `.pdf` that is not one fails with that reason.
- **D12 — refusal over half-reading.** An encrypted PDF is refused whole, even one that opens without a password: decryption is not implemented.
- **D13 — images remain ordinary and extractable.** New PNG and JPEG images are embedded as PDF image XObjects. A visible, searchable `Image description:` caption carries the required alternative text into `doc_read` and RAG projections; image pixels are not OCR-read. Inputs are bounded before decode and nonstandard image types are refused.

**Progress — 2026-09-29.** A PDF table that spans pages repeats its header row
after each automatic page break. The regression test reads the generated PDF
back, confirms the header occurs on multiple pages, and checks the first and
last data rows remain searchable. This applies to authored tables and the
searchable data tables printed below vector charts.

**Progress — 2026-09-29.** The reader keeps large-gap text runs with their
horizontal coordinates and recognizes a table only when at least two
consecutive lines repeat the same column count, aligned starts and row spacing.
Recognized rows are available as `page:<n>/table:<m>` in the paginated
`doc_read(view="table")` view and contribute to PDF table stats. Ambiguous or
complex layouts remain in the anchored text view; the reader does not guess a
grid from a single line or from irregular columns.

## The op set

The ops a Word document shares keep Word's names and fields: `replace_paragraph_text` and `delete_paragraph` act on one line, `add_paragraph`, `add_table`, and `add_image` set new content (after a page, `after: page:3`, or at the end), and `set_title` sets the title. PDF image descriptions are printed as searchable captions. A PDF adds `add_chart`, `add_page_break`, `add_comment` and `highlight` on a line (authored by the runtime's Agent id, never a name the model supplies), `fill_field` (text, choice, check box and radio fields; the viewer redraws them), and `rotate_page`, `delete_page` and `move_page`. A new PDF takes only the ops that need no anchor.

## Review, choices and shared drafts

A draft's lineage is its origin plus one step per `office_apply` call. A one-step draft offers one choice per op, each shown with the changes it makes alone; keeping some replays exactly those ops against the same source, and the narrower draft is checked like the original. A draft made in several calls names places in drafts in between, so it is taken whole. The diff matches pages by their text, never by object number, and reports pages added, removed, moved and turned, lines changed, added and removed, comments, highlights and field values, and the title. Impacts state what acceptance does beyond the visible changes: signatures it invalidates, saved revisions it drops.

A shared draft of a PDF holds each revision's steps. PDF anchors are positions, so merging is conservative, as Word's positional merge is: a change to the page order conflicts with any edit on the other branch, deleting a line conflicts with any line edit on that page, and two edits of one place conflict.

## Layers

- L0: `crates/vak-pdf/src/lexer.rs` (tokens, objects, content operations with byte spans, inline images skipped), `crates/vak-pdf/src/object.rs`, `crates/vak-pdf/src/filter.rs` (Flate, LZW, ASCIIHex, ASCII85, RunLength, PNG and TIFF predictors; bounded PNG authoring decode), and `crates/vak-pdf/src/file.rs` (header, classic tables and cross-reference streams through `/Prev`, hybrid files, object streams, a rebuild by scanning).
- L1: `crates/vak-pdf/src/text.rs`, `crates/vak-pdf/src/font.rs` and `crates/vak-pdf/src/content.rs` (encodings, ToUnicode CMaps, fonts, and the interpreter, which also records where each line is and which operations drew it).
- L2: `crates/vak-pdf/src/read.rs` (the anchored read projection, outline, information and inspection) and `crates/vak-pdf/src/projection.rs` (the page-by-page units a client draws).
- L3: `crates/vak-pdf/src/edit.rs` (the op engine), with `crates/vak-pdf/src/write.rs` (the clean whole-file writer) and `crates/vak-pdf/src/layout.rs` (setting new content, with bookmarks for headings).
- L4 and L5: `crates/vak-pdf/src/diff.rs` and `crates/vak-pdf/src/review.rs`.

## Surfaces

- **`doc_read`** (`crates/vak-tools/src/doc_read.rs`): the Office header contract, then the text view (with `section` as `page:3`, `3-5` or a bookmark title), `summary` or `outline`; `table` is refused, since a PDF has no grid.
- **`office_apply`** (`crates/vak-tools/src/office_apply.rs`, `crates/vak-tools/src/office_pdf.rs`): a PDF draft in the call's execution directory in the runtime root (named `.vak/scratch/…`), never the workspace file; the source is checked against `base_digest`.
- **Text tools**: `read`, `edit` and `write` refuse a PDF and name `doc_read` and `office_apply`.
- **Review**: the candidate's review, narrowing and projection endpoints and anchored comments take PDFs (`crates/vak-server/src/lib.rs`), and the lineage keeps a PDF's calls as steps.
- **Shared drafts**: `crates/vak-server/src/office_workspace.rs`; the client's text view edits a PDF line with `replace_paragraph_text`.
- **Preview**: the canvas shows a PDF's pages in the browser's own viewer, and its Text view is the shared document pane (`crates/vak-client-ui/src/components/ArtifactCanvas.tsx`); a citation such as `report.pdf#page:3` opens the pages there.
- **Inbox and channels**: a PDF sent on a channel or dropped in a client is named for `doc_read` (`crates/vak-server/src/inbox.rs`), and an accepted draft goes back to the chat it came from.
- **Verification**: `format.pdf-structure` reads the whole file through the reader (`crates/vak-sandbox/src/lib.rs`).
- **CLI**: `vak office read|apply|diff|verify` take PDFs (`crates/vak/src/office.rs`).

## Deferred until asked

- **Encrypted files.** RC4 and AES need primitives the tree does not carry; they are refused with the reason.
- **Scans.** A page with images and no text layer is named as likely a scan; there is no OCR, and a deleted line removes text, not an image of text.
- **Reading image pixels.** Added images have an extractable alternative-text caption. Existing image contents are not OCR'd or otherwise understood.
- **Layout.** Text keeps content-stream order; multi-column reading order and complex or irregular tables as grids are not reconstructed. Simple repeated aligned columns can be retrieved with `doc_read(view="table")`; uncertain rows remain text.
- **Fonts.** A Type0 font with neither a ToUnicode map nor a Unicode CMap, and a symbol font with no `/Differences`, give no way to turn glyphs into letters. Writing uses the standard Helvetica faces, so only WinAnsi (Latin) text can be written, and a replaced line may not match the document's own font.
- **Deeper edits.** Lines inside form XObjects, true redaction of images, bookmarks for content added to an existing PDF, and page operations on a document with more pages than were read.
- **Signatures, attachments, XFA.** Never made, opened or rendered.

## Tests

`crates/vak-pdf/tests/read.rs` and `crates/vak-pdf/tests/edit.rs` (reading, every op, refusals, the diff, choices and narrowing, the projection, cycles, bombs and corruption sweeps, and repeated headers across authored multi-page tables) and each module's unit tests; `crates/vak-tools/src/office_pdf.rs` and `crates/vak-tools/src/doc_read.rs` (including aligned-table recognition and the paginated table view); `crates/vak-sandbox/src/lib.rs`; the server's PDF review, narrowing and promotion test and room merge rules; the inbox note in `crates/vak-server/src/gateway.rs`; and `crates/vak/tests/office_cli.rs`. Written files were also rendered by the macOS PDF engine to check them outside our own reader.
