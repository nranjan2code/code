import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { previewSandbox, previewWindowDocument, sandboxedSrcdoc } from "../../safeUrl";
import { artifactPreviewHtml, subjectReader } from "../../artifactPreview";
import { previewSource, subjectKey, subjectPath } from "../../canvasSubject";
import { createLoader, readText } from "./createLoader";
import DeviceViewport from "./DeviceViewport";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";

type Shown =
  /** Served from an origin of its own, with the files it loads. */
  | { mode: "origin"; id: string; url: string }
  /** One document in an opaque-origin frame. */
  | { mode: "document"; page: string; warning: string | null };

/**
 * A page. One with files behind it is served from a preview origin, so its
 * own stylesheets, scripts, modules, images and links load as they do on any
 * web server; markup from the conversation, and a page the server cannot
 * serve from an origin of its own, is shown as one document.
 *
 * The page is read once for what it is (`subjectKey`) and again only when the
 * reader asks to reload: reopening it, or moving to the code and back, keeps
 * the page as the reader left it, with whatever they typed or clicked in it.
 */
export default function HtmlViewer(props: ViewerProps) {
  const loader = createLoader(
    () => subjectKey(props.subject),
    async () => {
      const subject = props.subject;
      const reader = subjectReader(subject);
      const file = subjectPath(subject);
      const raw = subject.kind === "inline" ? subject.html : await readText(reader, file);
      let warning: string | null = null;
      const parsed = new DOMParser().parseFromString(raw, "text/html");
      if (!parsed.body?.textContent?.trim() && !parsed.body?.children.length && !parsed.querySelector("script")) {
        warning = "This page has nothing visible to show. Open Code to see what it contains.";
      }

      const source = previewSource(subject);
      const origin = source ? await api.openPreview(source) : null;
      if (origin) return { raw, warning, shown: { mode: "origin", id: origin.id, url: origin.url } satisfies Shown };

      let page: string;
      if (file && reader) {
        try {
          page = await artifactPreviewHtml(file, raw, reader);
        } catch {
          page = sandboxedSrcdoc(raw);
          warning = "Some files this page uses could not be loaded, so it may look incomplete.";
        }
      } else {
        page = sandboxedSrcdoc(raw);
      }
      return { raw, warning, shown: { mode: "document", page, warning } satisfies Shown };
    },
    (loaded) => { if (loaded.shown.mode === "origin") void api.closePreview(loaded.shown.id); },
    { reloadOn: () => props.reloadKey },
  );

  createEffect(() => {
    const shown = loader.data()?.shown;
    props.register(shown ? {
      popout: () => {
        if (shown.mode === "origin") {
          window.open(shown.url, "_blank", "noopener,noreferrer");
          return;
        }
        const url = URL.createObjectURL(new Blob([previewWindowDocument(shown.page)], { type: "text/html" }));
        window.open(url, "_blank", "noopener,noreferrer");
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      },
    } : null);
  });
  onCleanup(() => props.register(null));

  return (
    <LoadState loader={loader} label="Opening the page…">{(loaded) => {
      // Shown only once it has drawn, so the stage does not flash between the
      // Canvas's colour and the page's.
      const [drawn, setDrawn] = createSignal(false);
      const showAnyway = setTimeout(() => setDrawn(true), 1500);
      onCleanup(() => clearTimeout(showAnyway));
      return (
        <>
          {/* The page stays alive behind the code, so coming back to it finds it as it was. */}
          <div class="artifact-canvas-page" classList={{ resting: props.view === "source" }}>
            <Show when={loaded.shown.mode === "document" ? loaded.shown.warning : loaded.warning}>{(warning) => <div class="artifact-canvas-preview-warning" role="status">{warning()}</div>}</Show>
            <DeviceViewport>
              {loaded.shown.mode === "origin"
                ? <iframe class="artifact-canvas-frame" classList={{ drawing: !drawn() }} onLoad={() => setDrawn(true)} src={loaded.shown.url} title={props.subject.title} sandbox={previewSandbox("origin")} />
                : <iframe class="artifact-canvas-frame" classList={{ drawing: !drawn() }} onLoad={() => setDrawn(true)} srcdoc={loaded.shown.page} title={props.subject.title} sandbox={previewSandbox("static")} />}
            </DeviceViewport>
          </div>
          <Show when={props.view === "source"}>
            <SourcePane text={loaded.raw} label={props.subject.kind === "inline" ? "From the conversation" : props.subject.title} path={subjectPath(props.subject)} selection={props.selection} onSelect={props.onSelect} />
          </Show>
        </>
      );
    }}</LoadState>
  );
}
