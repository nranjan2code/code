// Office files and PDFs as the client names them (docs/design/72, 77): which
// paths are Word, Excel, PowerPoint, Visio or PDF files, the shapes of the
// readers' anchors, and a citation of a place, `path#anchor`, the form
// doc_read tells the model to use. An anchor is checked for shape only
// (vak_ooxml::is_anchor, vak_pdf::is_anchor); whether the place exists is
// answered by the view that opens it.

const OFFICE_EXTENSIONS = new Set([
  "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "xlam",
  "pptx", "pptm", "potx", "potm", "ppsx", "ppsm", "ppam",
  "vsdx", "vsdm", "vstx", "vstm", "vssx", "vssm",
]);

export function isOfficePath(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  return OFFICE_EXTENSIONS.has(extension);
}

export type OfficeCitation = { path: string; anchor: string };

const digits = (text: string) => /^[0-9]+$/.test(text);

/** The shapes the reader gives anchors, mirroring `vak_ooxml::is_anchor`. */
export function isAnchor(anchor: string): boolean {
  if (!anchor || anchor.length > 300 || /[\u0000-\u001f]/.test(anchor)) return false;
  const comment = anchor.indexOf("/comment:");
  if (comment >= 0) {
    const paragraph = anchor.slice(0, comment);
    return (paragraph.startsWith("p:") || paragraph.startsWith("p@")) && isAnchor(paragraph) && digits(anchor.slice(comment + 9));
  }
  if (anchor.startsWith("tbl@")) {
    const [table, row, ...extra] = anchor.slice(4).split("/");
    return extra.length === 0 && digits(table) && (row === undefined || (row.startsWith("r") && digits(row.slice(1))));
  }
  if (anchor.startsWith("p:")) return /^[0-9A-Fa-f]{1,8}$/.test(anchor.slice(2));
  if (anchor.startsWith("p@")) return digits(anchor.slice(2));
  for (const prefix of ["slide:", "page:"]) {
    if (!anchor.startsWith(prefix)) continue;
    const [id, ...rest] = anchor.slice(prefix.length).split("/");
    const part = rest.join("/");
    if (!digits(id)) return false;
    if (!part) return true;
    if (part.startsWith("shape:")) return digits(part.slice(6));
    if (prefix === "page:") return false;
    return part === "notes" || /^placeholder:[A-Za-z0-9]+$/.test(part);
  }
  const bang = anchor.lastIndexOf("!");
  if (bang <= 0) return false;
  const sheet = anchor.slice(0, bang);
  const cells = anchor.slice(bang + 1);
  const sheetOk = sheet.startsWith("'") && sheet.endsWith("'") && sheet.length > 2
    ? !sheet.slice(1, -1).replace(/''/g, "").includes("'")
    : /^[\p{L}\p{N}_.]+$/u.test(sheet);
  if (!sheetOk) return false;
  if (!cells) return true;
  const cell = (text: string) => /^[A-Za-z]{1,3}[0-9]+$/.test(text);
  const [first, second, ...extra] = cells.split(":");
  return extra.length === 0 && cell(first) && (second === undefined || cell(second));
}

export function isPdfPath(path: string): boolean {
  return path.split(".").pop()?.toLowerCase() === "pdf";
}

/** A PDF place, `page:3` or `page:3/line:12`, mirroring `vak_pdf::is_anchor`. */
export function isPdfAnchor(anchor: string): boolean {
  return /^page:[1-9][0-9]{0,6}(\/line:[1-9][0-9]{0,6})?$/.test(anchor);
}

/** The page a PDF anchor names, for the viewer's `#page=` fragment. */
export function pdfAnchorPage(anchor: string | undefined): number | null {
  return anchor && isPdfAnchor(anchor) ? Number(anchor.slice(5).split("/")[0]) : null;
}

/** `inbox/deck.pptx#slide:256` or `inbox/scan.pdf#page:3` → its path and
 * anchor; null otherwise. */
export function parseOfficeCitation(text: string): OfficeCitation | null {
  const trimmed = text.trim();
  const hash = trimmed.indexOf("#");
  if (hash <= 0) return null;
  const path = trimmed.slice(0, hash);
  const anchor = trimmed.slice(hash + 1);
  const cited = (isOfficePath(path) && isAnchor(anchor)) || (isPdfPath(path) && isPdfAnchor(anchor));
  return cited ? { path, anchor } : null;
}
