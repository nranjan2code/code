import { createEffect, createMemo, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type {
  DocumentBlock,
  InlineNode,
  OutputItem,
  OutputTimeline,
  PresentationDocument,
} from "../types";
import { density, openInEditor, uiPreferences } from "../store";
import { approve, openFileSmart } from "../App";
import Icon from "./Icon";
import MarkdownView from "./MarkdownView";
import { safeUrl } from "../safeUrl";
import { highlight, languageForFence } from "../highlight";
import ResearchCards from "./presentation/ResearchCards";
import DiffInspector from "./presentation/DiffInspector";
import TestMatrix from "./presentation/TestMatrix";
import UniversalChart from "./presentation/UniversalChart";
import DataGrid from "./presentation/DataGrid";
import TerminalConsole from "./presentation/TerminalConsole";
import RecipeCard from "./presentation/RecipeCard";

function InlineSequence(props: { nodes: InlineNode[] }): JSX.Element {
  return (
    <For each={props.nodes}>
      {(node) => {
        switch (node.type) {
          case "text":
            return node.text;
          case "strong":
            return <strong><InlineSequence nodes={node.content} /></strong>;
          case "emphasis":
            return <em><InlineSequence nodes={node.content} /></em>;
          case "strikethrough":
            return <s><InlineSequence nodes={node.content} /></s>;
          case "code": {
            const pathLike = /^[\w@.-]+(\/[\w@.-]+)+$|^\.[\w/-]+$/.test(node.code);
            return (
              <code
                class="ic"
                classList={{ "semantic-path": pathLike }}
                tabIndex={pathLike ? 0 : undefined}
                role={pathLike ? "button" : undefined}
                aria-label={pathLike ? `Open ${node.code} in editor` : undefined}
                onClick={() => pathLike && openInEditor(node.code)}
                onKeyDown={(event) => {
                  if (pathLike && (event.key === "Enter" || event.key === " ")) {
                    event.preventDefault();
                    openInEditor(node.code);
                  }
                }}
              >
                {node.code}
              </code>
            );
          }
          case "link":
            return node.safe && safeUrl(node.url) ? (
              <a class="semantic-link" href={node.url} target="_blank" rel="noreferrer noopener" title={node.title ?? node.url}>
                <InlineSequence nodes={node.label} />
              </a>
            ) : (
              <span class="semantic-unsafe-link" title="Unsafe link omitted">
                <InlineSequence nodes={node.label} />
              </span>
            );
          case "image":
            return node.safe && safeUrl(node.url, true) ? <img class="semantic-image" src={node.url} alt={node.alt} title={node.title ?? undefined} /> : <span>{node.alt}</span>;
          case "soft_break":
            return " ";
          case "hard_break":
            return <br />;
          case "raw_html":
            return <code class="semantic-raw" title="Raw HTML shown as inert text">{node.html}</code>;
        }
      }}
    </For>
  );
}

function Heading(props: { block: Extract<DocumentBlock, { type: "heading" }> }) {
  const content = () => <InlineSequence nodes={props.block.content} />;
  switch (props.block.level) {
    case 1: return <h2 class="semantic-heading h1">{content()}</h2>;
    case 2: return <h3 class="semantic-heading h2">{content()}</h3>;
    case 3: return <h4 class="semantic-heading h3">{content()}</h4>;
    default: return <h5 class="semantic-heading h4">{content()}</h5>;
  }
}

function CodeBlock(props: { language?: string | null; filename?: string | null; content: string; diff?: boolean }) {
  let pre!: HTMLPreElement;
  let renderSeq = 0;
  createEffect(() => {
    const content = props.content;
    const language = props.language;
    const seq = ++renderSeq;
    pre.textContent = content;
    pre.classList.remove("shiki");
    if (!language || props.diff) return;
    void highlight(content, languageForFence(language)).then((html) => {
      if (!html || seq !== renderSeq || !pre.isConnected) return;
      const template = document.createElement("template");
      template.innerHTML = html;
      const highlighted = template.content.querySelector("pre");
      if (highlighted) {
        pre.innerHTML = highlighted.innerHTML;
        pre.classList.add("shiki");
      }
    });
  });
  const copy = (button: HTMLButtonElement) => {
    void navigator.clipboard.writeText(props.content);
    button.textContent = "Copied";
    setTimeout(() => (button.textContent = "Copy"), 900);
  };
  return (
    <div class="semantic-code" classList={{ diff: !!props.diff }}>
      <div class="semantic-code-head">
        <span>{props.filename ?? props.language ?? (props.diff ? "diff" : "text")}</span>
        <button onClick={(event) => copy(event.currentTarget)}>Copy</button>
      </div>
      <pre ref={pre}><code>{props.content}</code></pre>
    </div>
  );
}

function inlineToText(nodes: InlineNode[]): string {
  return nodes
    .map((n) => {
      if (n.type === "text") return n.text;
      if ("content" in n && Array.isArray((n as any).content)) return inlineToText((n as any).content);
      if ("code" in n && typeof (n as any).code === "string") return (n as any).code;
      return "";
    })
    .join("");
}

function Blocks(props: { blocks: DocumentBlock[]; recipeId?: string }): JSX.Element {
  return (
    <For each={props.blocks}>
      {(block) => {
        switch (block.type) {
          case "heading":
            return <Heading block={block} />;
          case "paragraph":
            return <p class="semantic-paragraph"><InlineSequence nodes={block.content} /></p>;
          case "list": {
            const items = () => <For each={block.items}>{(item) => <li><Blocks blocks={item} recipeId={props.recipeId} /></li>}</For>;
            return block.ordered ? <ol class="semantic-list" start={block.start ?? undefined}>{items()}</ol> : <ul class="semantic-list">{items()}</ul>;
          }
          case "table": {
            if (props.recipeId === "data.spreadsheet_grid" || block.rows.length >= 3) {
              const columns = block.header.map((cell, idx) => ({
                key: `col_${idx}`,
                label: inlineToText(cell) || `Col ${idx + 1}`,
                isNumeric: block.alignments[idx] === "right",
              }));
              const rows = block.rows.map((row) => {
                const record: Record<string, any> = {};
                row.forEach((cell, idx) => {
                  record[`col_${idx}`] = inlineToText(cell);
                });
                return record;
              });
              return <DataGrid data={{ columns, rows }} />;
            }
            return (
              <div class="semantic-table-wrap">
                <table class="semantic-table">
                  <thead><tr><For each={block.header}>{(cell, index) => <th style={{ "text-align": block.alignments[index()] === "right" ? "right" : block.alignments[index()] === "center" ? "center" : "left" }}><InlineSequence nodes={cell} /></th>}</For></tr></thead>
                  <tbody><For each={block.rows}>{(row) => <tr><For each={row}>{(cell, index) => <td style={{ "text-align": block.alignments[index()] === "right" ? "right" : block.alignments[index()] === "center" ? "center" : "left" }}><InlineSequence nodes={cell} /></td>}</For></tr>}</For></tbody>
                </table>
              </div>
            );
          }
          case "quote":
            return <blockquote class="semantic-quote"><Blocks blocks={block.blocks} recipeId={props.recipeId} /></blockquote>;
          case "code":
            if (block.language === "diff") {
              return <DiffInspector rawDiff={block.content} filename={block.filename ?? undefined} />;
            }
            if (props.recipeId === "terminal.session" && (block.language === "bash" || block.language === "sh" || block.language === "shell")) {
              return <TerminalConsole data={{ command: block.content.split("\n")[0], output: block.content, exit_code: 0 }} />;
            }
            return <CodeBlock language={block.language} filename={block.filename} content={block.content} />;
          case "diff":
            return <DiffInspector rawDiff={block.content} />;
          case "callout":
            return <section class={`semantic-callout ${block.tone}`}><Show when={block.title}><strong>{block.title}</strong></Show><Blocks blocks={block.blocks} recipeId={props.recipeId} /></section>;
          case "citations":
            return <ol class="semantic-citations"><For each={block.items}>{(citation) => <li><Show when={safeUrl(citation.url)} fallback={<span>{citation.label}</span>}><a href={citation.url} target="_blank" rel="noreferrer noopener">{citation.label}</a></Show></li>}</For></ol>;
          case "media":
            return safeUrl(block.source, true) ? (block.media_type?.startsWith("image/") ? <img class="semantic-media" src={block.source} alt={block.alt} /> : <a class="semantic-media-link" href={block.source} target="_blank" rel="noreferrer noopener">{block.alt || "Open media"}</a>) : <span class="semantic-unsafe-link">{block.alt || "Unsafe media omitted"}</span>;
          case "artifact_ref":
            return <Artifact item={{ id: block.id, turn_id: "", timestamp: "", role: "assistant", kind: "artifact", status: "succeeded", content: { type: "artifact", artifact: block.artifact }, actions: [], fallback_text: block.artifact.path ?? block.artifact.name }} />;
          case "rule":
            return <hr class="semantic-rule" />;
          case "raw_markdown":
            return <div class="semantic-limited"><span>Limited rendering</span><pre>{block.markdown}</pre><small>{block.reason}</small></div>;
        }
      }}
    </For>
  );
}

export function PresentationDocumentView(props: { document: PresentationDocument }) {
  const recipeId = () => props.document.metadata.recipe_id;
  const plain = () => props.document.blocks.every((block) => ["heading", "paragraph", "list", "quote", "rule"].includes(block.type));
  if (!recipeId() && plain() && props.document.source_markdown.trim()) {
    return <><MarkdownView text={props.document.source_markdown} /><RenderAudit document={props.document} /></>;
  }
  return (
    <div class="semantic-document">
      <Blocks blocks={props.document.blocks} recipeId={recipeId()} />
      <For each={props.document.diagnostics}>{(diagnostic) => <div class="semantic-diagnostic">{diagnostic}</div>}</For>
      <RenderAudit document={props.document} />
    </div>
  );
}

function RenderAudit(props: { document: PresentationDocument }) {
  const metadata = props.document.metadata;
  const recipe = metadata.recipe_id;
  return <Show when={recipe || props.document.diagnostics.length > 0}>
    <details class="semantic-render-audit">
      <summary>Why this rendering?</summary>
      <dl>
        <Show when={recipe}><div><dt>Recipe</dt><dd>{recipe} · {metadata.recipe_version ?? "unknown version"}</dd></div></Show>
        <Show when={metadata.renderer}><div><dt>Renderer</dt><dd>{metadata.renderer}</dd></div></Show>
        <Show when={metadata.matched_signals}><div><dt>Signals</dt><dd>{metadata.matched_signals}</dd></div></Show>
        <Show when={props.document.diagnostics.length > 0}><div><dt>Diagnostics</dt><dd>{props.document.diagnostics.join("; ")}</dd></div></Show>
      </dl>
    </details>
  </Show>;
}

function Artifact(props: { item: OutputItem }) {
  if (props.item.content.type !== "artifact") return null;
  const artifact = props.item.content.artifact;
  const path = () => artifact.path ?? null;
  return (
    <article class="artifact-item">
      <span class="artifact-icon"><Icon name={artifact.media_type?.startsWith("image/") ? "preview" : "file"} size={15} /></span>
      <span class="artifact-copy"><strong>{artifact.name}</strong><small>{artifact.description ?? artifact.media_type ?? "Artifact"}</small></span>
      <Show when={path()}>{(value) => <button class="artifact-open" onClick={() => void openFileSmart(value())}>Open</button>}</Show>
    </article>
  );
}

function SemanticApproval(props: { item: OutputItem; sessionId: string }) {
  if (props.item.content.type !== "approval") return null;
  const content = props.item.content;
  const pending = () => props.item.status === "pending";
  return (
    <section class="approval semantic-approval">
      <div class="ap-head">Approval requested — {content.tool}</div>
      <div class="ap-reason">{content.reason}</div>
      <details class="ap-details"><summary>View request details</summary><pre class="ap-args">{content.args_json}</pre></details>
      <Show when={pending()} fallback={<div class="ap-done">{props.item.status}</div>}>
        <div class="ap-actions">
          <button class="btn primary" onClick={() => void approve(content.request_id, true, props.sessionId)}>Allow once</button>
          <button class="btn danger" onClick={() => void approve(content.request_id, false, props.sessionId)}>Deny</button>
        </div>
      </Show>
    </section>
  );
}

function ActivityRow(props: { item: OutputItem }) {
  const label = () => {
    switch (props.item.content.type) {
      case "progress": return props.item.content.label;
      case "retry": return `Retry ${props.item.content.attempt}`;
      case "information": return props.item.content.label;
      case "error": return props.item.content.source ?? "Error";
      case "outcome": return "Run outcome";
      default: return props.item.kind;
    }
  };
  const provenance = props.item.provenance;
  const evidence = [provenance?.source, provenance?.entry_id, provenance?.tool_call_id].filter(Boolean).join(" · ");
  return <div class={`semantic-activity ${props.item.status}`} title={evidence || undefined}><span class="semantic-status-dot" /><strong>{label()}</strong><span>{props.item.fallback_text}</span><small>{props.item.status} · {new Date(props.item.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</small></div>;
}

function StructuredView(props: { output: import("../types").StructuredOutput }) {
  if (!uiPreferences.richPreviews) {
    return <div class="rich-unsupported"><strong>{props.output.semantic_type}</strong><span>Rich rendering is disabled in Appearance settings.</span></div>;
  }
  const payload = props.output.payload as any;
  if (props.output.semantic_type === "research.synthesis") {
    return <ResearchCards data={payload} />;
  }
  if (props.output.semantic_type === "coding.diff") {
    return <DiffInspector data={payload} />;
  }
  if (props.output.semantic_type === "test.report") {
    return <TestMatrix data={payload} />;
  }
  if (props.output.semantic_type === "terminal.view") {
    return <TerminalConsole data={payload} />;
  }
  if (props.output.semantic_type === "data.grid") {
    return <DataGrid data={payload} />;
  }
  if (props.output.semantic_type === "recipe.card") {
    return <RecipeCard data={payload} />;
  }
  if (props.output.semantic_type === "chart" && Array.isArray(payload.series)) {
    return <UniversalChart data={payload} />;
  }
  if (props.output.semantic_type === "link.preview" && typeof payload.url === "string") {
    return <a class="rich-link-card" href={safeUrl(payload.url) ? payload.url : undefined} target="_blank" rel="noreferrer noopener">
      <Show when={typeof payload.image_url === "string" && safeUrl(payload.image_url, true)}><img src={payload.image_url as string} alt="" /></Show>
      <span><strong>{String(payload.title ?? payload.url)}</strong><small>{String(payload.description ?? payload.site_name ?? payload.url)}</small></span>
    </a>;
  }
  if (props.output.semantic_type === "metric") {
    return <div class="rich-metric"><small>{String(payload.label ?? "Metric")}</small><strong>{String(payload.value ?? "—")}{payload.unit ? ` ${String(payload.unit)}` : ""}</strong></div>;
  }
  if (props.output.semantic_type.startsWith("media.") && uiPreferences.externalMedia && typeof payload.source === "string" && safeUrl(payload.source, true)) {
    return payload.media_type === "image" || String(payload.media_type).startsWith("image/")
      ? <figure class="rich-media"><img src={payload.source} alt={String(payload.alt ?? "")} /><Show when={payload.alt}><figcaption>{String(payload.alt)}</figcaption></Show></figure>
      : String(payload.media_type).startsWith("video/") ? <video class="rich-video" src={payload.source} controls preload="metadata" autoplay={uiPreferences.autoplayMedia} aria-label={String(payload.alt ?? "Video")} />
      : <a class="rich-media-link" href={payload.source} target="_blank" rel="noreferrer noopener">Open {String(payload.media_type ?? "media")}</a>;
  }
  return <div class="rich-unsupported"><strong>{props.output.semantic_type}</strong><span>{JSON.stringify(payload)}</span></div>;
}

function Turn(props: { id: string; items: OutputItem[]; sessionId: string }) {
  const users = () => props.items.filter((item) => item.role === "user" && item.content.type === "document");
  const outcomes = () => props.items.filter((item) => item.kind === "outcome" && item.content.type === "document");
  const errors = () => props.items.filter((item) => item.kind === "error");
  const approvals = () => props.items.filter((item) => item.kind === "approval");
  const artifacts = () => props.items.filter((item) => item.kind === "artifact");
  const structured = () => props.items.filter((item) => item.content.type === "structured");
  const ordered = () => props.items;
  const OrderedItem = (item: OutputItem): JSX.Element | null => {
    if (item.role === "user" && item.content.type === "document") {
      return <div class="semantic-user"><PresentationDocumentView document={item.content.document} /></div>;
    }
    if (item.kind === "approval") return <SemanticApproval item={item} sessionId={props.sessionId} />;
    if (item.kind === "outcome" && item.content.type === "document") return <article class="semantic-outcome"><PresentationDocumentView document={item.content.document} /></article>;
    if (item.content.type === "structured") return <StructuredView output={item.content.output} />;
    if (item.kind === "error") return <section class="semantic-recovery" role="alert"><Icon name="warning" size={15} /><div><strong>{item.status === "partial" ? "Partial outcome" : "Run needs attention"}</strong><p>{item.fallback_text}</p></div></section>;
    if (item.kind === "artifact") return <section class="artifact-shelf" aria-label="Artifact"><Artifact item={item} /></section>;
    if (["progress", "retry", "information"].includes(item.kind)) return <ActivityRow item={item} />;
    return null;
  };
  return (
    <section class="semantic-turn" data-turn={props.id}>
      <Show when={density() !== "outcome"} fallback={
        <>
          <For each={users()}>{(item) => item.content.type === "document" && <div class="semantic-user"><PresentationDocumentView document={item.content.document} /></div>}</For>
          <For each={approvals()}>{(item) => <SemanticApproval item={item} sessionId={props.sessionId} />}</For>
          <For each={outcomes()}>{(item) => item.content.type === "document" && <article class="semantic-outcome"><PresentationDocumentView document={item.content.document} /></article>}</For>
          <For each={structured()}>{(item) => item.content.type === "structured" && <StructuredView output={item.content.output} />}</For>
          <For each={errors()}>{(item) => <section class="semantic-recovery" role="alert"><Icon name="warning" size={15} /><div><strong>{item.status === "partial" ? "Partial outcome" : "Run needs attention"}</strong><p>{item.fallback_text}</p></div></section>}</For>
          <Show when={artifacts().length > 0}><section class="artifact-shelf" aria-label="Artifacts"><div class="artifact-shelf-head">Artifacts <span>{artifacts().length}</span></div><For each={artifacts()}>{(item) => <Artifact item={item} />}</For></section></Show>
        </>
      }>
        <For each={ordered()}>{(item) => OrderedItem(item)}</For>
      </Show>
    </section>
  );
}

export default function PresentationTimelineView(props: { timeline: OutputTimeline; sessionId: string }) {
  const turns = createMemo(() => {
    const order: string[] = [];
    const grouped = new Map<string, OutputItem[]>();
    for (const item of props.timeline.items) {
      if (!grouped.has(item.turn_id)) {
        order.push(item.turn_id);
        grouped.set(item.turn_id, []);
      }
      grouped.get(item.turn_id)!.push(item);
    }
    return order.map((id) => ({ id, items: grouped.get(id)! }));
  });
  return <div class="semantic-timeline"><For each={turns()}>{(turn) => <Turn id={turn.id} items={turn.items} sessionId={props.sessionId} />}</For></div>;
}
