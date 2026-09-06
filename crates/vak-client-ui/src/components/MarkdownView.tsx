import { createEffect, onCleanup } from "solid-js";
import type { JSX } from "solid-js";
import { openInEditor, uiPreferences } from "../store";
import { renderMarkdown } from "../md";
import { highlight, languageForFence } from "../highlight";

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
    const out = await highlight(code.textContent ?? "", lang);
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
    if (target.classList.contains("cb-copy")) {
      void navigator.clipboard.writeText(target.getAttribute("data-copy") ?? "");
      target.textContent = "copied";
      setTimeout(() => (target.textContent = "copy"), 900);
      return;
    }
    const code = target.closest("code.ic[data-path]");
    if (code && /^[\w@.-]+(\/[\w@.-]+)+$|^\.[\w/-]+$/.test(code.textContent ?? "")) {
      openInEditor(code.textContent ?? "");
    }
  };

  return <div class="md" classList={{ streaming: !!props.streaming }} ref={el} onClick={onClick} />;
}

