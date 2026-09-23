export type DelimitedPreview = {
  headers: string[];
  rows: string[][];
  totalRows: number;
  truncated: boolean;
};

/** Parse enough of a saved CSV/TSV to inspect it without building an unbounded table DOM. */
export function parseDelimitedPreview(source: string, delimiter: "," | "\t", limit = 200): DelimitedPreview {
  const text = source.startsWith("\uFEFF") ? source.slice(1) : source;
  const records: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let afterQuote = false;
  let fieldStart = true;
  let totalRows = 0;
  let headers: string[] | null = null;

  const endField = () => {
    row.push(field);
    field = "";
    fieldStart = true;
    afterQuote = false;
  };
  const endRow = () => {
    endField();
    if (!headers) {
      headers = row;
      if (headers.length === 0 || headers.every((cell) => cell === "")) throw new Error("This data file has no header columns.");
    } else {
      if (row.length !== headers.length) throw new Error(`Row ${totalRows + 1} has ${row.length} columns; the header has ${headers.length}. Open Source to inspect it.`);
      totalRows += 1;
      if (records.length < limit) records.push(row);
    }
    row = [];
  };

  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (quoted) {
      if (char === '"') {
        if (text[index + 1] === '"') { field += '"'; index += 1; }
        else { quoted = false; afterQuote = true; }
      } else field += char;
      continue;
    }
    if (afterQuote && char !== delimiter && char !== "\n" && char !== "\r") {
      throw new Error(`Unexpected text after a closing quote near row ${totalRows + 1}. Open Source to inspect it.`);
    }
    if (char === delimiter) { endField(); continue; }
    if (char === "\n" || char === "\r") {
      if (char === "\r" && text[index + 1] === "\n") index += 1;
      endRow();
      continue;
    }
    if (char === '"') {
      if (!fieldStart) throw new Error(`Unexpected quote near row ${totalRows + 1}. Open Source to inspect it.`);
      quoted = true;
      fieldStart = false;
      continue;
    }
    field += char;
    fieldStart = false;
  }
  if (quoted) throw new Error("This data file ends inside a quoted value. Open Source to inspect it.");
  if (row.length > 0 || field !== "" || afterQuote || (!headers && text.length > 0 && !/[\r\n]$/.test(text))) endRow();
  if (!headers) throw new Error("This data file has no header columns.");
  return { headers, rows: records, totalRows, truncated: totalRows > records.length };
}
