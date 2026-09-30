import { createEffect, onCleanup, Show } from "solid-js";
import { previewSandbox, previewWindowDocument, sandboxedSrcdoc } from "../../safeUrl";
import { artifactPreviewHtml, subjectReader } from "../../artifactPreview";
import { subjectPath } from "../../canvasSubject";
import { createLoader, readText } from "./createLoader";
import DeviceViewport from "./DeviceViewport";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";

/** A page, from a file read through its subject's route or from the conversation's own markup. */
export default function HtmlViewer(props: ViewerProps) {
  const loader = createLoader(
    () => [props.subject, props.reloadKey] as const,
    async ([subject]) => {
      const reader = subjectReader(subject);
      const file = subjectPath(subject);
      const raw = subject.kind === "inline" ? subject.html : await readText(reader, file);
      let page: string;
      let warning: string | null = null;
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
      const parsed = new DOMParser().parseFromString(raw, "text/html");
      if (!warning && !parsed.body?.textContent?.trim() && !parsed.body?.children.length && !parsed.querySelector("script")) {
        warning = "This HTML has no visible page content. Open Code to inspect it or ask for a revision.";
      }
      return { raw, page, warning };
    },
  );

  createEffect(() => {
    const loaded = loader.data();
    props.register(loaded ? {
      popout: () => {
        const url = URL.createObjectURL(new Blob([previewWindowDocument(loaded.page)], { type: "text/html" }));
        window.open(url, "_blank", "noopener,noreferrer");
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      },
    } : null);
  });
  onCleanup(() => props.register(null));

  return (
    <LoadState loader={loader}>{(loaded) =>
      <>
        <Show when={props.view === "source"} fallback={
          <>
            <Show when={loaded.warning}>{(warning) => <div class="artifact-canvas-preview-warning" role="status">{warning()}</div>}</Show>
            <DeviceViewport>
              <iframe class="artifact-canvas-frame" srcdoc={loaded.page} title={props.subject.title} sandbox={previewSandbox("static")} />
            </DeviceViewport>
          </>
        }>
          <SourcePane text={loaded.raw} label={subjectPath(props.subject) || "inline markup"} selection={props.selection} onSelect={props.onSelect} />
        </Show>
      </>
    }</LoadState>
  );
}
