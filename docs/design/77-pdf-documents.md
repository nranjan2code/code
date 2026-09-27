# 77 — PDF documents: the read-only reader

Status: **shipped 2026-09-27** (the reader and every surface below). Read-only by design; what is not built is listed under "Deferred until asked" with the reason.

## Owner direction

**2026-09-27.** Add PDF beside the Open XML pipeline as its own crate, wired the way `crates/vak-ooxml` is, and write it fresh rather than as a wrapper around a PDF library.

## Why

A PDF is the document people most often send an Agent. Before this, a PDF that arrived on a channel or was dropped in a client was saved to the inbox with the note that no reader understood it, `doc_read` refused it as not text, and the only PDF code in the tree was a structure check in `crates/vak-sandbox/src/lib.rs` built on a third-party library.

## Decisions

- **D1 — our own parser.** `crates/vak-pdf` is written from ISO 32000-2 with no PDF library beneath it and no other vak crate as a dependency, so it can be tested and fuzzed alone, like `crates/vak-ooxml`. Its one outside dependency is `flate2` for zlib, pinned and already in the tree through `zip`. The old verifier moved onto it and `lopdf` left the workspace: one PDF parser, not two (invariant 30).
- **D2 — worker only.** Every vak call site runs the reader inside the broker worker (invariant 14): `doc_read` is a worker tool, the verifier runs in `VerifyTargets`, and `vak pdf read` is the `PdfRead` worker task. A defect in the reader on some input is caught and reported as a damaged file, never a crash of the worker.
- **D3 — bounded.** `vak_pdf::Limits` caps the file (128 MiB), the object count, each decoded stream (64 MiB) and the whole document's decoding (512 MiB, counted as bytes produced), pages read (2,000), lines per page, line length, form nesting, bookmarks and annotations; every nesting depth and reference chain is bounded, and cycles in the page tree, outline, forms and name trees are visited once.
- **D4 — nothing runs or is followed.** JavaScript, automatic, launch, submit and remote actions, embedded files, XFA forms, rich media and links are counted and named in the flags. Signatures are counted, never verified; the flag says so.
- **D5 — labelled text.** Content is data. A line of invisible text (rendering mode 3 or 7, often a scan's recognised-text layer), white text, tiny text (under 1pt) and text placed off the page carries a label, so a hidden instruction never reads as body text. Comments and form fields come after a page's text, labelled with their kind and author; a hidden annotation says so.
- **D6 — anchors.** Every line is `page:<n>/line:<m>`, both counted from 1 as a viewer counts pages, valid for the file's digest. `vak_pdf::is_anchor` checks the shape.
- **D7 — detection by bytes.** `doc_read` routes a file named `.pdf`, or any file that starts `%PDF-`, to the reader, and a file named `.pdf` that is not one fails with that reason.
- **D8 — refusal over half-reading.** An encrypted PDF is refused whole, even one that opens without a password, as an encrypted Office package is: decryption is not implemented.

## Layers

- L0: `crates/vak-pdf/src/lexer.rs` (tokens, objects, content operations, inline images skipped), `crates/vak-pdf/src/object.rs`, `crates/vak-pdf/src/filter.rs` (Flate, LZW, ASCIIHex, ASCII85, RunLength, PNG and TIFF predictors; image codecs never decoded), and `crates/vak-pdf/src/file.rs` (header, classic tables and cross-reference streams through `/Prev`, hybrid files, object streams, and a rebuild by scanning when the table is unusable).
- L1: `crates/vak-pdf/src/text.rs` (WinAnsi, MacRoman, Standard and PDFDoc encodings, glyph names, text strings, ToUnicode CMaps with byte-wise code spaces), `crates/vak-pdf/src/font.rs` (simple, Type3 and Type0 fonts: code lengths, text per code, widths), and `crates/vak-pdf/src/content.rs` (the interpreter: graphics state, text matrices, fill colour, rendering mode, form XObjects; runs on one baseline with the same labels join into a line, a `TJ` adjustment of a fifth of an em or more is a word space).
- L2: `crates/vak-pdf/src/read.rs` (pages as anchored lines, outline with destinations and named destinations, document information, the inspection and its flags, sections by page range or bookmark, and what was not read).

## Surfaces

- **`doc_read`** (`crates/vak-tools/src/doc_read.rs`): the Office header contract — kind and version, title, sha256, counts, `Flags:`, `Not read:`, the data-not-instructions line, and how to cite a place — then the text view (with `section` as `page:3`, `3-5` or a bookmark title), `summary` (information, outline, links) or `outline`. `table` is refused: a PDF has no grid to show.
- **Text tools**: `read`, `edit` and `write` refuse a PDF and point at `doc_read` (`crates/vak-tools/src/office_apply.rs`, `text_tool_refusal`).
- **Inbox**: a PDF sent on a channel or dropped in a client is named for `doc_read` (`crates/vak-server/src/inbox.rs`).
- **Verification**: `format.pdf-structure` reads the whole file through the reader and reports its counts, flags and what was not read (`crates/vak-sandbox/src/lib.rs`).
- **CLI**: `vak pdf read <file> [--from N | --at page:N | --facts]` and `vak pdf verify <file>` print the worker's JSON (`crates/vak/src/pdf.rs`).
- **Citations**: `path.pdf#page:3` in backticks opens the client's PDF view at that page (`crates/vak-client-ui/src/officeFiles.ts`).

## Deferred until asked

- **Encrypted files.** RC4 and AES decryption need primitives the tree does not carry; they are refused with the reason.
- **Scans.** A page with images and no text layer is named as likely a scan; there is no OCR.
- **Layout.** Text keeps content-stream order, which is reading order for most generated files; multi-column reading order and tables as grids are not reconstructed.
- **Fonts.** A Type0 font with neither a ToUnicode map nor a Unicode CMap, and a symbol font with no `/Differences`, give no way to turn glyphs into letters; their text is named under "Not read". The standard 14 fonts use estimated widths, which only affects where spaces are inferred between separately placed strings.
- **Editing, review, signatures, attachments.** The reader never writes a PDF, never verifies a signature and never opens an embedded file.
- **XFA.** Only the PDF pages are read; the XFA layer is flagged.

## Tests

`crates/vak-pdf/tests/read.rs` (anchors and labels, the inspection, object and cross-reference streams with a Type0 font, a rebuilt table, refusals, a decompression bomb, cycles and self-drawing forms, truncation and corruption sweeps), the unit tests in each `crates/vak-pdf/src` module, `crates/vak-tools/src/doc_read.rs`, `crates/vak-sandbox/src/lib.rs`, `crates/vak-server/src/gateway.rs` (the inbox note), and `crates/vak/tests/pdf_cli.rs`.
