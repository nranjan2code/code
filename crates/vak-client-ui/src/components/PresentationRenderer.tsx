import { createEffect, createMemo, createSignal, ErrorBoundary, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type {
  DocumentBlock,
  InlineNode,
  OutputItem,
  OutputTimeline,
  PresentationDocument,
} from "../types";
import { density, openInEditor, presentationMode, uiPreferences } from "../store";
import { approve, openFileSmart } from "../App";
import Icon from "./Icon";
import { safeUrl } from "../safeUrl";
import * as api from "../api";
import { highlight, languageForFence } from "../highlight";
import ResearchCards, { type ResearchData } from "./presentation/ResearchCards";
import DiffInspector from "./presentation/DiffInspector";
import TestMatrix from "./presentation/TestMatrix";
import UniversalChart from "./presentation/UniversalChart";
import DataGrid, { type DataGridData, type DataGridColumn } from "./presentation/DataGrid";
import TerminalConsole from "./presentation/TerminalConsole";
import RecipeCard, { type RecipeData } from "./presentation/RecipeCard";
import MermaidViewer from "./presentation/MermaidViewer";
import UIPreviewCard from "./presentation/UIPreviewCard";
import UniversalCard from "./presentation/UniversalCard";

// Advanced is still a user-facing presentation. Operator chrome is reserved
// for development builds so production never becomes a ledger UI.
const showOperatorChrome = () => import.meta.env.DEV && presentationMode() === "advanced";
import TimelineCard, { type TimelineData } from "./presentation/TimelineCard";
import { parseVakFence, stripControlScaffolding } from "../structured";

/** Wraps settled assistant content with the same Vak avatar + name header
 *  that the streaming transcript uses, so completed turns don't lose their
 *  visual identity when ChatPane switches to PresentationTimelineView. */
function AssistantMessage(props: { children: JSX.Element }) {
  return (
    <div class="semantic-assistant">
      <div class="assistant-turn-head">
        <span class="assistant-avatar-mark">
          <img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" class="assistant-avatar-img" />
        </span>
        <span class="assistant-name">Vak</span>
      </div>
      <div class="assistant-turn-body">
        {props.children}
      </div>
    </div>
  );
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
            return node.safe && uiPreferences.externalMedia && safeUrl(node.url, true) ? <img class="semantic-image" src={node.url} alt={node.alt} title={node.title ?? undefined} loading="lazy" /> : <span>{node.alt || "Image unavailable"}</span>;
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
    case 1: return <h1 class="semantic-heading h1">{content()}</h1>;
    case 2: return <h2 class="semantic-heading h2">{content()}</h2>;
    case 3: return <h3 class="semantic-heading h3">{content()}</h3>;
    case 4: return <h4 class="semantic-heading h4">{content()}</h4>;
    case 5: return <h5 class="semantic-heading h5">{content()}</h5>;
    default: return <h6 class="semantic-heading h6">{content()}</h6>;
  }
}

