import { createMemo, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type {
  DocumentBlock,
  InlineNode,
  OutputItem,
  OutputTimeline,
  PresentationDocument,
} from "../types";
import { density, openInEditor } from "../store";
import { approve, openFileSmart } from "../App";
import Icon from "./Icon";

function safeUrl(value: string, media = false): boolean {
  const normalized = value.trim().toLowerCase();
  if (media && normalized.startsWith("data:image/")) return true;
  return normalized.startsWith("https://") || normalized.startsWith("http://") || normalized.startsWith("mailto:");
}

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
                onClick={() => pathLike && openInEditor(node.code)}
              >
                {node.code}
              </code>
            );
          }
          case "link":
            return node.safe && safeUrl(node.url) ? (
              <a class="semantic-link" href={node.url} target="_blank" rel="noreferrer" title={node.title ?? node.url}>
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
      <pre><code>{props.content}</code></pre>
    </div>
  );
}

function Blocks(props: { blocks: DocumentBlock[] }): JSX.Element {
  return (
    <For each={props.blocks}>
      {(block) => {
        switch (block.type) {
          case "heading":
            return <Heading block={block} />;
          case "paragraph":
            return <p class="semantic-paragraph"><InlineSequence nodes={block.content} /></p>;
          case "list": {
            const items = () => <For each={block.items}>{(item) => <li><Blocks blocks={item} /></li>}</For>;
            return block.ordered ? <ol class="semantic-list" start={block.start ?? undefined}>{items()}</ol> : <ul class="semantic-list">{items()}</ul>;
          }
          case "table":
            return (
              <div class="semantic-table-wrap">
                <table class="semantic-table">
                  <thead><tr><For each={block.header}>{(cell, index) => <th style={{ "text-align": block.alignments[index()] === "right" ? "right" : block.alignments[index()] === "center" ? "center" : "left" }}><InlineSequence nodes={cell} /></th>}</For></tr></thead>
                  <tbody><For each={block.rows}>{(row) => <tr><For each={row}>{(cell, index) => <td style={{ "text-align": block.alignments[index()] === "right" ? "right" : block.alignments[index()] === "center" ? "center" : "left" }}><InlineSequence nodes={cell} /></td>}</For></tr>}</For></tbody>
                </table>
              </div>
            );
          case "quote":
            return <blockquote class="semantic-quote"><Blocks blocks={block.blocks} /></blockquote>;
          case "code":
            return <CodeBlock language={block.language} filename={block.filename} content={block.content} />;
          case "diff":
            return <CodeBlock language="diff" content={block.content} diff />;
          case "callout":
            return <section class={`semantic-callout ${block.tone}`}><Show when={block.title}><strong>{block.title}</strong></Show><Blocks blocks={block.blocks} /></section>;
          case "citations":
            return <ol class="semantic-citations"><For each={block.items}>{(citation) => <li><Show when={safeUrl(citation.url)} fallback={<span>{citation.label}</span>}><a href={citation.url} target="_blank" rel="noreferrer">{citation.label}</a></Show></li>}</For></ol>;
          case "media":
            return safeUrl(block.source, true) ? (block.media_type?.startsWith("image/") ? <img class="semantic-media" src={block.source} alt={block.alt} /> : <a class="semantic-media-link" href={block.source} target="_blank" rel="noreferrer">{block.alt || "Open media"}</a>) : <span class="semantic-unsafe-link">{block.alt || "Unsafe media omitted"}</span>;
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
  return (
    <div class="semantic-document">
      <Blocks blocks={props.document.blocks} />
      <For each={props.document.diagnostics}>{(diagnostic) => <div class="semantic-diagnostic">{diagnostic}</div>}</For>
    </div>
  );
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
  return <div class={`semantic-activity ${props.item.status}`}><span class="semantic-status-dot" /><strong>{label()}</strong><span>{props.item.fallback_text}</span><small>{props.item.status}</small></div>;
}

function Turn(props: { id: string; items: OutputItem[]; sessionId: string }) {
  const users = () => props.items.filter((item) => item.role === "user" && item.content.type === "document");
  const outcomes = () => props.items.filter((item) => item.kind === "outcome" && item.content.type === "document");
  const errors = () => props.items.filter((item) => item.kind === "error");
  const approvals = () => props.items.filter((item) => item.kind === "approval");
  const artifacts = () => props.items.filter((item) => item.kind === "artifact");
  const activity = () => props.items.filter((item) => ["progress", "retry", "information"].includes(item.kind));
  return (
    <section class="semantic-turn" data-turn={props.id}>
      <For each={users()}>{(item) => item.content.type === "document" && <div class="semantic-user"><PresentationDocumentView document={item.content.document} /></div>}</For>
      <For each={approvals()}>{(item) => <SemanticApproval item={item} sessionId={props.sessionId} />}</For>
      <For each={outcomes()}>{(item) => item.content.type === "document" && <article class="semantic-outcome"><PresentationDocumentView document={item.content.document} /></article>}</For>
      <For each={errors()}>{(item) => <section class="semantic-recovery" role="alert"><Icon name="warning" size={15} /><div><strong>{item.status === "partial" ? "Partial outcome" : "Run needs attention"}</strong><p>{item.fallback_text}</p></div></section>}</For>
      <Show when={artifacts().length > 0}><section class="artifact-shelf" aria-label="Artifacts"><div class="artifact-shelf-head">Artifacts <span>{artifacts().length}</span></div><For each={artifacts()}>{(item) => <Artifact item={item} />}</For></section></Show>
      <Show when={activity().length > 0 && density() !== "outcome"}>
        <details class="semantic-audit" open={density() === "audit"}>
          <summary>Activity <span>{activity().length}</span></summary>
          <For each={activity()}>{(item) => <ActivityRow item={item} />}</For>
        </details>
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
