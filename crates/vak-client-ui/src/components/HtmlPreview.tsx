import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { previewSandbox, sandboxedSrcdoc } from "../safeUrl";
import { sendPreviewTheme, themedPreview } from "../previewTheme";

/** One document, with a human-owned, temporary permission to run its code.
 * Locality describes where bytes live, never who authored or audited them. */
export default function HtmlPreview(props: {
  html: string;
  title: string;
  sourceLabel: string;
  page: string;
  origin?: string;
  prepare?: () => Promise<string>;
  localFiles?: boolean;
  libraries?: readonly string[];
  hidden?: boolean;
  frameClass: string;
  viewport?: (frame: JSX.Element) => JSX.Element;
  permissionKey?: unknown;
  onState?: (running: boolean) => void;
}) {
  const [running, setRunning] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal("");
  const [interactive, setInteractive] = createSignal("");
  let generation = 0;
  onCleanup(() => { generation++; props.onState?.(false); });
  const sources = createMemo(() => {
    const doc = new DOMParser().parseFromString(props.html, "text/html");
    let embedded = 0;
    let handlers = 0;
    const local: string[] = [];
    const external: string[] = [];
    for (const script of doc.querySelectorAll("script")) {
      const type = (script.getAttribute("type") ?? "").trim().toLowerCase();
      if (type && type !== "module" && !/^(text|application)\/(java|ecma)script$/.test(type)) continue;
      if (!script.hasAttribute("src")) { embedded++; continue; }
      const src = script.getAttribute("src") ?? "";
      // Scheme-relative and backslash URLs are external too. Absolute local
      // paths cannot be bundled by the confined reader, but stay visibly local.
      (/^(?:[a-z][a-z\d+.-]*:|[/\\]{2})/i.test(src.trim()) ? external : local).push(src);
    }
    for (const element of doc.querySelectorAll("*")) {
      handlers += [...element.attributes].filter(a => /^on/i.test(a.name) || /^(href|action)$/i.test(a.name) && /^\s*javascript:/i.test(a.value)).length;
    }
    return { embedded, handlers, local, external };
  });
  const hasCode = () => sources().embedded + sources().handlers + sources().local.length + sources().external.length + (props.libraries?.length ?? 0) > 0;
  // A changed document revokes permission even if its viewer stays mounted.
  createEffect(() => {
    props.html; props.page; props.origin; props.permissionKey; props.libraries?.join("\0");
    generation++;
    setRunning(false); setInteractive(""); setBusy(false); setProblem("");
  });
  createEffect(() => props.onState?.(running()));
  const stop = () => { generation++; setRunning(false); setInteractive(""); setBusy(false); };
  const run = async () => {
    const mine = ++generation;
    setBusy(true); setProblem("");
    try {
      const html = props.prepare ? await props.prepare() : props.html;
      if (mine !== generation) return;
      setInteractive(sandboxedSrcdoc(html, "interactive"));
      setRunning(true);
    } catch {
      if (mine === generation) setProblem("The page’s local files could not be loaded. Its code has stayed paused.");
    } finally { if (mine === generation) setBusy(false); }
  };
  const frame = () => <Show when={running()} fallback={
    <Show when={props.origin} fallback={<iframe class={props.frameClass} title={props.title} sandbox={previewSandbox("static")} srcdoc={themedPreview(props.page)} />}>
      {(url) => {
        let iframe: HTMLIFrameElement | undefined;
        createEffect(() => sendPreviewTheme(iframe, url()));
        return <iframe ref={iframe} class={props.frameClass} title={props.title} sandbox={previewSandbox("origin")} src={url()} onLoad={() => sendPreviewTheme(iframe, url())} />;
      }}
    </Show>
  }><iframe class={props.frameClass} title={props.title} sandbox={previewSandbox("interactive")} srcdoc={themedPreview(interactive())} /></Show>;

  return <div class="html-preview" hidden={props.hidden}>
    <div class="html-preview-permissions">
      <span role="status">{running() ? "Page code running · network access possible" : hasCode() ? "Page code paused · offline preview" : "Offline preview · no page scripts found"}</span>
      <Show when={hasCode() && !running()}><p class="html-preview-paused">Interactive features are paused. Read Preview permissions before choosing whether to run this page.</p></Show>
      <details>
        <summary>Preview permissions</summary>
        <p>{props.sourceLabel}. Where code is stored does not establish who wrote it.</p>
        <p>{sources().embedded} embedded scripts · {sources().handlers} inline actions · {sources().local.length} local script files · {sources().external.length} external script references</p>
        <Show when={props.libraries?.length}><p>Requested built-in libraries: {props.libraries?.join(", ")}. Available libraries are bundled with Vakyartha; nothing is downloaded.</p></Show>
        <For each={sources().local}>{src => <p class="html-preview-source">Local file: {src} — {props.localFiles ? "loaded through this file’s reader when you run" : "unavailable in this saved document"}</p>}</For>
        <For each={sources().external}>{src => <p class="html-preview-source">External reference: {src} — blocked</p>}</For>
        <p>Running allows embedded code{props.localFiles ? " and local files bundled with this page" : " only"} in an isolated frame. External script downloads, ordinary web requests and access to Vakyartha’s data stay blocked. Browser features such as WebRTC can still make network connections. This list shows declared references; code can construct other addresses.</p>
        <Show when={running()} fallback={<Show when={hasCode()}><button type="button" class="btn sm" disabled={busy()} onClick={() => void run()}>{busy() ? "Loading page code…" : "Run page code"}</button></Show>}>
          <button type="button" class="btn sm" onClick={stop}>Stop page code</button>
        </Show>
        <p>Permission ends when you stop, reload, switch files or close this preview. A separate window opens with code paused.</p>
        <Show when={problem()}><p role="alert">{problem()}</p></Show>
      </details>
    </div>
    {props.viewport ? props.viewport(frame()) : frame()}
  </div>;
}
