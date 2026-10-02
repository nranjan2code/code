// Minimal, injection-safe markdown renderer. Everything is escaped first;
// only a fixed set of constructs produces markup.
//
// This renders the *live/streaming* half of assistant messages; once a turn
// settles, ChatPane swaps to PresentationRenderer.tsx, which renders the
// server's pulldown-cmark-compiled structured document instead. The two are
// kept in feature/behavior parity deliberately (same link/image safety
// policy via safeUrl.ts, same heading-level offset, same GFM surface: bold,
// italic, strikethrough, task lists, nested lists, tables) so a message
// doesn't visibly reflow the moment it settles. If you add a construct here,
// add it to crates/vak-delivery/src/presentation.rs too (and vice versa).

import { safeUrl, isLocalArtifactPath, cleanArtifactPath } from "./safeUrl";
import { parseOfficeCitation } from "./officeFiles";
import { parseMailCalendarCitation } from "./mailCalendarCitation";

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
  out = out.replace(/`([^`\n]+)`/g, (_m, rawCode: string) => {
    const code = rawCode.trim();
    const mailCitation = parseMailCalendarCitation(code);
    if (mailCitation) {
      return `<code class="ic mail-calendar-cite" role="button" tabindex="0" data-mail-account="${esc(mailCitation.accountId)}" data-mail-thread="${esc(mailCitation.threadId)}" data-mail-message="${esc(mailCitation.messageId)}" title="Open cited conversation">${esc(code)}</code>`;
    }
    // A cited place in an Office file opens the file's view there.
    const citation = parseOfficeCitation(code);
    if (citation) {
      return `<code class="ic office-cite" role="button" tabindex="0" data-office-path="${citation.path}" data-anchor="${citation.anchor}" title="Open ${citation.path} at ${citation.anchor}">${code}</code>`;
    }
    const cleanPath = code.replace(/[.,;:!?)]'"`]+$/, "").trim();
    const isDir =
      cleanPath.endsWith("/") ||
      cleanPath === ".vak/scratch" ||
      cleanPath.endsWith("/.vak/scratch") ||
      cleanPath === ".vak";
    const isPath =
      isDir ||
      /^[\w@.-]+(\/[\w@.-]+)+$/.test(cleanPath) ||
      /^\.[\w/-]+$/.test(cleanPath) ||
      /\.\w{1,6}$/.test(cleanPath);
    const isScratch =
      cleanPath === ".vak/scratch" ||
      cleanPath === ".vak/scratch/" ||
      cleanPath.includes(".vak/scratch/");
    const isPreviewable =
      !isDir &&
      isPath &&
      (/\.(html?|xhtml|svg|pdf|png|jpe?g|gif|webp|ico|bmp|csv|tsv)$/i.test(cleanPath) ||
        (isScratch && /\.\w{1,6}$/.test(cleanPath)));
    const title = isDir
      ? (isScratch ? "open in Workbench folder view" : "open in editor")
      : (isPreviewable ? "open in Artifact Canvas" : "open in editor");
    return `<code class="ic"${isPath ? ` data-path="true" data-dir="${isDir ? 'true' : 'false'}" data-scratch="${isScratch ? 'true' : 'false'}" data-previewable="${isPreviewable ? 'true' : 'false'}" data-clean-path="${cleanPath}" title="${title}"` : ""}>${code}</code>`;
  });
  // bold then italic then strikethrough (order matters: bold before italic so
  // `**x**` isn't first read as two adjacent `*x*` italics)
  out = out.replace(/\*\*([^*\n]+?)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/(^|[\s(])\*([^*\n]+?)\*(?=[\s).,;:!?]|$)/g, "$1<em>$2</em>");
  out = out.replace(/~~([^~\n]+?)~~/g, "<del>$1</del>");
  // images (before links — same `![...]` prefix distinguishes them), gated
  // by the same policy PresentationRenderer.tsx uses for media
  // `url` here is already HTML-escaped (entities), which is valid and safe
  // inside a src/href attribute — do not escape it again.
  out = out.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (m, alt: string, url: string) => {
    if (!safeUrl(url, true)) return m;
    return `<img class="md-img" src="${url}" alt="${alt}" loading="lazy" />`;
  });
  // links → real anchors, gated by the same scheme allow-list as the settled
  // (server-compiled) renderer, so behavior doesn't change once a turn settles
  out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, label: string, url: string) => {
    if (safeUrl(url)) {
      return `<a class="lnk" href="${url}" target="_blank" rel="noreferrer noopener">${label}</a>`;
    }
    if (isLocalArtifactPath(url)) {
      const targetPath = cleanArtifactPath(url);
      const isDir =
        targetPath.endsWith("/") ||
        targetPath === ".vak/scratch" ||
        targetPath.endsWith("/.vak/scratch") ||
        targetPath === ".vak";
      const isScratch =
        targetPath === ".vak/scratch" ||
        targetPath === ".vak/scratch/" ||
        targetPath.includes(".vak/scratch/");
      const isPreview =
        !isDir &&
        (/\.(html?|xhtml|svg|pdf|png|jpe?g|gif|webp|ico|bmp|csv|tsv)$/i.test(targetPath) ||
          (isScratch && /\.\w{1,6}$/.test(targetPath)));
      const title = isDir
        ? (isScratch ? "open in Workbench folder view" : "open in editor")
        : (isPreview ? "open in Artifact Canvas" : "open in editor");
      return `<a class="lnk artifact-lnk" data-path="${esc(targetPath)}" data-clean-path="${esc(targetPath)}" data-dir="${isDir ? 'true' : 'false'}" data-scratch="${isScratch ? 'true' : 'false'}" data-previewable="${isPreview ? 'true' : 'false'}" title="${title}" role="button" href="#">${label}</a>`;
    }
    return m;
  });
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
      const langLower = (lang || "").toLowerCase();
      const canPreview =
        langLower === "html" ||
        langLower === "xhtml" ||
        langLower === "svg" ||
        /<!doctype\s+html/i.test(code) ||
        /<html[\s>]/i.test(code) ||
        /<svg[\s>]/i.test(code);
      html += `<div class="cb"><div class="cb-h"><span>${esc(lang || "text")}</span><div style="display:flex;gap:6px;margin-left:auto;">${canPreview ? `<button type="button" class="cb-preview" data-preview="${esc(code)}" data-lang="${esc(lang || "html")}">preview</button>` : ""}<button type="button" class="cb-copy" data-copy="${esc(code)}">copy</button></div></div><pre><code>${esc(code)}</code></pre></div>`;
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

