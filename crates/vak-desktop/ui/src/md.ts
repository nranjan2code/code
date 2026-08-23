// Minimal, injection-safe markdown renderer. Everything is escaped first;
// only a fixed set of constructs produces markup.

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function inline(s: string): string {
  let out = esc(s);
  // inline code; path-looking spans become clickable editor links
  out = out.replace(/`([^`\n]+)`/g, (_m, code: string) => {
    const isPath =
      /^[\w@.-]+(\/[\w@.-]+)+$/.test(code) || /^\.[\w/-]+$/.test(code) || /\.\w{1,6}$/.test(code);
    return `<code class="ic"${isPath ? ' data-path="true" title="open in editor"' : ""}>${code}</code>`;
  });
  // bold then italic
  out = out.replace(/\*\*([^*\n][^*\n]*?)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/(^|[\s(])\*([^*\n]+)\*(?=[\s).,;:!?]|$)/g, "$1<em>$2</em>");
  // links → non-navigating styled span (external nav needs the opener plugin)
  out = out.replace(
    /\[([^\]]+)\]\((https?:[^)\s]+)\)/g,
    '<span class="lnk" title="$2">$1</span>',
  );
  return out;
}

export function renderMarkdown(src: string): string {
  const parts = src.split(/^```(\w*)[ \t]*$\n?([\s\S]*?)^```[ \t]*$/gm);
  if (parts.length === 1) return block(src);

  let html = "";
  for (let i = 0; i < parts.length; i++) {
    if (i % 3 === 0) html += block(parts[i]);
    else if (i % 3 === 1) continue; // lang captured below
    else {
      const lang = parts[i - 1] || "";
      const code = parts[i].replace(/\n$/, "");
      html += `<div class="cb"><div class="cb-h"><span>${esc(lang || "text")}</span><button class="cb-copy" data-copy="${esc(code)}">copy</button></div><pre><code>${esc(code)}</code></pre></div>`;
    }
  }
  return html;
}


// ---- GitHub-flavoured tables ------------------------------------------------
//
// A table is a row, a divider, then rows. The divider is what distinguishes it
// from prose that happens to contain a pipe, so it is required.

type Align = "left" | "center" | "right";

function isTableRow(line: string): boolean {
  return line.includes("|") && line.trim() !== "";
}

function isTableDivider(line: string): boolean {
  const t = line.trim();
  if (!t.includes("-") || !t.includes("|")) return false;
  return /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/.test(t);
}

/** Split a row on unescaped pipes, dropping the optional outer delimiters. */
function splitRow(line: string): string[] {
  let t = line.trim();
  if (t.startsWith("|")) t = t.slice(1);
  if (t.endsWith("|") && !t.endsWith("\\|")) t = t.slice(0, -1);
  const cells: string[] = [];
  let cur = "";
  for (let i = 0; i < t.length; i++) {
    if (t[i] === "\\" && t[i + 1] === "|") {
      cur += "|";
      i++;
    } else if (t[i] === "|") {
      cells.push(cur.trim());
      cur = "";
    } else {
      cur += t[i];
    }
  }
  cells.push(cur.trim());
  return cells;
}

function parseAlignments(divider: string): Align[] {
  return splitRow(divider).map((c) => {
    const left = c.startsWith(":");
    const right = c.endsWith(":");
    if (left && right) return "center";
    if (right) return "right";
    return "left";
  });
}

function renderTable(header: string[], aligns: Align[], body: string[][]): string {
  const cols = header.length;
  const cell = (text: string, idx: number, tag: "th" | "td") => {
    const a = aligns[idx] ?? "left";
    const style = a === "left" ? "" : ` style="text-align:${a}"`;
    return `<${tag}${style}>${inline(text)}</${tag}>`;
  };
  const head = `<thead><tr>${header.map((c, i) => cell(c, i, "th")).join("")}</tr></thead>`;
  const rows = body
    .map((r) => {
      // Ragged rows are common in hand-written markdown; pad or trim to the
      // header width so the table never renders lopsided.
      const cells = r.slice(0, cols);
      while (cells.length < cols) cells.push("");
      return `<tr>${cells.map((c, i) => cell(c, i, "td")).join("")}</tr>`;
    })
    .join("");
  return `<div class="md-tablewrap"><table class="md-table">${head}<tbody>${rows}</tbody></table></div>`;
}

function block(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  let list: "ul" | "ol" | null = null;

  const closeList = () => {
    if (list) {
      out.push(`</${list}>`);
      list = null;
    }
  };

  // Indexed rather than for-of: tables need to look ahead at the separator
  // row before deciding the current line starts a table at all.
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.trimEnd();
    if (isTableRow(line) && i + 1 < lines.length && isTableDivider(lines[i + 1])) {
      closeList();
      const aligns = parseAlignments(lines[i + 1]);
      const header = splitRow(line);
      const body: string[][] = [];
      let j = i + 2;
      while (j < lines.length && isTableRow(lines[j])) {
        body.push(splitRow(lines[j]));
        j++;
      }
      out.push(renderTable(header, aligns, body));
      i = j - 1;
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) {
      closeList();
      const lvl = h[1].length;
      out.push(`<h${lvl + 1} class="md-h">${inline(h[2])}</h${lvl + 1}>`);
      continue;
    }
    if (/^\s*[-*]\s+/.test(line)) {
      if (list !== "ul") {
        closeList();
        out.push('<ul class="md-ul">');
        list = "ul";
      }
      out.push(`<li>${inline(line.replace(/^\s*[-*]\s+/, ""))}</li>`);
      continue;
    }
    if (/^\s*\d+[.)]\s+/.test(line)) {
      if (list !== "ol") {
        closeList();
        out.push('<ol class="md-ol">');
        list = "ol";
      }
      out.push(`<li>${inline(line.replace(/^\s*\d+[.)]\s+/, ""))}</li>`);
      continue;
    }
    if (/^>\s?/.test(line)) {
      closeList();
      out.push(`<blockquote class="md-bq">${inline(line.replace(/^>\s?/, ""))}</blockquote>`);
      continue;
    }
    if (/^\s*(---|\*\*\*)\s*$/.test(line)) {
      closeList();
      out.push('<hr class="md-hr">');
      continue;
    }
    if (line.trim() === "") {
      closeList();
      continue;
    }
    closeList();
    out.push(`<p class="md-p">${inline(line)}</p>`);
  }
  closeList();
  return out.join("");
}
