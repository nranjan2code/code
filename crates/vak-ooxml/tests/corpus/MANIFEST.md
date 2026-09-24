# Open XML test corpus: provenance manifest

docs/design/72-openxml-documents.md, P0. Every file in this directory is
listed here with where it came from. Only self-authored files are allowed.

## Generated fixtures (in code, not files)

`src/fixtures.rs` builds these at test time. They are synthetic packages
written by hand from the ECMA-376 and MS-VSDX schemas. They prove the reader
and writer against known shapes; they do not prove compatibility with files
the real applications write.

| Fixture | Covers |
|---|---|
| `docx()` | Title and heading styles by built-in name, `w14:paraId`, tracked insertion and deletion, hidden run, DDE field, table, comment, a tab-stop definition that must not become text |
| `xlsx()` | Shared strings with a phonetic run, inline string, boolean, formula with cached value, hidden sheet, defined name |
| `pptx()` | Title placeholder, off-slide shape, speaker notes, hidden slide |
| `vsdx()` | One page, shapes with text and an inline `cp` element |
| adversarial (in `tests/package.rs`) | CFB container, non-ZIP, traversal and absolute names, case-insensitive duplicate, declared-size bomb, lying directory size, DOCTYPE entity expansion, deep nesting, escaping relationship target, remote template, macro package renamed to `.docx`, signature origin, sensitivity label, Strict relationship namespace, missing content types, `.xlsb` |

## Application-authored files

None yet. Files saved by Word, Excel, PowerPoint and Visio (macOS and
Windows), LibreOffice and a Google Workspace export are added here as they
are authored, each with application, version, platform and date. Preserve
cells in the support matrix wait for them.
