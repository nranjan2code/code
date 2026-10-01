import { createEffect, onCleanup } from "solid-js";
import type { JSX } from "solid-js";
import {
  openInEditor,
  uiPreferences,
  isPreviewableArtifact,
  openArtifactFile,
  openArtifactCanvas,
  openWorkbenchFolder,
  isScratchDirectory,
  openOfficeCitation,
  openMailCalendarCitation,
} from "../store";
import { openFileSmart } from "../App";
import { renderMarkdown } from "../md";
import { highlight, languageForFence } from "../highlight";
import { parseMailCalendarCitation } from "../mailCalendarCitation";

function closeUnclosedFences(src: string): string {
  const lines = src.split("\n");
  let open = false;
  for (const line of lines) {
    if (/^```/.test(line.trim())) {
      open = !open;
    }
  }
  return open ? `${src}\n\`\`\`` : src;
}

async function colorizeCodeBlocks(root: HTMLElement) {
  if (!root || !root.isConnected) return;
  for (const cb of Array.from(root.querySelectorAll<HTMLElement>(".cb"))) {
    if (cb.dataset.hl === "1") continue;
    const code = cb.querySelector("pre > code");
    if (!code) continue;
    const lang = languageForFence(cb.querySelector(".cb-h span")?.textContent ?? "");
    if (!lang) continue;
    // Grammar and the WASM engine are lazy-loaded. A transient import/WASM
    // delay must not permanently strand a code block in monochrome fallback.
    let out = await highlight(code.textContent ?? "", lang);
    if (!out) {
      await new Promise((resolve) => window.setTimeout(resolve, 120));
      out = await highlight(code.textContent ?? "", lang);
    }
    if (!out || !cb.isConnected) continue;
    const pre = cb.querySelector("pre");
    if (!pre) continue;
    pre.outerHTML = out;
    cb.dataset.hl = "1";
  }
}

export default function MarkdownView(props: { text: string; streaming?: boolean }): JSX.Element {
  let el!: HTMLDivElement;
  let hlTimer: number | null = null;

  createEffect(() => {
    void uiPreferences.theme;
    const raw = props.text;
    if (!el) return;
    const textToRender = props.streaming ? closeUnclosedFences(raw) : raw;
    el.innerHTML = renderMarkdown(textToRender);

    if (props.streaming) {
      if (hlTimer !== null) clearTimeout(hlTimer);
      hlTimer = window.setTimeout(() => {
        hlTimer = null;
        void colorizeCodeBlocks(el);
      }, 400);
    } else {
      if (hlTimer !== null) {
        clearTimeout(hlTimer);
        hlTimer = null;
      }
      void colorizeCodeBlocks(el);
    }
  });

  onCleanup(() => {
    if (hlTimer !== null) clearTimeout(hlTimer);
  });

  const onClick = (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    if (target.classList.contains("cb-preview")) {
      const codeContent = target.getAttribute("data-preview") ?? "";
      const lang = target.getAttribute("data-lang") ?? "html";
      openArtifactCanvas({ kind: "inline", title: `${lang.toUpperCase()} Preview`, html: codeContent });
      return;
    }
    if (target.classList.contains("cb-copy")) {
      void navigator.clipboard.writeText(target.getAttribute("data-copy") ?? "")
        .then(() => {
          target.textContent = "copied";
          setTimeout(() => (target.textContent = "copy"), 900);
        })
        .catch(() => {
          target.textContent = "copy failed";
          setTimeout(() => (target.textContent = "copy"), 1400);
        });
      return;
    }
    const cite = target.closest("code.office-cite[data-office-path]");
    if (cite) {
      openOfficeCitation({ path: cite.getAttribute("data-office-path") ?? "", anchor: cite.getAttribute("data-anchor") ?? "" });
      return;
    }
    const mailCite = target.closest("code.mail-calendar-cite[data-mail-account]");
    if (mailCite) {
      const citation = parseMailCalendarCitation(`mailcite:${encodeURIComponent(mailCite.getAttribute("data-mail-account") ?? "")}/${encodeURIComponent(mailCite.getAttribute("data-mail-thread") ?? "")}/${encodeURIComponent(mailCite.getAttribute("data-mail-message") ?? "")}`);
      if (citation) openMailCalendarCitation(citation);
      return;
    }
    const link = target.closest("a.artifact-lnk[data-path]");
    if (link) {
      event.preventDefault();
      const p = link.getAttribute("data-clean-path") || (link.getAttribute("data-path") ?? "").trim();
      const isDir = link.getAttribute("data-dir") === "true";
      const isScratch = link.getAttribute("data-scratch") === "true" || isScratchDirectory(p);
      if (p) {
        if (isScratch || (isDir && p.includes(".vak/scratch"))) {
          openWorkbenchFolder(p);
        } else if (!isDir && isPreviewableArtifact(p)) {
          openArtifactFile(p);
        } else {
          void openFileSmart(p);
        }
      }
      return;
    }
    const code = target.closest("code.ic[data-path]");
    if (code) {
      const p = code.getAttribute("data-clean-path") || (code.textContent ?? "").trim().replace(/[.,;:!?)]'"`]+$/, "").trim();
      const isDir = code.getAttribute("data-dir") === "true";
      const isScratch = code.getAttribute("data-scratch") === "true" || isScratchDirectory(p);
      if (p) {
        if (isScratch || (isDir && p.includes(".vak/scratch"))) {
          openWorkbenchFolder(p);
        } else if (!isDir && isPreviewableArtifact(p)) {
          openArtifactFile(p);
        } else {
          void openFileSmart(p);
        }
      }
    }
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    const target = event.target as HTMLElement;
    const mailCite = target.closest("code.mail-calendar-cite[data-mail-account]");
    if (!mailCite) return;
    event.preventDefault();
    const citation = parseMailCalendarCitation(`mailcite:${encodeURIComponent(mailCite.getAttribute("data-mail-account") ?? "")}/${encodeURIComponent(mailCite.getAttribute("data-mail-thread") ?? "")}/${encodeURIComponent(mailCite.getAttribute("data-mail-message") ?? "")}`);
    if (citation) openMailCalendarCitation(citation);
  };

  return <div class="md" classList={{ streaming: !!props.streaming }} ref={el} onClick={onClick} onKeyDown={onKeyDown} />;
}
