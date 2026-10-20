import { render } from "solid-js/web";
import "../src/components/ChatPane";
import MarkdownView from "../src/components/MarkdownView";
import { PresentationDocumentView } from "../src/components/PresentationRenderer";
import MapCard from "../src/components/cards/Map";
import { setMapSettings } from "../src/mapSettings";
import { MediaForm, LinkForm } from "../src/components/cards/code";
import { setUiPreferences } from "../src/store";
import "../src/styles.css";

import { previewSandbox, sandboxedSrcdoc } from "../src/safeUrl";
import { installExternalLinks } from "../src/externalLinks";

const opened: string[] = [];
Object.assign(window, { openedExternalLinks: opened });
const params = new URLSearchParams(location.search);
installExternalLinks(document, params.get("native") === "1" ? async (url) => { opened.push(url); } : undefined);
document.documentElement.dataset.theme = params.get("theme") ?? "light";
setUiPreferences("externalMedia", params.get("media") !== "off");
const url = "http://127.0.0.1:39493/collect?marker=SYNTHETIC_PRIVATE_MARKER";
const markdown = `![External illustration](${url})`;
const image = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==";
const requests: string[] = [];
let unsafePreview = false;
window.addEventListener("message", (event) => { if (event.data === "UNSAFE_PREVIEW_EXECUTED") unsafePreview = true; });
window.fetch = async (input) => { requests.push(String(input)); return new Response("{}", { headers: { "Content-Type": "application/json" } }); };
setMapSettings({ tiles: true });
render(() => <main class="settings-content">
  <h1>Answer network boundaries</h1>
  <section aria-label="Streaming answer"><h2>Streaming answer</h2><MarkdownView text={markdown} streaming /></section>
  <section aria-label="Saved answer"><h2>Saved answer</h2><MarkdownView text={markdown} /></section>
  <section aria-label="Settled answer"><h2>Settled answer</h2><PresentationDocumentView document={{ schema_version: 1, source_markdown: markdown, metadata: {}, diagnostics: [], coverage: [], blocks: [
    { type: "paragraph", id: "image", content: [{ type: "image", alt: "External illustration", url, safe: true }] },
    { type: "media", id: "media", source: url, media_type: "image/png", alt: "External attachment" },
  ] }} /></section>
  <section aria-label="Media cards"><h2>Media cards</h2>
    <MediaForm payload={{ format: "image", url, alt: "External image" }} />
    <MediaForm payload={{ format: "video", url, alt: "External video" }} />
    <MediaForm payload={{ format: "audio", url, alt: "External audio" }} />
    <LinkForm payload={{ url, title: "External resource", image_url: url }} />
  </section>
  <section aria-label="Local map"><h2>Local map</h2><MapCard payload={{ title: "Public landmarks", places: [{ label: "Tower Bridge", latitude: 51.5055, longitude: -0.0754 }] }} /></section>
  <section aria-label="Document preview"><h2>Document preview</h2><iframe title="Script-blocked document" sandbox={previewSandbox("static")} srcdoc={sandboxedSrcdoc('<label>Native input <input value="Editable" /></label><script>parent.postMessage("UNSAFE_PREVIEW_EXECUTED","*")</script>')} /></section>
  <section aria-label="Self-contained image"><h2>Self-contained image</h2><MarkdownView text={`![Local illustration](${image})`} /></section>
</main>, document.getElementById("root")!);
Object.assign(window, { runChecks: () => {
  if (unsafePreview) throw new Error("Document script executed");
  const remote = [...document.querySelectorAll("img,video,audio,source,iframe,image")].filter(el => ["src", "href", "xlink:href"].some(attr => el.getAttribute(attr)?.includes("SYNTHETIC_PRIVATE_MARKER")));
  if (remote.length) throw new Error("Remote media became active");
  if (requests.some(url => url.includes("/maps/"))) throw new Error("Map tiles loaded without consent");
  if (![...document.querySelectorAll("button")].some(button => button.textContent?.includes("Load street map"))) throw new Error("Map consent control is missing");
  if (!document.querySelector('img[src^="data:image/png"]')) throw new Error("Self-contained image was lost");
  if (document.querySelectorAll(`a[href="${url}"]`).length < 7) throw new Error("Explicit external links are missing");
  if (document.documentElement.scrollWidth > window.innerWidth) throw new Error("Horizontal overflow");
  return { checked: true, externalLinks: document.querySelectorAll(`a[href="${url}"]`).length, width: innerWidth, theme: document.documentElement.dataset.theme, media: params.get("media") ?? "on" };
} });