function CodeBlock(props: { language?: string | null; filename?: string | null; content: string; diff?: boolean }) {
  let pre!: HTMLPreElement;
  let renderSeq = 0;
  createEffect(() => {
    const content = props.content;
    const language = props.language;
    void uiPreferences.theme;
    const seq = ++renderSeq;
    pre.textContent = content;
    pre.classList.remove("shiki");
    if (!language || props.diff) return;
    void highlight(content, languageForFence(language)).then(async (html) => {
      if (!html) {
        await new Promise((resolve) => window.setTimeout(resolve, 120));
        html = await highlight(content, languageForFence(language));
      }
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
  const copy = async (button: HTMLButtonElement) => {
    try {
      await navigator.clipboard.writeText(props.content);
      button.textContent = "Copied";
    } catch {
      button.textContent = "Copy failed";
    }
    setTimeout(() => (button.textContent = "Copy"), 900);
  };
  return (
    <div class="semantic-code" classList={{ diff: !!props.diff }}>
      <div class="semantic-code-head">
        <span>{props.filename ?? props.language ?? (props.diff ? "diff" : "text")}</span>
        <button type="button" onClick={(event) => copy(event.currentTarget)}>Copy</button>
      </div>
      <pre ref={pre}><code>{props.content}</code></pre>
    </div>
  );
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
            if (block.language === "vak" || block.language === "json") {
              const structured = parseVakFence(block.content);
              if (structured) return <StructuredView output={structured} fallback={block.content} />;
            }
            if (block.language === "mermaid") {
              return <MermaidViewer source={block.content} title={block.filename ?? undefined} />;
            }
            if (block.language === "diff") {
              return <DiffInspector rawDiff={block.content} filename={block.filename ?? undefined} />;
            }
            return <CodeBlock language={block.language} filename={block.filename} content={block.content} />;
          case "diff":
            return <DiffInspector rawDiff={block.content} />;
          case "structured":
            return <StructuredView output={block.output} fallback={block.fallback_markdown} />;
          case "diagram":
            return <MermaidViewer source={block.source} />;
          case "callout":
            return <section class={`semantic-callout ${block.tone}`}><Show when={block.title}><strong>{block.title}</strong></Show><Blocks blocks={block.blocks} recipeId={props.recipeId} /></section>;
          case "citations":
            return <ol class="semantic-citations"><For each={block.items}>{(citation) => <li><Show when={safeUrl(citation.url)} fallback={<span>{citation.label}</span>}><a href={citation.url} target="_blank" rel="noreferrer noopener">{citation.label}</a></Show></li>}</For></ol>;
          case "media":
            return safeUrl(block.source, true) ? (block.media_type?.startsWith("image/") && uiPreferences.externalMedia ? <img class="semantic-media" src={block.source} alt={block.alt} loading="lazy" /> : <a class="semantic-media-link" href={block.source} target="_blank" rel="noreferrer noopener">{block.alt || "Open media"}</a>) : <span class="semantic-unsafe-link">{block.alt || "Unsafe media omitted"}</span>;
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
  const outcomeStatus = () => props.document.metadata.outcome_status;
  const completion = () => props.document.metadata.outcome_completion;
  const outcomeLabel = () => {
    const execution = outcomeStatus();
    const done = completion();
    if (done && execution && done !== execution) return `Outcome: ${done} · execution ${execution}`;
    if (done) return `Outcome: ${done}`;
    if (execution) return `Outcome: ${execution}`;
    return "";
  };
  return (
    <div class="semantic-document">
      <Show when={showOperatorChrome() && outcomeLabel()}><div class={`semantic-outcome-status ${completion() || outcomeStatus()}`} role="status">{outcomeLabel()}</div></Show>
      <Show when={props.document.blocks.length === 0 && props.document.source_markdown}><div class="semantic-source">{props.document.source_markdown}</div></Show>
      <Blocks blocks={props.document.blocks} recipeId={recipeId()} />
      <Show when={showOperatorChrome()}>
        <For each={props.document.diagnostics}>{(diagnostic) => <div class="semantic-diagnostic">{diagnostic}</div>}</For>
      </Show>
      <Show when={showOperatorChrome()}>
        <RenderAudit document={props.document} />
      </Show>
    </div>
  );
}

function RenderAudit(props: { document: PresentationDocument }) {
  const metadata = props.document.metadata;
  const recipe = metadata.recipe_id;
  const outcome = metadata.outcome_objective;
  const evaluation = () => {
    if (!metadata.outcome_evaluation) return [];
    try {
      return JSON.parse(metadata.outcome_evaluation) as Array<{ requirement_id: string; status: string; reason: string }>;
    } catch {
      return [];
    }
  };
  return <Show when={recipe || outcome || props.document.diagnostics.length > 0}>
    <details class="semantic-render-audit">
      <summary>View details</summary>
      <dl>
        <Show when={outcome}><div><dt>Requested</dt><dd>{outcome}</dd></div></Show>
        <Show when={metadata.outcome_revision}><div><dt>Plan revision</dt><dd>{metadata.outcome_revision}</dd></div></Show>
        <Show when={metadata.outcome_requirements}><div><dt>Requirements</dt><dd>{metadata.outcome_requirements}</dd></div></Show>
        <Show when={metadata.outcome_completion}><div><dt>Completion</dt><dd>{metadata.outcome_completion}</dd></div></Show>
        <Show when={metadata.outcome_status}><div><dt>Execution</dt><dd>{metadata.outcome_status}</dd></div></Show>
        <Show when={metadata.outcome_evidence_receipts}><div><dt>Evidence receipts</dt><dd>{metadata.outcome_evidence_receipts}</dd></div></Show>
        <Show when={metadata.outcome_evidence_state}><div><dt>Evidence freshness</dt><dd>{metadata.outcome_evidence_state}</dd></div></Show>
        <Show when={metadata.outcome_human_review}><div><dt>Human review</dt><dd>{metadata.outcome_human_review}</dd></div></Show>
        <Show when={metadata.outcome_review_verdict}><div><dt>Review verdict</dt><dd>{metadata.outcome_review_verdict}</dd></div></Show>
        <Show when={metadata.outcome_requirement_rejections}><div><dt>Rejected checks</dt><dd>{metadata.outcome_requirement_rejections}</dd></div></Show>
        <Show when={evaluation().length > 0}><div><dt>Checks</dt><dd><For each={evaluation()}>{(item) => <div class={`semantic-check ${item.status}`}><strong>{item.requirement_id}: {item.status}</strong> — {item.reason}</div>}</For></dd></div></Show>
        <Show when={recipe}><div><dt>Recipe</dt><dd>{recipe} · {metadata.recipe_version ?? "unknown version"}</dd></div></Show>
        <Show when={metadata.renderer}><div><dt>Renderer</dt><dd>{metadata.renderer}</dd></div></Show>
        <Show when={metadata.renderer_blocks}><div><dt>Blocks</dt><dd>{metadata.renderer_blocks}</dd></div></Show>
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
      <Show when={path()}>{(value) => <button type="button" class="artifact-open" onClick={() => void openFileSmart(value())}>Open</button>}</Show>
    </article>
  );
}

function SemanticApproval(props: { item: OutputItem; sessionId: string }) {
  if (props.item.content.type !== "approval") return null;
  const content = props.item.content;
  const pending = () => props.item.status === "pending";
  return (
    <section
      class="approval semantic-approval"
      role={pending() ? "alert" : "status"}
      aria-live={pending() ? "assertive" : "polite"}
      aria-atomic="true"
      aria-label={`${pending() ? "Approval requested" : "Approval resolved"} for ${content.tool}`}
    >
      <div class="ap-head">Approval requested — {content.tool}</div>
      <div class="ap-reason">{content.reason}</div>
      <details class="ap-details"><summary>View request details</summary><pre class="ap-args">{content.args_json}</pre></details>
      <Show when={pending()} fallback={<div class="ap-done">{props.item.status}</div>}>
        <div class="ap-actions">
          <button type="button" class="btn primary" onClick={() => void approve(content.request_id, true, props.sessionId)}>Allow once</button>
          <button type="button" class="btn danger" onClick={() => void approve(content.request_id, false, props.sessionId)}>Deny</button>
        </div>
      </Show>
    </section>
  );
}

function OutcomeReviewActions(props: { item: OutputItem; sessionId: string }) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const review = async (verdict: "accepted" | "needs_work" | "rejected") => {
    setBusy(true);
    setError(null);
    try {
      const turn = props.item.actions.find((action) => action.data.verdict === verdict)?.data.turn;
      await api.recordOutcomeReview(props.sessionId, verdict, turn ? Number(turn) : undefined);
      window.location.reload();
    } catch (cause) {
      setError(`Could not record review: ${cause instanceof Error ? cause.message : String(cause)}`);
    } finally {
      setBusy(false);
    }
  };
  return <Show when={props.item.actions.length > 0}>
    <div class="outcome-review-actions" aria-label="Outcome review">
      <Show when={error()}><div class="error-state" role="alert">{error()}</div></Show>
      <For each={props.item.actions}>{(action) => {
        const verdict = action.data.verdict as "accepted" | "needs_work" | "rejected";
        const glyph = verdict === "accepted" ? "👍" : verdict === "rejected" ? "👎" : "↗";
        const label = verdict === "accepted" ? "Helpful" : verdict === "rejected" ? "Not helpful" : "Needs another pass";
        return <button type="button" class="outcome-review-button" aria-label={label} title={label} disabled={busy()} onClick={() => void review(verdict)}><span aria-hidden="true">{glyph}</span></button>;
      }}</For>
    </div>
  </Show>;
}

function ResultOutcomeSummary(props: { item: OutputItem }) {
  const outcome = () => props.item.outcome;
  return <Show when={showOperatorChrome() && outcome()}>{(value) => <div class={`result-outcome-summary ${value().status}`} role="status">
    <strong>{value().status === "partial" ? "Partial result" : `Result ${value().status}`}</strong>
    <Show when={value().completion}><span>Completion: {value().completion}</span></Show>
    <Show when={value().evidence_state}><span>Evidence: {value().evidence_state}</span></Show>
    <Show when={value().human_review}><span>Human review: {value().human_review}</span></Show>
  </div>}</Show>;
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

/**
 * Every assistant result gets the same rich answer surface, even when the
 * server did not classify it as research, a diff, a chart, or another
 * specialist semantic type. Specialist renderers can still add their own
 * cards; this is the universal replacement for the old unframed transcript.
 */
function AnswerCard(props: { item: OutputItem; document: PresentationDocument }) {
  return (
    <article class="semantic-answer-card">
      <header class="semantic-answer-head">
        <span class="semantic-answer-mark"><Icon name="chat" size={14} /></span>
        <strong>Answer</strong>
        <Show when={showOperatorChrome() && props.item.provenance?.source}><small>{props.item.provenance?.source}</small></Show>
      </header>
      <PresentationDocumentView document={props.document} />
    </article>
  );
}

function PresentationFeedback(props: { sessionId: string; semanticType: string }) {
  const [status, setStatus] = createSignal("");
  const [expanded, setExpanded] = createSignal(false);
  let input!: HTMLTextAreaElement;
  const send = async (choice: string) => {
    setStatus("Saving…");
    try {
      if (choice === "Use this layout") await api.selectPresentationForSemantic(props.sessionId, props.semanticType);
      await api.submitPresentationFeedback(props.sessionId, choice, input?.value.trim() || undefined);
      setStatus("Saved");
      setExpanded(false);
    } catch {
      setStatus("Could not save");
    }
  };
  return <section class="presentation-feedback" aria-label="Presentation feedback">
    <div class="presentation-feedback-actions">
      <button type="button" onClick={() => void send("Use this layout")}>Use this</button>
      <button type="button" onClick={() => void send("Keep original")}>Keep original</button>
      <button type="button" onClick={() => setExpanded(!expanded())}>Suggest a change</button>
      <Show when={status()}><small role="status">{status()}</small></Show>
    </div>
    <Show when={expanded()}>
      <div class="presentation-feedback-editor">
        <textarea ref={input} rows={2} placeholder="What should be clearer?" aria-label="Presentation feedback" />
        <button type="button" onClick={() => void send("Revise")}>Send feedback</button>
      </div>
    </Show>
  </section>;
}

export function StructuredView(props: { output: import("../types").StructuredOutput; fallback?: string; sessionId?: string }) {
  const [showOriginal, setShowOriginal] = createSignal(false);
  const fallback = () => <div class="semantic-source">{props.fallback || JSON.stringify(props.output.payload, null, 2)}</div>;
  return <ErrorBoundary fallback={() => <section><p role="status">Rich presentation unavailable. Original content:</p>{fallback()}</section>}>
    <Show when={uiPreferences.richPreviews && props.output.schema_version === 2 && props.output.payload && typeof props.output.payload === "object"} fallback={fallback()}>
      <Show when={showOriginal() && showOperatorChrome()} fallback={
        <>
          <StructuredRenderer output={props.output} />
          <Show when={showOperatorChrome() && props.sessionId}>
            <PresentationFeedback sessionId={props.sessionId!} semanticType={props.output.semantic_type} />
          </Show>
        </>
      }>
        <section class="presentation-original" aria-label="Original result"><p role="status">Original result</p>{fallback()}</section>
      </Show>
      <Show when={showOperatorChrome()}>
        <button type="button" class="presentation-original-toggle" onClick={() => setShowOriginal(!showOriginal())}>{showOriginal() ? "Show presentation" : "Show original"}</button>
      </Show>
    </Show>
  </ErrorBoundary>;
}

type StructuredRendererComponent = (props: { data: any; output: import("../types").StructuredOutput }) => JSX.Element;

function normalizeTimeline(data: any, defaultTitle = "Plan"): TimelineData {
  if (!data || typeof data !== "object") {
    return { title: defaultTitle, items: [] };
  }
  const title = String(data.title ?? data.label ?? data.name ?? defaultTitle);
  let rawItems: any[] = [];
  if (Array.isArray(data.items)) rawItems = data.items;
  else if (Array.isArray(data.steps)) rawItems = data.steps;
  else if (Array.isArray(data.milestones)) rawItems = data.milestones;
  else if (Array.isArray(data.slots)) rawItems = data.slots;
  else if (Array.isArray(data.agenda)) rawItems = data.agenda;
  else if (Array.isArray(data.tasks)) rawItems = data.tasks;
  else if (Array.isArray(data.choices)) rawItems = data.choices;
  else if (Array.isArray(data.questions)) rawItems = data.questions;
  else if (Array.isArray(data.qa)) rawItems = data.qa;
  else if (Array.isArray(data.entries)) rawItems = data.entries;
  else if (Array.isArray(data)) rawItems = data;
  else {
    const entries = Object.entries(data).filter(([k]) => !["title", "semantic_type", "label", "summary"].includes(k));
    if (entries.length > 0) {
      rawItems = entries.map(([k, v]) => ({
        label: k.replace(/_/g, " "),
        detail: typeof v === "object" ? JSON.stringify(v) : String(v ?? ""),
      }));
    }
  }

  const items = rawItems.map((it: any) => {
    if (typeof it === "string" || typeof it === "number") {
      return { label: String(it) };
    }
    if (it && typeof it === "object") {
      const label = String(it.label ?? it.title ?? it.name ?? it.question ?? it.task ?? it.text ?? it.choice ?? it.activity ?? "Item");
      const detail = it.detail ?? it.description ?? it.answer ?? it.notes ?? it.time ?? it.snippet ?? (it.reason ? String(it.reason) : undefined);
      const status = it.status ?? (typeof it.done === "boolean" ? (it.done ? "complete" : "pending") : undefined);
      return {
        label,
        detail: detail != null ? String(detail) : undefined,
        status: status != null ? String(status) : undefined,
      };
    }
    return { label: "Item" };
  });

  return { title, items };
}

function normalizeDataGrid(data: any, defaultTitle = "Dataset"): DataGridData {
  if (!data || typeof data !== "object") {
    return { title: defaultTitle, columns: [], rows: [] };
  }
  const title = String(data.title ?? data.label ?? data.name ?? defaultTitle);
  
  if (Array.isArray(data.columns) && Array.isArray(data.rows)) {
    const columns: DataGridColumn[] = data.columns.map((c: any) => ({
      key: String(c.key ?? c.name ?? c.label),
      label: String(c.label ?? c.name ?? c.key),
      isNumeric: Boolean(c.isNumeric || c.is_numeric),
    }));
    return { title, columns, rows: data.rows };
  }

  if (Array.isArray(data.pros) || Array.isArray(data.cons)) {
    const rows = [
      ...(data.pros || []).map((p: any) => ({ type: "Pro", point: typeof p === "string" ? p : (p.text ?? p.point ?? JSON.stringify(p)) })),
      ...(data.cons || []).map((c: any) => ({ type: "Con", point: typeof c === "string" ? c : (c.text ?? c.point ?? JSON.stringify(c)) })),
    ];
    return {
      title,
      columns: [
        { key: "type", label: "Type" },
        { key: "point", label: "Point" },
      ],
      rows,
    };
  }

  if (data.left != null || data.right != null) {
    const leftLabel = String(data.left_label ?? data.option_a ?? "Option A");
    const rightLabel = String(data.right_label ?? data.option_b ?? "Option B");
    const rows: Record<string, any>[] = [];
    if (typeof data.left === "object" && typeof data.right === "object") {
      const keys = Array.from(new Set([...Object.keys(data.left || {}), ...Object.keys(data.right || {})]));
      for (const k of keys) {
        rows.push({
          aspect: k.replace(/_/g, " "),
          left: typeof data.left[k] === "object" ? JSON.stringify(data.left[k]) : String(data.left[k] ?? "—"),
          right: typeof data.right[k] === "object" ? JSON.stringify(data.right[k]) : String(data.right[k] ?? "—"),
        });
      }
    } else {
      rows.push({ aspect: "Value", left: String(data.left ?? "—"), right: String(data.right ?? "—") });
    }
    return {
      title,
      columns: [
        { key: "aspect", label: "Aspect" },
        { key: "left", label: leftLabel },
        { key: "right", label: rightLabel },
      ],
      rows,
    };
  }

  const rawRows: any[] = Array.isArray(data.rows) ? data.rows : Array.isArray(data) ? data : Array.isArray(data.items) ? data.items : [];
  if (rawRows.length > 0 && typeof rawRows[0] === "object") {
    const keys = Array.from(new Set(rawRows.flatMap((r) => Object.keys(r || {}))));
    const columns: DataGridColumn[] = keys.map((k) => ({
      key: k,
      label: k.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
      isNumeric: rawRows.some((r) => typeof r[k] === "number"),
    }));
    return { title, columns, rows: rawRows };
  }

  const entries = Object.entries(data).filter(([k]) => !["title", "semantic_type", "label"].includes(k));
  if (entries.length > 0) {
    const rows = entries.map(([k, v]) => ({
      metric: k.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
      value: typeof v === "object" ? JSON.stringify(v) : v,
    }));
    return {
      title,
      columns: [
        { key: "metric", label: "Metric / Item" },
        { key: "value", label: "Value", isNumeric: entries.some(([, v]) => typeof v === "number") },
      ],
      rows,
    };
  }

  return { title, columns: [], rows: [] };
}

function normalizeResearch(data: any): ResearchData {
  if (!data || typeof data !== "object") {
    return { takeaways: [], sources: [] };
  }
  const title = data.title != null ? String(data.title) : undefined;
  const rawSources = Array.isArray(data.sources)
    ? data.sources
    : Array.isArray(data.references)
    ? data.references
    : Array.isArray(data.citations)
    ? data.citations
    : [];
  const sources = rawSources.map((s: any) => {
    if (typeof s === "string") return { title: s, url: s };
    return {
      title: String(s?.title ?? s?.name ?? s?.url ?? "Source"),
      url: String(s?.url ?? ""),
      snippet: s?.snippet != null ? String(s.snippet) : undefined,
      source_name: s?.source_name != null ? String(s.source_name) : undefined,
      published_at: s?.published_at != null ? String(s.published_at) : undefined,
    };
  });

  const rawTakeaways = Array.isArray(data.takeaways)
    ? data.takeaways
    : Array.isArray(data.findings)
    ? data.findings
    : Array.isArray(data.points)
    ? data.points
    : Array.isArray(data.items)
    ? data.items
    : [];
  const takeaways = rawTakeaways.map((t: any) => {
    if (typeof t === "string") return t;
    if (t && typeof t === "object") {
      return {
        text: String(t.text ?? t.point ?? t.claim ?? t.label ?? ""),
        citation_indices: Array.isArray(t.citation_indices) ? t.citation_indices : undefined,
      };
    }
    return String(t);
  });

  return { title, sources, takeaways };
}

function normalizeRecipe(data: any): RecipeData {
  if (!data || typeof data !== "object") {
    return { title: "Recipe", ingredients: [], steps: [] };
  }
  const rawIngredients = Array.isArray(data.ingredients)
    ? data.ingredients
    : Array.isArray(data.items)
    ? data.items
    : [];
  const ingredients = rawIngredients.map((item: any) => {
    if (typeof item === "string") return item;
    if (item && typeof item === "object") {
      const name = String(item.name ?? item.item ?? item.ingredient ?? item.label ?? "");
      const amount = typeof item.amount === "number" ? item.amount : typeof item.quantity === "number" ? item.quantity : undefined;
      const unit = item.unit ?? item.measurement ?? (typeof item.quantity === "string" ? item.quantity : undefined);
      return { name, amount, unit: unit != null ? String(unit) : undefined };
    }
    return String(item);
  });

  const rawSteps = Array.isArray(data.steps)
    ? data.steps
    : Array.isArray(data.instructions)
    ? data.instructions
    : Array.isArray(data.directions)
    ? data.directions
    : Array.isArray(data.method)
    ? data.method
    : [];
  const steps = rawSteps.map((step: any) => {
    if (typeof step === "string") return step;
    if (step && typeof step === "object") {
      const text = String(step.text ?? step.step ?? step.instruction ?? step.description ?? step.action ?? "");
      let timer_seconds = typeof step.timer_seconds === "number" ? step.timer_seconds : undefined;
      if (timer_seconds === undefined && typeof step.timer_minutes === "number") {
        timer_seconds = step.timer_minutes * 60;
      }
      if (timer_seconds === undefined && typeof step.duration_minutes === "number") {
        timer_seconds = step.duration_minutes * 60;
      }
      return { text, timer_seconds };
    }
    return String(step);
  });

  return {
    title: String(data.title ?? data.name ?? "Recipe"),
    servings: typeof data.servings === "number" ? data.servings : (typeof data.yield === "number" ? data.yield : undefined),
    prep_time_minutes: typeof data.prep_time_minutes === "number" ? data.prep_time_minutes : (typeof data.prep_time === "number" ? data.prep_time : undefined),
    cook_time_minutes: typeof data.cook_time_minutes === "number" ? data.cook_time_minutes : (typeof data.cook_time === "number" ? data.cook_time : undefined),
    ingredients,
    steps,
  };
}

// Every named semantic type resolves through this single registry. All 62+
// outcome types registered across core and plugin skills map directly here.
const STRUCTURED_RENDERERS: Record<string, StructuredRendererComponent> = {
  // Universal semantic shapes share a safe, lossless baseline renderer until
  // a richer domain-neutral interaction is available.
  "map": ({ data }) => <UniversalCard data={data} kind="Map" />,
  "route_map": ({ data }) => <UniversalCard data={data} kind="Route map" />,
  "calendar": ({ data }) => <UniversalCard data={data} kind="Calendar" />,
  "availability": ({ data }) => <UniversalCard data={data} kind="Availability" />,
  "board": ({ data }) => <UniversalCard data={data} kind="Board" />,
  "entity": ({ data }) => <UniversalCard data={data} kind="Entity" />,
  "search_results": ({ data }) => <UniversalCard data={data} kind="Search results" />,
  "evidence": ({ data }) => <UniversalCard data={data} kind="Evidence" />,
  "decision_analysis": ({ data }) => <UniversalCard data={data} kind="Decision" />,
  "document": ({ data }) => <UniversalCard data={data} kind="Document" />,
  "graph": ({ data }) => <UniversalCard data={data} kind="Graph" />,
  "form": ({ data }) => <UniversalCard data={data} kind="Form" />,
  "action": ({ data }) => <UniversalCard data={data} kind="Action" />,
  "transaction": ({ data }) => <UniversalCard data={data} kind="Transaction" />,
  "alert": ({ data }) => <UniversalCard data={data} kind="Alert" />,
  "conversation": ({ data }) => <UniversalCard data={data} kind="Conversation" />,
  "progress_dashboard": ({ data }) => <UniversalCard data={data} kind="Progress dashboard" />,
  "simulation": ({ data }) => <UniversalCard data={data} kind="Simulation" />,
  // 1. Research & Synthesis (research.synthesis, research_brief, research, news)
  "research.synthesis": ({ data }) => <ResearchCards data={normalizeResearch(data)} />,
  "research_brief": ({ data }) => <ResearchCards data={normalizeResearch(data)} />,
  "research": ({ data }) => <ResearchCards data={normalizeResearch(data)} />,
  "news": ({ data }) => <ResearchCards data={normalizeResearch(data)} />,

  // 2. Code, Tests & Terminal (coding.diff, test.report, terminal.view, etc.)
  "coding.diff": ({ data }) => <DiffInspector data={data} />,
  "diff": ({ data }) => <DiffInspector data={data} />,
  "test.report": ({ data }) => <TestMatrix data={data} />,
  "test": ({ data }) => <TestMatrix data={data} />,
  "ci.test_matrix": ({ data }) => <TestMatrix data={data} />,
  "test_matrix": ({ data }) => <TestMatrix data={data} />,
  "terminal.view": ({ data }) => <TerminalConsole data={data} />,
  "terminal": ({ data }) => <TerminalConsole data={data} />,
  "terminal.session": ({ data }) => <TerminalConsole data={data} />,

  // 3. Coding Specific Flows
  "coding.benchmark": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Benchmark Results")} />,
  "coding.dependencies": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Dependencies")} />,
  "coding.deployment": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Deployment")} kicker="Deployment" />,
  "coding.incident": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Incident Summary")} kicker="Incident" />,
  "coding.architecture": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Architecture Decisions")} kicker="Architecture" />,
  "coding.release": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Release Notes")} kicker="Release" />,
  "coding.search": ({ data }) => (Array.isArray(data?.results) || Array.isArray(data?.rows)) ? <DataGrid data={normalizeDataGrid(data, "Search Results")} /> : <ResearchCards data={normalizeResearch(data)} />,

  // 4. Data Grids, Tables & Comparisons
  "data.grid": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Data Grid")} />,
  "table": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Table")} />,
  "dataset": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Dataset")} />,
  "comparison": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Comparison")} />,
  "comparison_table": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Comparison Table")} />,
  "pros_cons": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Pros & Cons")} />,
  "inventory": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Inventory")} />,
  "scorecard": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Scorecard")} />,

  // 5. Financial Summaries & Budgets
  "budget": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Budget Breakdown")} />,
  "finance_summary": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Financial Summary")} />,
  "invoice_summary": ({ data }) => <DataGrid data={normalizeDataGrid(data, "Invoice Summary")} />,

  // 6. Culinary & Lifestyle
  "recipe.card": ({ data }) => <RecipeCard data={normalizeRecipe(data)} />,
  "recipe": ({ data }) => <RecipeCard data={normalizeRecipe(data)} />,
  "lifestyle.recipe": ({ data }) => <RecipeCard data={normalizeRecipe(data)} />,
  "lifestyle.culinary_recipe": ({ data }) => <RecipeCard data={normalizeRecipe(data)} />,
  "recipe_summary": ({ data }) => <RecipeCard data={normalizeRecipe(data)} />,
  "meal_plan": ({ data }) => Array.isArray(data?.steps) || Array.isArray(data?.ingredients) ? <RecipeCard data={normalizeRecipe(data)} /> : <TimelineCard data={normalizeTimeline(data, "Meal Plan")} kicker="Meal Plan" />,

  // 7. Interactive Previews
  "ui.preview": ({ data }) => <UIPreviewCard data={data} />,
  "preview": ({ data }) => <UIPreviewCard data={data} />,

  // 8. Timelines, Plans, Checklists, Schedules & Notes
  "plan.timeline": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Plan Timeline")} kicker="Plan" />,
  "timeline": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Timeline")} kicker="Timeline" />,
  "itinerary": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Itinerary")} kicker="Itinerary" />,
  "checklist": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Checklist")} kicker="Checklist" />,
  "schedule": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Schedule")} kicker="Schedule" />,
  "agenda": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Agenda")} kicker="Agenda" />,
  "milestones": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Milestones")} kicker="Milestones" />,
  "progress": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Progress Tracking")} kicker="Progress" />,
  "status": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Status")} kicker="Status" />,
  "steps": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Steps")} kicker="Steps" />,
  "overview": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Overview")} kicker="Overview" />,
  "summary": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Summary")} kicker="Summary" />,
  "detail": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Details")} kicker="Detail" />,
  "notes": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Notes")} kicker="Notes" />,
  "follow_up": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Follow-Up Items")} kicker="Follow-Up" />,
  "reminder": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Reminders")} kicker="Reminder" />,
  "shopping_list": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Shopping List")} kicker="Shopping List" />,
  "lesson": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Lesson Plan")} kicker="Lesson" />,
  "reading_list": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Reading List")} kicker="Reading List" />,
  "habit_plan": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Habit Plan")} kicker="Habit" />,
  "project_plan": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Project Plan")} kicker="Project" />,
  "meeting_notes": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Meeting Notes")} kicker="Meeting" />,
  "contact_log": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Contact Log")} kicker="Contact" />,
  "travel_options": ({ data }) => (Array.isArray(data?.rows) || Array.isArray(data?.columns)) ? <DataGrid data={normalizeDataGrid(data, "Travel Options")} /> : <TimelineCard data={normalizeTimeline(data, "Travel Options")} kicker="Travel" />,
  "home_project": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Home Project")} kicker="Project" />,
  "care_plan": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Care Plan")} kicker="Care Plan" />,
  "event_plan": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Event Plan")} kicker="Event" />,
  "media_list": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Media List")} kicker="Media" />,
  "collection": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Collection")} kicker="Collection" />,
  "faq": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Frequently Asked Questions")} kicker="FAQ" />,
  "decision": ({ data }) => <TimelineCard data={normalizeTimeline(data, "Decision Analysis")} kicker="Decision" />,

  // 9. Visualizations & Charts
  "chart": ({ data }) => Array.isArray(data?.series) ? <UniversalChart data={data} /> : <></>,
  "telemetry.chart": ({ data }) => Array.isArray(data?.series) ? <UniversalChart data={data} /> : <></>,

  // 10. Media & Links
  "link.preview": ({ data }) => (
    <a class="rich-link-card" href={safeUrl(data.url) ? data.url : undefined} target="_blank" rel="noreferrer noopener">
      <Show when={uiPreferences.externalMedia && typeof data.image_url === "string" && safeUrl(data.image_url, true)}><img src={data.image_url as string} alt="" loading="lazy" /></Show>
      <span><strong>{String(data.title ?? data.url)}</strong><small>{String(data.description ?? data.site_name ?? data.url)}</small></span>
    </a>
  ),
  "media.image": ({ data }) => (
    <Show when={uiPreferences.externalMedia && typeof data?.source === "string" && safeUrl(data.source, true)}>
      <figure class="rich-media"><img src={data.source} alt={String(data.alt ?? "")} /><Show when={data.alt}><figcaption>{String(data.alt)}</figcaption></Show></figure>
    </Show>
  ),
  "media.video": ({ data }) => (
    <Show when={uiPreferences.externalMedia && typeof data?.source === "string" && safeUrl(data.source, true)}>
      <video class="rich-video" src={data.source} controls preload="metadata" autoplay={uiPreferences.autoplayMedia} aria-label={String(data.alt ?? "Video")} />
    </Show>
  ),
  "media.audio": ({ data }) => (
    <Show when={uiPreferences.externalMedia && typeof data?.source === "string" && safeUrl(data.source, true)}>
      <audio class="rich-audio" src={data.source} controls preload="metadata" aria-label={String(data.alt ?? "Audio")} />
    </Show>
  ),

  // 11. Metrics & Weather
  "weather": (props) => STRUCTURED_RENDERERS.metric(props),
  "telemetry.metric": (props) => STRUCTURED_RENDERERS.metric(props),
  "metric": ({ data }) => {
    if (typeof data?.label === "string" || typeof data?.value === "string" || typeof data?.value === "number") {
      return (
        <div class="rich-metric">
          <small>{String(data.label ?? "Metric")}</small>
          <strong>{String(data.value ?? "—")}{data.unit ? ` ${String(data.unit)}` : ""}</strong>
        </div>
      );
    }
    const entries = Object.entries(data || {}).filter(([k]) => k !== "title" && k !== "semantic_type");
    if (entries.length > 0) {
      return (
        <div class="canvas-card metric-grid-card">
          <Show when={data.location || data.title || data.label}>
            <div class="metric-grid-header">
              {String(data.location ?? data.title ?? data.label)}
            </div>
          </Show>
          <div class="metric-grid-container">
            <For each={entries}>
              {([key, val]) => (
                <div class="metric-grid-item">
                  <small class="metric-grid-label">{key.replace(/_/g, " ")}</small>
                  <strong class="metric-grid-value">{typeof val === "object" ? JSON.stringify(val) : String(val)}</strong>
                </div>
              )}
            </For>
          </div>
        </div>
      );
    }
    return <div class="rich-metric"><small>{String(data?.label ?? "Metric")}</small><strong>{String(data?.value ?? "—")}{data?.unit ? ` ${String(data.unit)}` : ""}</strong></div>;
  },
};

function StructuredRenderer(props: { output: import("../types").StructuredOutput }) {
  const payload = props.output.payload as any;
  const semanticType = props.output.semantic_type.toLowerCase();
  const registered = STRUCTURED_RENDERERS[semanticType];
  if (registered) {
    return registered({ data: payload, output: props.output });
  }
  // Rich rendering is protocol-driven. An unknown semantic type must not be
  // guessed from payload shape: that lets a new contract silently take over
  // an older card and makes regressions invisible.
  return (
    <div class="rich-structured-card">
      <div class="rich-structured-header"><strong>Presentation unavailable</strong><small>{props.output.semantic_type}</small></div>
      <p class="rich-structured-summary">No renderer is registered for this semantic type. The original result is preserved below.</p>
      <div class="semantic-source">{JSON.stringify(payload, null, 2)}</div>
    </div>
  );
}


function Turn(props: { id: string; items: OutputItem[]; sessionId: string }) {
  const ordered = () => props.items.filter((item) => (presentationMode() === "everyday" ? "outcome" : density()) !== "outcome" || !["progress", "retry", "information"].includes(item.kind));
  const OrderedItem = (item: OutputItem): JSX.Element | null => {
    if (item.role === "user" && item.content.type === "document") {
      return <div class="semantic-user"><PresentationDocumentView document={item.content.document} /></div>;
    }
    if (item.kind === "approval") return <SemanticApproval item={item} sessionId={props.sessionId} />;
    if (item.content.type === "document") return showOperatorChrome()
      ? <article class="semantic-outcome"><ResultOutcomeSummary item={item} /><Show when={item.role === "assistant"} fallback={<PresentationDocumentView document={item.content.document} />}><AnswerCard item={item} document={item.content.document} /></Show></article>
      : item.role === "user" ? <div class="semantic-user"><PresentationDocumentView document={item.content.document} /></div> : <AssistantMessage><PresentationDocumentView document={item.content.document} /></AssistantMessage>;
    if (item.content.type === "outcome") {
      if (showOperatorChrome()) {
        return (
          <article class="semantic-outcome">
            <ResultOutcomeSummary item={item} />
            {item.content.document ? <PresentationDocumentView document={item.content.document} /> : <p>{item.content.summary}</p>}
            <OutcomeReviewActions item={item} sessionId={props.sessionId} />
          </article>
        );
      }
      // Never leak internal lifecycle metadata (e.g. "primary deliverable: produced", "completed") as assistant prose
      if (!item.content.document) return null;
      return <AssistantMessage><PresentationDocumentView document={item.content.document} /></AssistantMessage>;
    }
    if (item.content.type === "structured") return <StructuredView output={item.content.output} fallback={item.fallback_text} sessionId={props.sessionId} />;
    if (item.content.type === "adaptive") return <AdaptiveTreeView tree={item.content.tree} fallback={item.content.fallback_text} />;
    if (item.kind === "error") {
      const isRawJson = item.fallback_text.trim().startsWith("{") || item.fallback_text.includes('"type":');
      const text = !showOperatorChrome() && isRawJson
        ? (item.fallback_text.includes("unknown_capability")
            ? "The requested capability or web integration is currently unavailable for this query."
            : "Tool execution required attention.")
        : item.fallback_text;
      return <section class="semantic-recovery" role="alert"><Icon name="warning" size={15} /><div><strong>{item.status === "partial" ? "Partial outcome" : "Run needs attention"}</strong><p>{text}</p></div></section>;
    }
    if (item.kind === "artifact") return <section class="artifact-shelf" aria-label="Artifact"><Artifact item={item} /></section>;
    if (["progress", "retry", "information"].includes(item.kind)) return <ActivityRow item={item} />;
    // Lifecycle summaries and scaffolding fallback items must not leak as assistant prose
    const cleanFallback = stripControlScaffolding(item.fallback_text).trim();
    if (!cleanFallback) return null;

    if (!showOperatorChrome()) {
      const lines = cleanFallback.split("\n").map((l) => l.trim()).filter(Boolean);
      if (
        lines.length === 0 ||
        lines.every((line) =>
          /^(?:primary deliverable\s*:|completed$|Surface:\s+|Outcome:\s+|contract_id:)/i.test(line)
        )
      ) {
        return null;
      }
    }
    return <div class="semantic-source">{cleanFallback}</div>;
  };
  return (
    <section class="semantic-turn" data-turn={props.id}>
      <For each={ordered()}>{(item) => OrderedItem(item)}</For>
    </section>
  );
}

function AdaptiveTreeView(props: { tree: import("../types").AdaptiveRenderTree; fallback: string }) {
  const render = (node: import("../types").AdaptiveRenderNode): JSX.Element => {
    const text = typeof node.props.text === "string" ? node.props.text : typeof node.props.value === "string" ? String(node.props.value) : "";
    const label = typeof node.props.label === "string" ? node.props.label : "";
    const title = typeof node.props.title === "string" ? node.props.title : "";
    const content = (
      <>
        {title && <h4 class="adaptive-node-title">{title}</h4>}
        {label && <strong class="adaptive-node-label">{label}</strong>}
        {text && <span class="adaptive-node-text">{text}</span>}
        {node.children.map(render)}
      </>
    );
    switch (node.primitive.toLowerCase()) {
      case "title":
        return <h3 class="adaptive-node adaptive-title">{title || text}</h3>;
      case "text":
      case "richtext":
        return <p class="adaptive-node adaptive-text">{text || content}</p>;
      case "section":
        return <section class="adaptive-node adaptive-section">{content}</section>;
      case "stack":
        return <div class="adaptive-node adaptive-stack">{node.children.map(render)}</div>;
      case "row":
        return <div class="adaptive-node adaptive-row">{node.children.map(render)}</div>;
      case "list":
        return <ul class="adaptive-node adaptive-list">{node.children.length ? node.children.map((child) => <li>{render(child)}</li>) : <li>{content}</li>}</ul>;
      case "checklist":
        return <ul class="adaptive-node adaptive-checklist">{node.children.length ? node.children.map((child) => <li class="adaptive-checklist-item"><span class="adaptive-check-marker">✓</span>{render(child)}</li>) : <li>{content}</li>}</ul>;
      case "steps":
        return <ol class="adaptive-node adaptive-steps">{node.children.length ? node.children.map((child) => <li>{render(child)}</li>) : <li>{content}</li>}</ol>;
      case "timeline":
        return (
          <div class="adaptive-node adaptive-timeline">
            {title && <h4 class="adaptive-timeline-title">{title}</h4>}
            <div class="adaptive-timeline-items">
              {node.children.map((child) => (
                <div class="adaptive-timeline-step">
                  <div class="adaptive-timeline-bullet" />
                  <div class="adaptive-timeline-content">{render(child)}</div>
                </div>
              ))}
            </div>
          </div>
        );
      case "comparison":
        return (
          <div class="adaptive-node adaptive-comparison">
            {title && <h4 class="adaptive-comparison-title">{title}</h4>}
            <div class="adaptive-comparison-grid">{node.children.map(render)}</div>
          </div>
        );
      case "keyvalue":
        return <div class="adaptive-node adaptive-keyvalue">{label && <span class="adaptive-kv-label">{label}</span>}{text && <span class="adaptive-kv-value">{text}</span>}{node.children.map(render)}</div>;
      case "metric":
        return <div class="adaptive-node adaptive-metric">{label && <small>{label}</small>}<strong>{text || String(node.props.value ?? "—")}</strong></div>;
      case "progress":
        return <div class="adaptive-node adaptive-progress"><progress value={Number(node.props.value ?? 0)} max={Number(node.props.max ?? 100)} />{label && <span>{label}</span>}</div>;
      case "badge":
      case "label":
        return <span class="adaptive-node adaptive-badge">{label || text}</span>;
      case "callout":
        return <aside class="adaptive-node adaptive-callout">{label && <strong>{label}</strong>}{text && <p>{text}</p>}{node.children.map(render)}</aside>;
      case "quote":
        return <blockquote class="adaptive-node adaptive-quote">{text || content}</blockquote>;
      case "divider":
        return <hr class="adaptive-node adaptive-divider" />;
      case "disclosure":
        return <details class="adaptive-node adaptive-disclosure"><summary>{title || label || "Details"}</summary>{content}</details>;
      default:
        return <div class={`adaptive-node adaptive-${node.primitive.toLowerCase()}`}>{content}</div>;
    }
  };
  return (
    <section class="adaptive-presentation" aria-label={props.tree.accessibility_summary ?? "Adaptive presentation"}>
      {render(props.tree.root)}
      <details class="adaptive-fallback-toggle">
        <summary>Show original</summary>
        <p>{props.fallback}</p>
      </details>
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
  return <div class="semantic-timeline">
    <Show when={showOperatorChrome() && props.timeline.goal}>{(goal) => <details class="goal-state" open={goal().control !== "active"}>
      <summary><span class="goal-state-label">{goal().control === "active" ? "Follow-up goal" : "Goal record"}</span><span class={`goal-state-control ${goal().control}`}>{goal().control === "active" ? "available" : goal().control}</span><span class="goal-state-revision">rev {goal().revision}</span></summary>
      <p class="goal-state-objective">{goal().objective}</p>
      <Show when={goal().additions.length}><ul><For each={goal().additions}>{(addition) => <li>{addition}</li>}</For></ul></Show>
      <Show when={goal().superseded_revisions.length}><small>Superseded revisions: {goal().superseded_revisions.join(", ")}</small></Show>
    </details>}</Show>
    <For each={turns()}>{(turn) => <Turn id={turn.id} items={turn.items} sessionId={props.sessionId} />}</For>
  </div>;
}
