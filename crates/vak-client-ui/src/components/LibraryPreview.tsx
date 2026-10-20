import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show, Switch, Match } from "solid-js";
import * as api from "../api";
import { parseDelimitedPreview } from "../delimitedPreview";
import { previewSandbox, sandboxedSrcdoc } from "../safeUrl";
import { themedPreview } from "../previewTheme";
import MarkdownView from "./MarkdownView";
import OfficeView from "./OfficeView";

/** A saved version is the only source here. Never substitute the current
 * workspace file for an older version, or let generated HTML into the app. */
export default function LibraryPreview(props: { artifact: api.ArtifactDetail; version: string }) {
  const extension = () => props.artifact.path.split(".").pop()?.toLowerCase() ?? "";
  const format = () => {
    const ext = extension();
    if (/^(docx|docm|dotx|xlsx|xlsm|xltx|pptx|pptm|potx|vsdx|pdf)$/.test(ext)) return "office";
    if (/^(png|jpe?g|gif|webp|svg|ico|bmp|avif)$/.test(ext)) return "image";
    if (/^(mp3|wav|ogg|m4a|aac|flac)$/.test(ext)) return "audio";
    if (/^(mp4|webm|mov|m4v)$/.test(ext)) return "video";
    if (/^(html?|xhtml)$/.test(ext)) return "html";
    if (/^(md|markdown)$/.test(ext)) return "markdown";
    if (/^(csv|tsv)$/.test(ext)) return "table";
    if (/^(txt|json|ya?ml|toml|css|[cm]?js|jsx|ts|tsx|py|rs|go|sh|sql|xml|ini|log|diff|patch)$/.test(ext)) return "text";
    return "download";
  };
  const [source, setSource] = createSignal(false);
  const [reload, setReload] = createSignal(0);
  const identity = createMemo(() => `${props.artifact.id}:${props.version}:${reload()}`);
  const [content] = createResource(
    identity,
    async () => {
      const { id } = props.artifact;
      const version = props.version;
      const kind = format();
      if (kind === "office" || kind === "download") return null;
      const bytes = await api.libraryVersionBytes(id, version);
      if (["image", "audio", "video"].includes(kind)) {
        const mime: Record<string, string> = { svg: "image/svg+xml", png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", avif: "image/avif", ico: "image/x-icon", bmp: "image/bmp", mp3: "audio/mpeg", wav: "audio/wav", ogg: "audio/ogg", m4a: "audio/mp4", aac: "audio/aac", flac: "audio/flac", mp4: "video/mp4", webm: "video/webm", mov: "video/quicktime", m4v: "video/mp4" };
        return { blob: new Blob([bytes], { type: mime[extension()] }), text: "", table: null, warning: null as string | null };
      }
      if (bytes.length > 2 * 1024 * 1024) throw new Error("This file is too large for an inline preview. Download it to read the full file.");
      const text = new TextDecoder().decode(bytes);
      let table = null;
      let warning: string | null = null;
      if (kind === "table") {
        try { table = parseDelimitedPreview(text, extension() === "tsv" ? "\t" : ","); }
        catch (error) { warning = error instanceof Error ? error.message : "This table could not be read. Open Source to inspect it."; }
      }
      if (kind === "html") {
        const doc = new DOMParser().parseFromString(text, "text/html");
        if ([...doc.querySelectorAll("[src], link[rel=stylesheet]")].some(el => {
          const value = el.getAttribute("src") ?? el.getAttribute("href") ?? "";
          return value && !/^(data:|blob:)/i.test(value);
        })) warning = "Linked images, scripts or styles may be unavailable in this saved preview.";
      }
      return { blob: null, text, table, warning };
    },
  );
  const mediaUrl = createMemo(() => {
    if (content.error || content.loading) return "";
    const blob = content()?.blob;
    return blob ? URL.createObjectURL(blob) : "";
  });
  createEffect(() => {
    const url = mediaUrl();
    onCleanup(() => { if (url) URL.revokeObjectURL(url); });
  });
  const shown = () => content();
  const hasSource = () => ["html", "markdown", "table"].includes(format());
  const table = () => shown()?.table;

  return (
    <section class="library-content" aria-label="File preview">
      <Show when={hasSource()}>
        <div class="library-preview-tools" role="group" aria-label="File view">
          <button class="btn sm" aria-pressed={!source()} onClick={() => setSource(false)}>Preview</button>
          <button class="btn sm" aria-pressed={source()} onClick={() => setSource(true)}>Source</button>
          <Show when={format() === "html"}><span class="library-note">Interactive preview · network access blocked</span></Show>
        </div>
      </Show>
      <Show when={!content.error} fallback={<div class="library-preview-error" role="alert"><p>{content.error instanceof Error ? content.error.message : "This preview could not be opened. Try again or download this version."}</p><button class="btn sm" onClick={() => setReload(n => n + 1)}>Try again</button></div>}>
        <Switch>
          <Match when={format() === "office"}>
            <div class="library-preview library-document"><OfficeView source={{ path: props.artifact.path, version: { artifact: props.artifact.id, version: props.version } }} fileName={props.artifact.path.split("/").pop() ?? props.artifact.name} /></div>
          </Match>
          <Match when={format() === "download"}><div class="library-preview dock-empty">A preview isn’t available for this file type. Download this version to open it in its app.</div></Match>
          <Match when={!content.loading && shown()}>
            {(loaded) => <Switch>
              <Match when={format() === "html"}>
                <Show when={loaded().warning && !source()}><p class="library-note" role="status">{loaded().warning}</p></Show>
                <iframe class="library-html-frame" hidden={source()} title={props.artifact.name} sandbox={previewSandbox("static")} srcdoc={themedPreview(sandboxedSrcdoc(loaded().text))} />
                <Show when={source()}><pre class="library-preview library-source">{loaded().text}</pre></Show>
              </Match>
              <Match when={source() && hasSource() || format() === "text"}><pre class="library-preview library-source">{loaded().text}</pre></Match>

              <Match when={format() === "markdown"}><div class="library-preview library-reading"><MarkdownView text={loaded().text} /></div></Match>
              <Match when={format() === "table" && loaded().warning}><p class="library-preview-error" role="status">{loaded().warning}</p></Match>
              <Match when={format() === "table" && table()}>{(data) => <div class="library-preview library-data"><p class="library-note">{data().totalRows} rows · {data().headers.length} columns{data().truncated ? ` · showing first ${data().rows.length}` : ""}</p><div class="library-table-scroll"><table><thead><tr><For each={data().headers}>{(header, i) => <th scope="col">{header || `Column ${i() + 1}`}</th>}</For></tr></thead><tbody><For each={data().rows}>{row => <tr><For each={row}>{cell => <td>{cell}</td>}</For></tr>}</For></tbody></table></div></div>}</Match>
              <Match when={format() === "image"}><div class="library-preview library-media"><img src={mediaUrl()} alt={props.artifact.name} /></div></Match>
              <Match when={format() === "audio"}><div class="library-preview library-media"><audio controls preload="metadata" src={mediaUrl()} aria-label={props.artifact.name} /></div></Match>
              <Match when={format() === "video"}><div class="library-preview library-media"><video controls preload="metadata" src={mediaUrl()} aria-label={props.artifact.name} /></div></Match>
            </Switch>}
          </Match>
          <Match when={content.loading}><div class="library-preview dock-empty" role="status">Opening preview…</div></Match>
        </Switch>
      </Show>
    </section>
  );
}
