import { createEffect, Show } from "solid-js";
import type { JSX } from "solid-js";
import { openInEditor } from "../store";
import { renderMarkdown } from "../md";
import { highlight, languageForFence } from "../highlight";

async function colorizeCodeBlocks(root: HTMLElement) {
  for (const cb of Array.from(root.querySelectorAll<HTMLElement>(".cb"))) {
    if (cb.dataset.hl === "1") continue;
    const code = cb.querySelector("pre > code");
    if (!code) continue;
    const lang = languageForFence(cb.querySelector(".cb-h span")?.textContent ?? "");
    if (!lang) continue;
    const out = await highlight(code.textContent ?? "", lang);
    if (!out) continue;
    const pre = cb.querySelector("pre");
    if (!pre) continue;
    pre.outerHTML = out;
    cb.dataset.hl = "1";
  }
}

export default function MarkdownView(props: { text: string; streaming?: boolean }): JSX.Element {
  let el!: HTMLDivElement;
  createEffect(() => {
    if (props.streaming) return;
    el.innerHTML = renderMarkdown(props.text);
    void colorizeCodeBlocks(el);
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
  return (
    <Show when={!props.streaming} fallback={<div class="md streaming semantic-stream-text">{props.text}</div>}>
      <div class="md" ref={el} onClick={onClick} />
    </Show>
  );
}
