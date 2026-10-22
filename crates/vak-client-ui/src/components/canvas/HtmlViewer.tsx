import { withPreviewLibraries } from "../../previewLibraries";
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { previewWindowDocument, previewOriginWindowDocument, sandboxedSrcdoc } from "../../safeUrl";
import { artifactPreviewHtml, artifactPreviewMarkup, subjectReader } from "../../artifactPreview";
import { previewSource, subjectKey, subjectPath } from "../../canvasSubject";
import { createLoader, readText } from "./createLoader";
import DeviceViewport from "./DeviceViewport";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";
import HtmlPreview from "../HtmlPreview";

type Shown =
  /** Served from an origin of its own, with the files it loads. */
  | { mode: "origin"; id: string; url: string }
  /** One document in an opaque-origin frame. */
  | { mode: "document"; page: string; warning: string | null };

/**
 * A page. One with files behind it is served from a preview origin, so its
 * own stylesheets and images load. Only the fixed theme bridge may run;
 * document scripts and event handlers start paused. A human may run a bundled
 * copy in an opaque frame, with possible network access disclosed. Markup from
 * the conversation and fallback pages are shown as one document.
 *
 * The page is read once for what it is (`subjectKey`) and again only when the
 * reader asks to reload: reopening it, or moving to the code and back, keeps
 * the page as the reader left it, with whatever they typed or clicked in it.
 */
export default function HtmlViewer(props: ViewerProps) {
  const [running, setRunning] = createSignal(false);
  const loader = createLoader(
    () => [subjectKey(props.subject), props.subject.kind === "inline" ? props.subject.html : ""],
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
        const libraries = subject.kind === "inline" ? subject.libraries ?? [] : [];
        page = sandboxedSrcdoc(await withPreviewLibraries(raw, libraries));
      }
      return { raw, warning, shown: { mode: "document", page, warning } satisfies Shown };
    },
    (loaded) => { if (loaded.shown.mode === "origin") void api.closePreview(loaded.shown.id); },
    { reloadOn: () => props.reloadKey },
  );

  createEffect(() => {
    const shown = loader.data()?.shown;
    props.register(shown ? {
      safetyLabel: running() ? "Page code running · network access possible" : "Safe preview, offline",
      popout: () => {
        const document = shown.mode === "origin"
          ? previewOriginWindowDocument(shown.url)
          : previewWindowDocument(shown.page);
        const url = URL.createObjectURL(new Blob([document], { type: "text/html" }));
        window.open(url, "_blank", "noopener,noreferrer");
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      },
    } : null);
  });
  onCleanup(() => props.register(null));

  return (
    <LoadState loader={loader} label="Opening the page…">{(loaded) => {
      return (
        <>
          {/* The page stays alive behind the code, so coming back to it finds it as it was. */}
          <div class="artifact-canvas-page" classList={{ resting: props.view === "source" }}>
            <Show when={loaded.shown.mode === "document" ? loaded.shown.warning : loaded.warning}>{(warning) => <div class="artifact-canvas-preview-warning" role="status">{warning()}</div>}</Show>
            <HtmlPreview
              html={loaded.raw}
              title={props.subject.title}
              sourceLabel={props.subject.kind === "inline" ? "From this conversation" : "From the selected file"}
              page={loaded.shown.mode === "document" ? loaded.shown.page : ""}
              origin={loaded.shown.mode === "origin" ? loaded.shown.url : undefined}
              localFiles={!!subjectPath(props.subject) && !!subjectReader(props.subject)}
              libraries={props.subject.kind === "inline" ? props.subject.libraries : undefined}
              prepare={async () => {
                const subject = props.subject;
                const reader = subjectReader(subject);
                const path = subjectPath(subject);
                return path && reader ? artifactPreviewMarkup(path, loaded.raw, reader)
                  : withPreviewLibraries(loaded.raw, subject.kind === "inline" ? subject.libraries ?? [] : []);
              }}
              frameClass="artifact-canvas-frame"
              viewport={frame => <DeviceViewport>{frame}</DeviceViewport>}
              permissionKey={props.reloadKey}
              onState={setRunning}
            />
          </div>
          <Show when={props.view === "source"}>
            <SourcePane text={loaded.raw} label={props.subject.kind === "inline" ? "From the conversation" : props.subject.title} path={subjectPath(props.subject)} selection={props.selection} onSelect={props.onSelect} />
          </Show>
        </>
      );
    }}</LoadState>
  );
}