// ---- lists (nested, with GFM task-list checkboxes) -------------------------
//
// Depth is derived from leading-whitespace width, bucketed every 2 columns so
// both 2-space and 4-space indented markdown nest predictably. A stack of
// open `<ul>`/`<ol>` tags lets us open/close levels as indentation changes.

type ListKind = "ul" | "ol";

function listMatch(line: string): { indent: number; kind: ListKind; rest: string } | null {
  const bullet = /^(\s*)[-*]\s+(.*)$/.exec(line);
  if (bullet) return { indent: bullet[1].length, kind: "ul", rest: bullet[2] };
  const ordered = /^(\s*)\d+[.)]\s+(.*)$/.exec(line);
  if (ordered) return { indent: ordered[1].length, kind: "ol", rest: ordered[2] };
  return null;
}

function renderListItem(rest: string): string {
  const task = /^\[( |x|X)\]\s+(.*)$/.exec(rest);
  if (task) {
    const checked = task[1].toLowerCase() === "x";
    return `<li class="md-task"><input type="checkbox" disabled${checked ? " checked" : ""}/>${inline(task[2])}</li>`;
  }
  return `<li>${inline(rest)}</li>`;
}

function block(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  // Stack of currently-open list tags, one entry per nesting depth.
  const listStack: { kind: ListKind; depth: number }[] = [];

  const closeListsDeeperThan = (depth: number | null) => {
    while (
      listStack.length &&
      (depth === null || listStack[listStack.length - 1].depth > depth)
    ) {
      out.push(`</${listStack.pop()!.kind}>`);
    }
  };
  const closeAllLists = () => closeListsDeeperThan(null);

  // Indexed rather than for-of: tables need to look ahead at the separator
  // row before deciding the current line starts a table at all.
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.trimEnd();
    if (isTableRow(line) && i + 1 < lines.length && isTableDivider(lines[i + 1])) {
      closeAllLists();
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
      closeAllLists();
      const lvl = h[1].length;
      out.push(`<h${lvl + 1} class="md-h">${inline(h[2])}</h${lvl + 1}>`);
      continue;
    }
    const li = listMatch(line);
    if (li) {
      const depth = Math.floor(li.indent / 2);
      // Close deeper/mismatched levels, then open new ones down to `depth`.
      closeListsDeeperThan(depth);
      const top = listStack[listStack.length - 1];
      if (!top || top.depth < depth) {
        out.push(li.kind === "ul" ? '<ul class="md-ul">' : '<ol class="md-ol">');
        listStack.push({ kind: li.kind, depth });
      } else if (top.kind !== li.kind) {
        out.push(`</${top.kind}>`);
        listStack.pop();
        out.push(li.kind === "ul" ? '<ul class="md-ul">' : '<ol class="md-ol">');
        listStack.push({ kind: li.kind, depth });
      }
      out.push(renderListItem(li.rest));
      continue;
    }
    if (/^>\s?/.test(line)) {
      closeAllLists();
      out.push(`<blockquote class="md-bq">${inline(line.replace(/^>\s?/, ""))}</blockquote>`);
      continue;
    }
    if (/^\s*(---|\*\*\*)\s*$/.test(line)) {
      closeAllLists();
      out.push('<hr class="md-hr">');
      continue;
    }
    if (line.trim() === "") {
      closeAllLists();
      continue;
    }
    closeAllLists();
    out.push(`<p class="md-p">${inline(line)}</p>`);
  }
  closeAllLists();
  return out.join("");
}
