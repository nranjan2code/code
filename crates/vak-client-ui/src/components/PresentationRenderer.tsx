import { createEffect, createMemo, createResource, createSignal, ErrorBoundary, For, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";
import type {
  ArtifactRef,
  DocumentBlock,
  InlineNode,
  OutputItem,
  OutputTimeline,
  PresentationDocument,
} from "../types";
import {
  activeId,
  agentForSession,
  openInEditor,
  openWorkbenchArtifact,
  uiPreferences,
  isPreviewableArtifact,
  openArtifactFile,
  openArtifactCanvas,
  openCandidateReview,
  openOfficeCitation,
  setReplyTarget,
  isRunning,
  presentationOf,
  technicalDetails,
} from "../store";
import MarkdownView from "./MarkdownView";
import MessageActions from "./MessageActions";
import { approve, sendPrompt } from "../App";
import Icon from "./Icon";
import { safeUrl, isLocalArtifactPath, sandboxedSrcdoc } from "../safeUrl";
import { fileSubject, runOrigin } from "../canvasSubject";
import { artifactPreviewHtml, type ArtifactPreviewReader } from "../artifactPreview";
import { fileKind, newestWaitingDraft, statusWords } from "../resultCard";
import { relAgo } from "../time";
import Skeleton from "./Skeleton";
import * as api from "../api";
import { host } from "../host";
import { isDocumentPath, parseOfficeCitation } from "../officeFiles";
import { officeFactsLine, officeFlagsLabel } from "../officeFacts";
import { highlight, languageForFence } from "../highlight";
import DiffInspector from "./presentation/DiffInspector";
import { downloadCsv } from "./presentation/data";
import MermaidViewer from "./presentation/MermaidViewer";

// Technical details are a disclosure preference only; they never change the
// result, route, authority, or which safety states remain visible.
const showOperatorChrome = () => technicalDetails();
import GenericSpecRenderer, { PresentationInteractionContext, buildTimelineSpec, buildMetricSpec, buildTableSpec, buildOptionsTableSpec, buildRecipeSpec, buildResearchSpec, buildDiffSpec, buildTerminalSpec, buildTestMatrixSpec, buildChartSpec, buildUiPreviewSpec, buildMediaSpec, buildUniversalCardSpec } from "./presentation/GenericSpecRenderer";
import AgentMark from "./AgentMark";
import { assistantParts, groupAssistantParts, isFleetingNarration, parseVakFence, stripControlScaffolding } from "../structured";
import type { AssistantPart } from "../structured";

/** Renders a run of `AssistantPart`s (text + cards) with adjacent cards
 * grouped into a connected layout instead of independent stacked blocks —
 * shared by every render site that replays `assistantParts()` output
 * (streamed paragraphs, raw_markdown fallbacks, and whole-document view). */
type PresentationDocumentContext = { sessionId?: string; resultId?: string; presentationId?: string };

function AssistantPartsView(props: { parts: AssistantPart[]; textWrap?: (text: string) => JSX.Element } & PresentationDocumentContext) {
  const groups = createMemo(() => groupAssistantParts(props.parts));
  const wrapText = (text: string) => (props.textWrap ? props.textWrap(text) : <p class="semantic-paragraph">{text}</p>);
  return (
    <For each={groups()}>
      {(group) =>
        group.type === "text" ? (
          wrapText(group.text)
        ) : group.cards.length === 1 ? (
          <StructuredView output={group.cards[0].output} fallback={group.cards[0].source} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />
        ) : (
          <div class="card-group" style={{ "--card-group-count": group.cards.length }}>
            <For each={group.cards}>
              {(card) => (
                <div class="card-group-item">
                  <StructuredView output={card.output} fallback={card.source} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />
                </div>
              )}
            </For>
          </div>
        )
      }
    </For>
  );
}

/** Wraps settled assistant content with the same Vakyartha avatar + name header
 *  that the streaming transcript uses, so completed turns don't lose their
 *  visual identity when ChatPane switches to PresentationTimelineView. */
function AssistantMessage(props: { children: JSX.Element; text?: string; sessionId?: string }) {
  return (
    <div class="semantic-assistant">
      <div class="assistant-turn-head">
        <AgentMark character={agentForSession(props.sessionId ?? activeId()).character} motion={agentForSession(props.sessionId ?? activeId()).animation} size={26} class="assistant-avatar-mark" />
        <span class="assistant-name">{agentForSession(props.sessionId ?? activeId()).name}</span>
      </div>
      <div class="assistant-turn-body">
        {props.children}
        <Show when={props.text}><MessageActions text={props.text!} role="assistant" /></Show>
      </div>
    </div>
  );
}

function UserMessage(props: { document: PresentationDocument; text: string }) {
  return <div class="msg user"><div class="msg-bubble-wrap">
    <div class="user-turn-head"><span class="turn-author-chip">You</span></div>
    <div class="msg-user-content"><PresentationDocumentView document={props.document} /></div>
    <MessageActions text={props.text} role="user" />
  </div></div>;
}

function inlineNodesToText(nodes: InlineNode[]): string {
  let out = "";
  for (const node of nodes) {
    switch (node.type) {
      case "text": out += node.text; break;
      case "soft_break":
      case "hard_break": out += "\n"; break;
      case "code": out += node.code; break;
      case "strong":
      case "emphasis":
      case "strikethrough":
        out += inlineNodesToText(node.content);
        break;
      case "link":
        out += inlineNodesToText(node.label);
        break;
      default: break;
    }
  }
  return out;
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
            const citation = parseOfficeCitation(node.code);
            if (!citation) return <code class="ic">{node.code}</code>;
            return <button type="button" class="ic office-cite" title={`Open ${citation.path} at ${citation.anchor}`} onClick={() => openOfficeCitation(citation)}>{node.code}</button>;
          }
          case "link": {
            if (node.safe && safeUrl(node.url)) {
              return (
                <a class="semantic-link" href={node.url} target="_blank" rel="noreferrer noopener" title={node.title ?? node.url}>
                  <InlineSequence nodes={node.label} />
                </a>
              );
            }
            if (isLocalArtifactPath(node.url)) return <span class="semantic-artifact-link"><InlineSequence nodes={node.label} /></span>;
            return (
              <span class="semantic-unsafe-link" title="Unsafe link omitted">
                <InlineSequence nodes={node.label} />
              </span>
            );
          }
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

  const canPreview = () => {
    const lang = (props.language || "").toLowerCase();
    const name = (props.filename || "").toLowerCase();
    if (props.diff) return false;
    return (
      lang === "html" ||
      lang === "xhtml" ||
      lang === "svg" ||
      name.endsWith(".html") ||
      name.endsWith(".htm") ||
      name.endsWith(".svg") ||
      /<!doctype\s+html/i.test(props.content) ||
      /<html[\s>]/i.test(props.content) ||
      /<svg[\s>]/i.test(props.content)
    );
  };

  const openPreview = () => {
    const title = props.filename || (props.language ? `${props.language.toUpperCase()} Preview` : "Artifact Preview");
    openArtifactCanvas({ kind: "inline", title, html: props.content });
  };

  return (
    <div class="semantic-code" classList={{ diff: !!props.diff }}>
      <div class="semantic-code-head">
        <span>{props.filename ?? props.language ?? (props.diff ? "diff" : "text")}</span>
        <div style={{ display: "flex", gap: "6px", "margin-left": "auto" }}>
          <Show when={canPreview()}>
            <button type="button" onClick={openPreview} title="Open interactive preview in Artifact Canvas">Preview</button>
          </Show>
          <button type="button" onClick={(event) => copy(event.currentTarget)}>Copy</button>
        </div>
      </div>
      <pre ref={pre}><code>{props.content}</code></pre>
    </div>
  );
}

function inlineToText(nodes: InlineNode[]): string {
  let s = "";
  for (const n of nodes) {
    if (n.type === "text") s += n.text;
    else if ("content" in n && Array.isArray((n as any).content)) s += inlineToText((n as any).content);
    else if ("code" in n) s += (n as any).code;
    else if ("label" in n && Array.isArray((n as any).label)) s += inlineToText((n as any).label);
  }
  return s;
}

function InteractiveTable(props: {
  header: InlineNode[][];
  rows: InlineNode[][][];
  alignments: string[];
}) {
  const [search, setSearch] = createSignal("");
  const [sortCol, setSortCol] = createSignal<number | null>(null);
  const [sortAsc, setSortAsc] = createSignal<boolean>(true);
  const [downloaded, setDownloaded] = createSignal(false);

  const headerTexts = createMemo(() => props.header.map((cell) => inlineToText(cell)));

  const handleSort = (colIndex: number) => {
    if (sortCol() === colIndex) {
      if (sortAsc()) {
        setSortAsc(false);
      } else {
        setSortCol(null);
        setSortAsc(true);
      }
    } else {
      setSortCol(colIndex);
      setSortAsc(true);
    }
  };

  const processedRows = createMemo(() => {
    let list = props.rows.map((row, originalIndex) => {
      const texts = row.map((cell) => inlineToText(cell));
      return { row, texts, originalIndex };
    });

    const q = search().toLowerCase().trim();
    if (q) {
      list = list.filter((item) =>
        item.texts.some((t) => t.toLowerCase().includes(q))
      );
    }

    const col = sortCol();
    if (col !== null) {
      list.sort((a, b) => {
        const textA = a.texts[col] ?? "";
        const textB = b.texts[col] ?? "";
        const numA = Number(textA.replace(/[$,%]/g, "").trim());
        const numB = Number(textB.replace(/[$,%]/g, "").trim());
        if (!Number.isNaN(numA) && !Number.isNaN(numB)) {
          return sortAsc() ? numA - numB : numB - numA;
        }
        return sortAsc()
          ? textA.localeCompare(textB)
          : textB.localeCompare(textA);
      });
    }

    return list;
  });

  const handleDownloadCsv = () => {
    const headers = headerTexts();
    const rows = processedRows().map((r) => r.texts);
    downloadCsv("table.csv", [headers, ...rows]);
    setDownloaded(true);
    setTimeout(() => setDownloaded(false), 1200);
  };

  const isInteractive = () => props.rows.length >= 2 || props.header.length >= 3;

  return (
    <div class="semantic-table-wrap interactive-table-wrap">
      <Show when={isInteractive()}>
        <div class="table-toolbar" style={{ display: "flex", "align-items": "center", "justify-content": "space-between", "margin-bottom": "6px", gap: "8px" }}>
          <div style={{ display: "flex", "align-items": "center", gap: "6px" }}>
            <input
              type="text"
              class="grid-search-input"
              placeholder="Filter table..."
              style={{ "font-size": "var(--fs-control)", padding: "5px 8px", width: "min(220px, 42vw)", "border-radius": "var(--radius-sm)" }}
              value={search()}
              onInput={(e) => setSearch(e.currentTarget.value)}
              aria-label="Filter table rows"
            />
            <span style={{ "font-size": "var(--fs-caption)", color: "var(--muted)" }}>
              {processedRows().length} of {props.rows.length} rows
            </span>
          </div>
          <button
            type="button"
            class="pill-action-btn"
            style={{ "font-size": "var(--fs-control)", padding: "5px 10px" }}
            onClick={handleDownloadCsv}
          >
            {downloaded() ? "Downloaded" : "CSV"}
          </button>
        </div>
      </Show>
      <table class="semantic-table">
        <thead>
          <tr>
            <For each={props.header}>
              {(cell, index) => (
                <th
                  style={{
                    "text-align":
                      props.alignments[index()] === "right"
                        ? "right"
                        : props.alignments[index()] === "center"
                        ? "center"
                        : "left",
                    cursor: "pointer",
                    "user-select": "none",
                  }}
                  onClick={() => handleSort(index())}
                  title="Click to sort"
                >
                  <InlineSequence nodes={cell} />
                  <span style={{ "font-size": "10px", "margin-left": "4px", opacity: "0.6" }}>
                    {sortCol() === index() ? (sortAsc() ? " ▲" : " ▼") : " ↕"}
                  </span>
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={processedRows()}>
            {(item) => (
              <tr>
                <For each={item.row}>
                  {(cell, index) => (
                    <td
                      style={{
                        "text-align":
                          props.alignments[index()] === "right"
                            ? "right"
                            : props.alignments[index()] === "center"
                            ? "center"
                            : "left",
                      }}
                    >
                      <InlineSequence nodes={cell} />
                    </td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

function Blocks(props: { blocks: DocumentBlock[]; recipeId?: string } & PresentationDocumentContext): JSX.Element {
  return (
    <For each={props.blocks}>
      {(block) => {
        switch (block.type) {
          case "heading":
            return <Heading block={block} />;
          case "paragraph": {
            const raw = inlineNodesToText(block.content);
            if (raw.includes('"semantic_type"')) {
              const parts = assistantParts(raw);
              if (parts.some((p) => p.type === "card")) {
                return <AssistantPartsView parts={parts} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />;
              }
            }
            return <p class="semantic-paragraph"><InlineSequence nodes={block.content} /></p>;
          }
          case "list": {
            const items = () => <For each={block.items}>{(item) => <li><Blocks blocks={item} recipeId={props.recipeId} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} /></li>}</For>;
            return block.ordered ? <ol class="semantic-list" start={block.start ?? undefined}>{items()}</ol> : <ul class="semantic-list">{items()}</ul>;
          }
          case "table": {
            return (
              <InteractiveTable
                header={block.header}
                rows={block.rows}
                alignments={block.alignments}
              />
            );
          }
          case "quote":
            return <blockquote class="semantic-quote"><Blocks blocks={block.blocks} recipeId={props.recipeId} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} /></blockquote>;
          case "code":
            if (block.language === "vak" || block.language === "json" || block.content.includes('"semantic_type"')) {
              const structured = parseVakFence(block.content);
              if (structured) return <StructuredView output={structured} fallback={block.content} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />;
              // If it has vak language or semantic_type, attempt recovery and never leak as a raw code block
              if (block.language === "vak" || block.content.includes('"semantic_type"')) {
                const match = block.content.match(/\{[\s\S]*"semantic_type"[\s\S]*\}/);
                if (match) {
                  const recovered = parseVakFence(match[0]);
                  if (recovered) return <StructuredView output={recovered} fallback={block.content} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />;
                }
                // A genuinely malformed vak/semantic-type fence (the model
                // emitted invalid JSON, e.g. mismatched brackets) used to
                // `return null` here whenever operator chrome was off. That
                // produced a real answer that rendered as
                // total silence: no card, no error, no raw JSON, nothing.
                // Same fallback contract as assistantParts()'s equivalent
                // fix (structured.ts) — never leave a parse failure with no
                // trace at all.
                return (
                  <p class="semantic-paragraph" style={{ opacity: 0.7, "font-style": "italic" }}>
                    This response could not be rendered — the result was malformed.
                  </p>
                );
              }
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
            return <StructuredView output={block.output} fallback={block.fallback_markdown} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />;
          case "diagram":
            return <MermaidViewer source={block.source} />;
          case "callout":
            return <section class={`semantic-callout ${block.tone}`}><Show when={block.title}><strong>{block.title}</strong></Show><Blocks blocks={block.blocks} recipeId={props.recipeId} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} /></section>;
          case "citations":
            return <ol class="semantic-citations"><For each={block.items}>{(citation) => <li><Show when={safeUrl(citation.url)} fallback={<span>{citation.label}</span>}><a href={citation.url} target="_blank" rel="noreferrer noopener">{citation.label}</a></Show></li>}</For></ol>;
          case "media":
            return safeUrl(block.source, true) ? (block.media_type?.startsWith("image/") && uiPreferences.externalMedia ? <img class="semantic-media" src={block.source} alt={block.alt} loading="lazy" /> : <a class="semantic-media-link" href={block.source} target="_blank" rel="noreferrer noopener">{block.alt || "Open media"}</a>) : <span class="semantic-unsafe-link">{block.alt || "Unsafe media omitted"}</span>;
          case "artifact_ref":
            return <Artifact item={{ id: block.id, turn_id: "", timestamp: "", role: "assistant", kind: "artifact", status: "succeeded", content: { type: "artifact", artifact: block.artifact }, actions: [], fallback_text: block.artifact.path ?? block.artifact.name }} />;
          case "rule":
            return <hr class="semantic-rule" />;
          case "raw_markdown": {
            if (block.markdown.includes('"semantic_type"')) {
              const parts = assistantParts(block.markdown);
              if (parts.some((p) => p.type === "card")) {
                return (
                  <AssistantPartsView
                    parts={parts}
                    sessionId={props.sessionId}
                    resultId={props.resultId}
                    presentationId={props.presentationId}
                    textWrap={(text) => <div class="semantic-limited" title={block.reason}><pre>{text}</pre></div>}
                  />
                );
              }
            }
            return <div class="semantic-limited" title={block.reason}><pre>{block.markdown}</pre></div>;
          }
        }
      }}
    </For>
  );
}

export function PresentationDocumentView(props: { document: PresentationDocument } & PresentationDocumentContext) {
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
      <Show when={props.document.blocks.length === 0 && props.document.source_markdown}>
        <AssistantPartsView
          parts={assistantParts(props.document.source_markdown)}
          sessionId={props.sessionId}
          resultId={props.resultId}
          presentationId={props.presentationId}
          textWrap={(text) => <div class="semantic-source">{text}</div>}
        />
      </Show>
      <Show when={props.document.blocks.length === 0 && !props.document.source_markdown}>
        <section class="semantic-recovery" role="status">
          <Icon name="warning" size={15} />
          <div>
            <strong>No result</strong>
            <p>This task finished without producing a visible result. Check task details for what happened.</p>
          </div>
        </section>
      </Show>
      <Blocks blocks={props.document.blocks} recipeId={recipeId()} sessionId={props.sessionId} resultId={props.resultId} presentationId={props.presentationId} />
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
        <Show when={metadata.outcome_status}><div><dt>Run</dt><dd>{metadata.outcome_status}</dd></div></Show>
        <Show when={metadata.outcome_evidence_receipts}><div><dt>Evidence</dt><dd>{metadata.outcome_evidence_receipts}</dd></div></Show>
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

/** What the reader says about an Office file, and Download and Open with,
 * shared by the file row and the result card. */
function useFileActions(artifact: ArtifactRef, item?: OutputItem, sessionId?: string) {
  const path = () => artifact.path ?? null;
  const saved = artifact.status?.state !== "in_folder" ? artifact.status?.saved_as : null;
  const executionId = item?.provenance?.tool_call_id;
  const source = (value: string): api.OfficeSource => ({
    path: saved?.path ?? value,
    sessionId,
    candidateId: saved?.version_id,
    executionId: saved ? undefined : executionId ?? undefined,
  });
  const [facts] = createResource(
    () => { const value = path(); return value && isDocumentPath(value) ? value : null; },
    (value) => api.readOfficeFacts(source(value)),
  );
  const [problem, setProblem] = createSignal<string | null>(null);
  const download = async (value: string) => {
    setProblem(null);
    try {
      const { bytes, mime } = saved && sessionId
        ? await api.readSandboxCandidateFileBytes(sessionId, saved.version_id, saved.path)
        : executionId && sessionId
          ? await api.readExecutionArtifactBytes(sessionId, executionId, value)
          : await api.readFileBytes(value);
      await host.saveFile(artifact.name || value.split("/").pop() || "file", bytes, mime);
    } catch (error) {
      setProblem(`Could not download: ${error instanceof Error ? error.message : String(error)}`);
    }
  };
  const openWith = async (value: string) => {
    setProblem(null);
    try {
      await host.openWith?.(value);
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    }
  };
  const canOpenWith = () => host.can("open-with") && Boolean(facts()) && !facts()!.macro_enabled;
  const flags = () => { const value = facts(); return value ? officeFlagsLabel(value) : null; };
  return { path, facts, flags, problem, download, openWith, canOpenWith };
}

/** Opens a file where it can be seen: a saved draft version in Canvas as
 * that version, a previewable file in Canvas, anything else in Workbench. */
function openFile(item: OutputItem, sessionId?: string) {
  if (item.content.type !== "artifact") return;
  const artifact = item.content.artifact;
  const status = artifact.status;
  const saved = status && status.state !== "in_folder" ? status.saved_as : null;
  const context = {
    sessionId: sessionId ?? item.provenance?.session_id ?? undefined,
    resultId: item.outcome?.result_id ?? undefined,
    // A file already in the folder is read from the folder: the call that wrote it left nothing in scratch.
    executionId: status?.state === "in_folder" ? undefined : item.provenance?.tool_call_id ?? undefined,
  };
  if (saved && context.sessionId) {
    openArtifactCanvas(fileSubject(saved.path, { ...runOrigin(context), sessionId: context.sessionId, candidateId: saved.version_id }, artifact.name));
  } else if (artifact.path && isPreviewableArtifact(artifact.path)) {
    openArtifactFile(artifact.path, runOrigin(context));
  } else if (artifact.path) {
    openWorkbenchArtifact(artifact.path);
  }
}

/**
 * A file named in a message: an attached file or a file a document points
 * to (docs/design/72, U6), with what the reader says about an Office file
 * and Open, Download and, where the host has it, Open with. A file the Agent
 * produced as a result is a `ResultCard` instead.
 */
export function Artifact(props: { item: OutputItem }) {
  if (props.item.content.type !== "artifact") return null;
  const artifact = props.item.content.artifact;
  const file = useFileActions(artifact, props.item, props.item.provenance?.session_id ?? undefined);
  return (
    <article class="artifact-item">
      <span class="artifact-icon"><Icon name={artifact.media_type?.startsWith("image/") ? "preview" : "file"} size={15} /></span>
      <span class="artifact-copy">
        <strong>{artifact.name}</strong>
        <small>{file.facts() ? officeFactsLine(file.facts()!) : artifact.description ?? fileKind(artifact.name, artifact.media_type)}</small>
        <Show when={file.flags()}>{(label) => <small class="artifact-flags" title={file.facts()!.flags.join("\n")}><Icon name="warning" size={11} />{label()}</small>}</Show>
        <Show when={file.problem()}><small class="artifact-problem" role="alert">{file.problem()}</small></Show>
      </span>
      <Show when={file.path()}>
        {(value) => <button type="button" class="artifact-open" onClick={() => openFile(props.item)}>{isPreviewableArtifact(value()) ? "Open Canvas" : "Open"}</button>}
      </Show>
      <Show when={file.path()}>
        {(value) => <button type="button" class="artifact-open" onClick={() => void file.download(value())}>Download</button>}
      </Show>
      <Show when={file.canOpenWith() && file.path()}>
        {(value) => <button type="button" class="artifact-open" onClick={() => void file.openWith(value())}>Open with…</button>}
      </Show>
    </article>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * The picture at the start of a result card: the page itself for a web page,
 * offline in a sandboxed frame as Review draws it, the image for an image,
 * and the kind of file for anything else. It reads the newest saved version
 * when there is one, so the card shows what Review would.
 */
function ResultPreview(props: { item: OutputItem; sessionId: string }) {
  const artifact = () => (props.item.content.type === "artifact" ? props.item.content.artifact : null);
  const source = createMemo(() => {
    const value = artifact();
    if (!value?.path) return null;
    const status = value.status;
    const saved = status && status.state !== "in_folder" ? status.saved_as : null;
    const reader: ArtifactPreviewReader = saved
      ? { readFile: (file) => api.readSandboxCandidateFile(props.sessionId, saved.version_id, file), readFileRaw: (file) => api.readSandboxCandidateFileRaw(props.sessionId, saved.version_id, file) }
      : props.item.provenance?.tool_call_id
        ? { readFile: (file) => api.readExecutionArtifact(props.sessionId, props.item.provenance!.tool_call_id!, file), readFileRaw: (file) => api.readExecutionArtifactRaw(props.sessionId, props.item.provenance!.tool_call_id!, file) }
      : api;
    const kind = fileKind(value.name, value.media_type);
    return { path: saved?.path ?? value.path, kind, reader, key: `${saved?.version_id ?? ""}:${saved?.path ?? value.path}` };
  }, undefined, { equals: (a, b) => a?.key === b?.key });
  // Signals, not a resource: a pending resource read here would suspend the
  // conversation's boundary and remount the turn.
  const [page, setPage] = createSignal<string | null>(null);
  const [image, setImage] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal(false);
  createEffect(() => {
    const value = source();
    setPage(null);
    setImage(null);
    setFailed(false);
    if (!value || (value.kind !== "Web page" && value.kind !== "Image")) return;
    let current = true;
    let objectUrl: string | null = null;
    onCleanup(() => {
      current = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    });
    if (value.kind === "Image") {
      value.reader.readFileRaw(value.path)
        .then((url) => { if (current) { objectUrl = url; setImage(url); } else URL.revokeObjectURL(url); })
        .catch(() => { if (current) setFailed(true); });
      return;
    }
    value.reader.readFile(value.path)
      .then((file) => {
        const content = file.content;
        if (content == null) throw new Error("not text");
        return artifactPreviewHtml(value.path, content, value.reader).catch(() => sandboxedSrcdoc(content));
      })
      .then((html) => { if (current) setPage(html); })
      .catch(() => { if (current) setFailed(true); });
  });
  const pending = () => !failed() && !page() && !image() && (source()?.kind === "Web page" || source()?.kind === "Image");
  return (
    <div class="result-card-preview" aria-hidden="true">
      <Show when={page()}>{(html) => <div class="result-card-page"><iframe title="" tabIndex={-1} sandbox="allow-scripts" srcdoc={html()} /></div>}</Show>
      <Show when={image()}>{(url) => <img class="result-card-image" src={url()} alt="" />}</Show>
      <Show when={pending()}><Skeleton label="Loading preview" shapes={["block"]} class="result-card-skeleton" /></Show>
      <Show when={!page() && !image() && !pending()}>
        <div class="result-card-kind"><Icon name={source()?.kind === "Image" ? "preview" : "file"} size={22} /><span>{source()?.kind ?? "File"}</span></div>
      </Show>
    </div>
  );
}

/**
 * A file the Agent produced, as the one result card (docs/design/75 §6.1): a
 * preview, where the file stands in words, and its actions. Review changes is
 * the primary action only on the conversation's newest draft still waiting
 * for review; a card whose status the server could not establish says
 * nothing about it rather than guess. Size and path are technical details.
 */
export function ResultCard(props: { item: OutputItem; sessionId: string; resultId?: string; showReview?: boolean; askForChanges?: boolean }) {
  if (props.item.content.type !== "artifact") return null;
  const artifact = props.item.content.artifact;
  const file = useFileActions(artifact, props.item, props.sessionId);
  const words = () => statusWords(artifact.status);
  const review = () => (props.showReview === false ? undefined : props.item.actions.find((action) => action.verb === "review_draft"));
  const primary = () => {
    if (!review() || artifact.status?.state !== "draft") return false;
    const timeline = presentationOf(props.sessionId);
    return timeline ? newestWaitingDraft(timeline.items) === props.item.id : true;
  };
  const made = () => {
    const at = Date.parse(props.item.timestamp);
    return at > Date.UTC(2000, 0, 1) ? `made ${relAgo(props.item.timestamp)}` : "";
  };
  const facts = () => file.facts() ? officeFactsLine(file.facts()!) : fileKind(artifact.name, artifact.media_type);
  const kind = () => fileKind(artifact.name, artifact.media_type);
  const fileIcon = () => kind() === "Word document" ? "word" as const : kind() === "Spreadsheet" ? "sheet" as const : kind() === "Presentation" ? "slides" as const : kind() === "PDF" ? "pdf" as const : kind() === "Diagram" ? "diagram" as const : "file" as const;
  const hasPreview = () => kind() === "Image" || kind() === "Web page";
  const askForChanges = () => {
    const resultId = props.resultId;
    if (!resultId) return;
    setReplyTarget({ sessionId: props.sessionId, resultId, label: artifact.name });
    window.dispatchEvent(new CustomEvent("vak:focus-composer"));
  };
  return (
    <article class="result-card" classList={{ "has-preview": hasPreview() }} aria-label={artifact.name}>
      <Show when={hasPreview()} fallback={<span class={`result-card-file-icon ${fileIcon()}`} aria-hidden="true"><Icon name={fileIcon()} size={19} /></span>}>
        <div class="result-card-preview-wrap">
          <ResultPreview item={props.item} sessionId={props.sessionId} />
          <span class={`result-card-file-icon ${fileIcon()}`} aria-hidden="true"><Icon name={fileIcon()} size={18} /></span>
        </div>
      </Show>
      <div class="result-card-text">
        <Show when={words()}>{(value) => <p class="result-card-status" classList={{ waiting: value().waiting }}><Show when={value().waiting}><span class="result-card-dot" aria-hidden="true" /></Show>{value().headline}</p>}</Show>
        <h3 class="result-card-name">{artifact.name}</h3>
        <p class="result-card-details"><span>{facts()}</span><Show when={made()}>{(value) => <span class="result-card-made"> · {value()}</span>}</Show><Show when={!words()?.waiting && words()?.folder}><span> · {words()?.folder}</span></Show></p>
        <Show when={file.flags()}>{(label) => <p class="result-card-flags" title={file.facts()!.flags.join("\n")}><Icon name="warning" size={12} />{label()}</p>}</Show>
        <Show when={technicalDetails() && (artifact.size_bytes != null || artifact.path)}>
          <p class="result-card-technical">{[artifact.size_bytes != null ? formatBytes(artifact.size_bytes) : "", artifact.path ?? ""].filter(Boolean).join(" · ")}</p>
        </Show>
        <Show when={file.problem()}><p class="result-card-problem" role="alert">{file.problem()}</p></Show>
      </div>
      <div class="result-card-actions">
        <Show when={review()}>
          {(action) => <button type="button" class={primary() ? "btn primary" : "btn"} onClick={() => openCandidateReview(action().data.execution_id, props.sessionId, action().data.candidate_id)}>Review changes</button>}
        </Show>
        <Show when={file.path()}><button type="button" class="btn" onClick={() => openFile(props.item, props.sessionId)}>Open</button></Show>
        <Show when={!words()?.waiting && file.path()}>{(value) => <button type="button" class="btn" onClick={() => void file.download(value())}>Download</button>}</Show>
        <Show when={file.canOpenWith() && file.path()}>{(value) => <button type="button" class="btn" onClick={() => void file.openWith(value())}>Open with…</button>}</Show>
        <Show when={props.askForChanges && props.resultId}><button type="button" class="result-card-link" onClick={askForChanges}>Ask for changes</button></Show>
      </div>
    </article>
  );
}

function compactFailure(text: string): { summary: string; details: string } {
  const details = text.trim();
  if (details === "max_turns") {
    return { summary: "Vakyartha reached this task’s step limit before finishing. Any saved work is shown above; you can ask it to continue.", details };
  }
  const cleaned = stripControlScaffolding(details)
    .replace(/\[working directory:[^\]]*\]\s*/gi, "")
    .replace(/\[file:[^\]]*\]\s*/gi, "")
    .replace(/\[(?:stdout|stderr)\]\s*/gi, "")
    .replace(/\s+/g, " ")
    .trim();
  const first = (cleaned.split(/(?:\s+at\s+|\s+\[exit code|\s+exit code:)/i)[0] ?? cleaned)
    .replace(/^Error:\s*/i, "")
    .trim();
  return { summary: first ? `${first.slice(0, 220)}${first.length > 220 ? "…" : ""}` : "The sandbox step failed.", details };
}

function SemanticApproval(props: { item: OutputItem; sessionId: string }) {
  if (props.item.content.type !== "approval") return null;
  const content = props.item.content;
  const pending = () => props.item.status === "pending";
  // An approval is a question, not a record: once answered it leaves the
  // conversation (the decision lives in the ledger and activity log).
  if (!pending()) return null;
  const argsPretty = () => { try { return JSON.stringify(JSON.parse(content.args_json), null, 2); } catch { return content.args_json; } };
  return (
    <section
      class="approval semantic-approval"
      role={pending() ? "alert" : "status"}
      aria-live={pending() ? "assertive" : "polite"}
      aria-atomic="true"
      aria-label={`${pending() ? "Approval requested" : "Approval resolved"} for ${content.tool}`}
    >
      <div class="ap-head">Vakyartha wants to use {content.tool}</div>
      <div class="ap-reason">This needs your approval before it can continue.</div>
      <details class="ap-details"><summary>View request details</summary><pre class="ap-args">{argsPretty()}</pre></details>
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

function ResultEvidence(props: { item: OutputItem }) {
  const outcome = () => props.item.outcome;
  return <Show when={outcome()}>{(value) => <Show when={value().status !== "succeeded"}>
    <footer class={`primary-result-evidence ${value().status}`} aria-label="Result evidence">
      <span><Icon name="warning" size={12} />{value().status === "partial" ? "Partial result" : "Needs attention"}</span>
    </footer>
  </Show>}</Show>;
}

/** Ask for changes on a result that is not a single file; a single file's
 * card carries it (`ResultCard`). */
function ResultActions(props: { answer: OutputItem; material: OutputItem[]; sessionId: string; resultId?: string }) {
  const files = () => props.material.filter((item) => item.content.type === "artifact").length;
  const isPlan = createMemo(() => [props.answer, ...props.material].some((item) => {
    if (item.content.type === "adaptive") return item.content.tree.root.primitive === "timeline";
    if (item.content.type !== "structured") return false;
    return /(?:^|[._-])(plan|timeline|checklist|options)(?:$|[._-])/.test(item.content.output.semantic_type.toLowerCase());
  }));
  const revise = () => {
    const resultId = props.resultId;
    if (!resultId) return;
    setReplyTarget({ sessionId: props.sessionId, resultId, label: "this result" });
    window.dispatchEvent(new CustomEvent("vak:focus-composer"));
  };
  return <Show when={files() !== 1 && props.resultId}>
    <nav class="primary-result-actions" aria-label="Result actions">
      <button type="button" onClick={revise}>{isPlan() ? "Adjust plan" : "Ask for changes"}</button>
    </nav>
  </Show>;
}

/** A result's files as result cards, after its answer. Files from one run
 * share one draft, so only the first offers Review changes. */
function ResultFiles(props: { files: OutputItem[]; answer?: OutputItem; sessionId: string }) {
  const reviewed = new Set<string>();
  return <Show when={props.files.length > 0}>
    <div class="primary-result-files">
      {props.files.map((item) => {
        const execution = item.actions.find((action) => action.verb === "review_draft")?.data.execution_id;
        const showReview = !execution || !reviewed.has(execution);
        if (execution) reviewed.add(execution);
        return <ResultCard item={item} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? props.answer?.outcome?.result_id ?? undefined} showReview={showReview} askForChanges={props.files.length === 1} />;
      })}
    </div>
  </Show>;
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

function PresentationFeedback(props: { sessionId: string; semanticType: string; presentationId?: string }) {
  const [status, setStatus] = createSignal("");
  const [expanded, setExpanded] = createSignal(false);
  let input!: HTMLTextAreaElement;
  const send = async (choice: string) => {
    // The server requires `presentation_id` (docs/design/68-context-engine.md
    // §10) and 400s without one; a card with no known Presentation entry
    // (e.g. still streaming, or not sourced from an `emit_*_card` call) has
    // nothing to key feedback on yet.
    const presentationId = props.presentationId;
    if (!presentationId) {
      setStatus("Could not save");
      return;
    }
    setStatus("Saving…");
    try {
      if (choice === "Use this layout") await api.selectPresentationForSemantic(props.sessionId, props.semanticType, presentationId);
      await api.submitPresentationFeedback(props.sessionId, choice, presentationId, input?.value.trim() || undefined);
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

function optionInteractionFor(sessionId?: string, resultId?: string) {
  if (!sessionId || !resultId) return undefined;
  return { onOptionSelect: (label: string) => {
    setReplyTarget({ sessionId, resultId, label: `option “${label}”` });
    window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: `Use “${label}” in this plan and show me the whole updated plan.`, mode: "append" } }));
  } };
}

export function StructuredView(props: { output: import("../types").StructuredOutput; fallback?: string; sessionId?: string; resultId?: string; presentationId?: string }) {
  const [showOriginal, setShowOriginal] = createSignal(false);
  const fallback = () => props.fallback && !parseVakFence(props.fallback)
    ? <MarkdownView text={props.fallback} />
    : <details class="tool-details"><summary>View original result</summary><pre>{props.fallback || JSON.stringify(props.output.payload, null, 2)}</pre></details>;
  return <ErrorBoundary fallback={() => <section><p role="status">Rich presentation unavailable. Original content:</p>{fallback()}</section>}>
    <Show when={uiPreferences.richPreviews && props.output.schema_version === 2 && props.output.payload && typeof props.output.payload === "object"} fallback={fallback()}>
      <Show when={showOriginal() && showOperatorChrome()} fallback={
        <>
          <PresentationInteractionContext.Provider value={optionInteractionFor(props.sessionId, props.resultId)}>
            <div class="presentation-content"><StructuredRenderer output={props.output} /></div>
          </PresentationInteractionContext.Provider>
          <Show when={showOperatorChrome() && props.sessionId}>
            <PresentationFeedback sessionId={props.sessionId!} semanticType={props.output.semantic_type} presentationId={props.presentationId} />
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

// Every named semantic type resolves through this single registry. Tests derive
// their coverage from this registry so newly registered types cannot bypass the
// completed-turn rendering contract.
const STRUCTURED_RENDERERS: Record<string, StructuredRendererComponent> = {
  // Universal semantic shapes share a safe, lossless baseline renderer until
  // a richer domain-neutral interaction is available.
  "map": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Map")} />,
  "route_map": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Route map")} />,
  "calendar": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Calendar")} />,
  "availability": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Availability")} />,
  "board": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Board")} />,
  "entity": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Entity")} />,
  "search_results": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Search results")} />,
  "evidence": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Evidence")} />,
  "decision_analysis": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Decision")} />,
  "document": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Document")} />,
  "graph": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Graph")} />,
  "form": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Form")} />,
  "action": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Action")} />,
  "transaction": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Transaction")} />,
  "alert": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Alert")} />,
  "conversation": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Conversation")} />,
  "progress_dashboard": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Progress dashboard")} />,
  "simulation": ({ data }) => <GenericSpecRenderer node={buildUniversalCardSpec(data, "Simulation")} />,
  // 1. Research & Synthesis (research.synthesis, research_brief, research, news)
  "research.synthesis": ({ data }) => <GenericSpecRenderer node={buildResearchSpec(data)} />,
  "research_brief": ({ data }) => <GenericSpecRenderer node={buildResearchSpec(data)} />,
  "news": ({ data }) => <GenericSpecRenderer node={buildResearchSpec(data)} />,

  // 2. Code, Tests & Terminal (coding.diff, test.report, terminal.view, etc.)
  "coding.diff": ({ data }) => <GenericSpecRenderer node={buildDiffSpec(data)} />,
  "test.report": ({ data }) => <GenericSpecRenderer node={buildTestMatrixSpec(data)} />,
  "terminal.view": ({ data }) => <GenericSpecRenderer node={buildTerminalSpec(data)} />,

  // 3. Coding Specific Flows
  "coding.benchmark": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Benchmark Results")} />,
  "coding.dependencies": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Dependencies")} />,
  "coding.deployment": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Deployment", "Deployment")} />,
  "coding.incident": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Incident Summary", "Incident")} />,
  "coding.architecture": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Architecture Decisions", "Architecture")} />,
  "coding.release": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Release Notes", "Release")} />,
  "coding.search": ({ data }) => (Array.isArray(data?.results) || Array.isArray(data?.rows)) ? <GenericSpecRenderer node={buildTableSpec(data, "Search Results")} /> : <GenericSpecRenderer node={buildResearchSpec(data)} />,

  // 4. Data Grids, Tables & Comparisons
  "data.grid": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Data Grid")} />,
  "table": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Table")} />,
  "dataframe": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Table")} />,
  "comparison": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Comparison")} />,
  "comparison_table": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Comparison Table")} />,
  "pros_cons": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Pros & Cons")} />,
  "inventory": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Inventory")} />,
  "scorecard": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Scorecard")} />,
  "decision_matrix": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Decision Matrix")} />,
  "criteria_matrix": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Criteria Matrix")} />,
  "tradeoff_analysis": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Tradeoff Analysis")} />,

  // 5. Financial Summaries & Budgets
  "budget": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Budget Breakdown")} />,
  "finance_summary": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Financial Summary")} />,
  "invoice_summary": ({ data }) => <GenericSpecRenderer node={buildTableSpec(data, "Invoice Summary")} />,

  // 6. Culinary & Lifestyle
  "recipe.card": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
  "recipe": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
  "lifestyle.recipe": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
  "lifestyle.culinary_recipe": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
  "recipe_summary": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
  "meal_plan": ({ data }) => Array.isArray(data?.steps) || Array.isArray(data?.ingredients) ? <GenericSpecRenderer node={buildRecipeSpec(data)} /> : <GenericSpecRenderer node={buildTimelineSpec(data, "Meal Plan", "Meal Plan")} />,

  // 7. Interactive Previews
  "ui.preview": ({ data }) => <GenericSpecRenderer node={buildUiPreviewSpec(data)} />,

  // 8. Timelines, Plans, Checklists, Schedules & Notes
  "plan.timeline": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Plan Timeline", "Plan")} />,
  // Every timeline-shaped key below routes through the generic declarative
  // renderer (see presentation/GenericSpecRenderer.tsx): `buildTimelineSpec`
  // builds a node client-side from the same raw payload shapes the legacy
  // `normalizeTimeline` handled, and the `timeline` primitive produces the
  // exact same "adaptive-timeline" DOM/CSS TimelineCard did (including the
  // per-key `kicker`). No registry key calls TimelineCard directly anymore.
  "timeline": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Timeline", "Timeline")} />,
  "itinerary": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Itinerary", "Itinerary")} />,
  "checklist": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Checklist", "Checklist")} />,
  "schedule": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Schedule", "Schedule")} />,
  "agenda": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Agenda", "Agenda")} />,
  "milestones": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Milestones", "Milestones")} />,
  "progress": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Progress Tracking", "Progress")} />,
  "status": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Status", "Status")} />,
  "steps": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Steps", "Steps")} />,
  "overview": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Overview", "Overview")} />,
  "summary": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Summary", "Summary")} />,
  "detail": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Details", "Detail")} />,
  "notes": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Notes", "Notes")} />,
  "follow_up": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Follow-Up Items", "Follow-Up")} />,
  "reminder": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Reminders", "Reminder")} />,
  "shopping_list": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Shopping List", "Shopping List")} />,
  "lesson": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Lesson Plan", "Lesson")} />,
  "reading_list": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Reading List", "Reading List")} />,
  "habit_plan": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Habit Plan", "Habit")} />,
  "project_plan": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Project Plan", "Project")} />,
  "meeting_notes": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Meeting Notes", "Meeting")} />,
  "contact_log": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Contact Log", "Contact")} />,
  "travel_options": ({ data }) => {
    const tableShape = !Array.isArray(data?.options) && (
      Array.isArray(data?.rows) || Array.isArray(data?.columns) ||
      Array.isArray(data?.pros) || Array.isArray(data?.cons) ||
      (data?.left && data?.right)
    );
    return <GenericSpecRenderer node={tableShape ? buildOptionsTableSpec(data, "Travel Options") : buildTimelineSpec(data, "Travel Options", "Travel")} />;
  },
  "home_project": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Home Project", "Project")} />,
  "care_plan": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Care Plan", "Care Plan")} />,
  "event_plan": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Event Plan", "Event")} />,
  "media_list": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Media List", "Media")} />,
  "collection": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Collection", "Collection")} />,
  "faq": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Frequently Asked Questions", "FAQ")} />,
  "decision": ({ data }) => <GenericSpecRenderer node={buildTimelineSpec(data, "Decision Analysis", "Decision")} />,

  // 9. Visualizations & Charts
  // Malformed/missing `series` no longer renders blank: buildChartSpec
  // normalizes it to an empty series list and the chart primitive shows its
  // "No data points supplied." placeholder instead.
  "chart": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,
  "telemetry.chart": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,
  "trend": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,
  "timeseries": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,
  "metric_chart": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,
  "bar_chart": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data, "bar")} />,
  "comparison_chart": ({ data }) => <GenericSpecRenderer node={buildChartSpec(data)} />,

  // 10. Media & Links
  "link.preview": ({ data }) => <GenericSpecRenderer node={buildMediaSpec(data, "link")} />,
  "media.image": ({ data }) => <GenericSpecRenderer node={buildMediaSpec(data, "image")} />,
  "media.video": ({ data }) => <GenericSpecRenderer node={buildMediaSpec(data, "video")} />,
  "media.audio": ({ data }) => <GenericSpecRenderer node={buildMediaSpec(data, "audio")} />,

  // 11. Metrics & Weather
  "weather": (props) => STRUCTURED_RENDERERS.metric(props),
  "telemetry.metric": (props) => STRUCTURED_RENDERERS.metric(props),
  // Validated alongside "timeline" as the generic declarative renderer's
  // second proving case (see presentation/GenericSpecRenderer.tsx). The
  // "canvas-card" wrapper class (added independently on main) is preserved
  // in GenericSpecRenderer's renderMetric so this migration didn't regress
  // that chrome fix.
  "metric": ({ data }) => <GenericSpecRenderer node={buildMetricSpec(data)} />,
};

export const structuredRendererTypes = Object.freeze(Object.keys(STRUCTURED_RENDERERS));

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
      <details class="tool-details"><summary>View original result</summary><pre>{JSON.stringify(payload, null, 2)}</pre></details>
    </div>
  );
}


function Turn(props: { id: string; items: OutputItem[]; sessionId: string; allowContinuation: boolean }) {
  const continuationPrompt = () => {
    // The original request and completed tool receipts are already in the
    // append-only ledger. Repeating its imperative text here causes the intent
    // resolver to demand a second file modification in this new turn, even
    // when the saved file is the work being continued.
    return "Continue the most recent unfinished task in this conversation. Inspect saved work, perform only outstanding steps, and give a concise answer. Read the actual saved file or data before calculating; after a successful read, use that result instead of rereading the same file or relying on a displayed card.";
  };
  const uniqueResultId = () => {
    const ids = new Set(props.items.map((entry) => entry.outcome?.result_id).filter((id): id is string => Boolean(id)));
    return ids.size === 1 ? [...ids][0] : undefined;
  };
  const ordered = () => {
    const seen = new Set<string>();
    const nonProgress = props.items
      .filter((item) => !["progress", "retry", "information"].includes(item.kind))
      // A denied approval can produce both a permission error and a later
      // abort marker. Present one calm outcome in chat; full receipts remain
      // available in task details.
      .filter((item) => {
        if (item.kind !== "error") return true;
        const raw = item.fallback_text.toLowerCase();
        if (!raw.includes("denied") && !raw.includes("aborted")) return true;
        if (seen.has("permission-stop")) return false;
        seen.add("permission-stop");
        return true;
      });

    const isRealAnswer = (content: import("../types").OutputContent): boolean => {
      if (["document", "structured", "adaptive"].includes(content.type)) return true;
      if (content.type === "outcome") return Boolean(content.document);
      return false;
    };

    // Check if the turn has any real assistant answer
    const hasAssistantAnswer = nonProgress.some(
      (item) => item.role === "assistant" && isRealAnswer(item.content),
    );

    const filtered = nonProgress.filter((item) => {
      // Suppress intermediate tool errors if the turn produced an assistant answer,
      // or if operator chrome is off (tool errors belong in Workbench/Details).
      if (item.role === "tool" && item.kind === "error") {
        if (hasAssistantAnswer || !showOperatorChrome()) return false;
      }
      // Bare lifecycle outcomes without a document are internal control summaries
      if (item.content.type === "outcome" && !item.content.document && !showOperatorChrome()) {
        return false;
      }
      return true;
    });

    const isFleetingAssistantItem = (item: OutputItem): boolean => {
      if (item.role !== "assistant") return false;
      if (item.content.type === "document") {
        return isFleetingNarration(item.content.document.source_markdown);
      }
      if (item.content.type === "outcome") {
        return !item.content.document || isFleetingNarration(item.content.document.source_markdown);
      }
      return false;
    };

    return filtered.filter((item, idx) => {
      // In outcome density, suppress earlier assistant items in the same turn
      // ONLY if they are fleeting narration and a subsequent real assistant answer exists.
      // Substantive documents, answers, and structured cards must never be hidden.
      if (isFleetingAssistantItem(item)) {
        for (let j = idx + 1; j < filtered.length; j++) {
          const next = filtered[j];
          if (next.role === "assistant" && isRealAnswer(next.content) && !isFleetingAssistantItem(next)) {
            return false;
          }
        }
      }
      return true;
    });
  };
  const OrderedItem = (item: OutputItem): JSX.Element | null => {
    if (item.role === "user" && item.content.type === "document") {
      return <UserMessage document={item.content.document} text={item.fallback_text} />;
    }
    if (item.kind === "approval") return <SemanticApproval item={item} sessionId={props.sessionId} />;
    if (item.content.type === "document") return showOperatorChrome()
      ? <article class="semantic-outcome"><ResultOutcomeSummary item={item} /><Show when={item.role === "assistant"} fallback={<PresentationDocumentView document={item.content.document} />}><AnswerCard item={item} document={item.content.document} /></Show></article>
      : item.role === "user" ? <UserMessage document={item.content.document} text={item.fallback_text} /> : <AssistantMessage sessionId={props.sessionId} text={item.content.document.source_markdown}><PresentationDocumentView document={item.content.document} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} /></AssistantMessage>;
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
      return <AssistantMessage sessionId={props.sessionId} text={item.content.document.source_markdown}><PresentationDocumentView document={item.content.document} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} /></AssistantMessage>;
    }
    if (item.content.type === "structured") return <StructuredView output={item.content.output} fallback={item.fallback_text} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? uniqueResultId() ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} />;
    if (item.content.type === "adaptive") return <AdaptiveTreeView tree={item.content.tree} fallback={item.content.fallback_text} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? uniqueResultId() ?? item.id} />;
    if (item.kind === "error") {
      if (item.content.type === "error" && item.content.message === "max_turns") {
        return <section class="semantic-recovery" role="status">
          <Icon name="warning" size={15} />
          <div>
            <strong>Continue this task?</strong>
            <p>Vakyartha reached this run’s step limit. Saved work is shown above. Continuing starts another bounded turn in this conversation.</p>
            <Show when={props.allowContinuation}><button type="button" class="btn" disabled={isRunning(props.sessionId)} onClick={() => void sendPrompt(continuationPrompt(), undefined, undefined, props.sessionId, undefined, "follow_up")}>Continue</button></Show>
          </div>
        </section>;
      }
      const isRawJson = item.fallback_text.trim().startsWith("{") || item.fallback_text.includes('"type":');
      const failureText = item.content.type === "error" ? item.content.message : item.fallback_text;
      const text = !showOperatorChrome() && isRawJson ? "The operation could not be completed." : compactFailure(failureText).summary;
      const details = isRawJson ? item.fallback_text : compactFailure(item.fallback_text).details;
      return <section class="semantic-recovery" role="alert"><Icon name="warning" size={15} /><div><strong>{item.status === "partial" ? "Partial outcome" : "Run needs attention"}</strong><p>{text}</p><details class="semantic-recovery-details"><summary>View details</summary><pre>{details}</pre></details></div></section>;
    }
    if (item.kind === "artifact") return <ResultCard item={item} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? undefined} askForChanges />;
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
  const visible = createMemo(() => ordered().map((item) => ({ item, node: OrderedItem(item) })).filter((entry) => entry.node !== null));
  // Cards, artifacts and the narration that follows them are one reply from
  // one author: they share a single assistant header, cards first (the order
  // the ledger recorded them). Without this a card rendered as an orphan row
  // above the header of the sentence that introduced it.
  const blocks = createMemo(() => {
    const out: JSX.Element[] = [];
    let lead: Array<{ item: OutputItem; node: JSX.Element }> = [];
    const flush = () => {
      if (lead.length === 0) return;
      const material = lead;
      // A capped turn can produce a real file or card before the result
      // evaluator assigns a result ID. Keep its observed file actions usable.
      const anchor = material.find((entry) => entry.item.outcome?.result_id)?.item ?? material[0]?.item;
      const cards = material.filter((entry) => entry.item.kind !== "artifact");
      out.push(<AssistantMessage sessionId={props.sessionId}>
        <article class="primary-result" data-result-id={anchor?.outcome?.result_id} aria-label="Agent result">
          <Show when={anchor && anchor.status !== "succeeded"}><div class="primary-result-caution" role="status"><Icon name="warning" size={14} />The requested outcome is not verified. Check the evidence before relying on completion claims.</div></Show>
          <Show when={cards.length > 0}><div class="primary-result-material">{cards.map((entry) => entry.node)}</div></Show>
          <ResultFiles files={material.filter((entry) => entry.item.kind === "artifact").map((entry) => entry.item)} answer={anchor} sessionId={props.sessionId} />
          <Show when={anchor}>{(item) => <><ResultEvidence item={item()} /><ResultActions answer={item()} material={material.map((entry) => entry.item)} sessionId={props.sessionId} resultId={item().outcome?.result_id ?? uniqueResultId() ?? item().id} /></>}</Show>
        </article>
      </AssistantMessage>);
      lead = [];
    };
    const grouping = !showOperatorChrome();
    const isPrimaryCard = (entry: { item: OutputItem }) => {
      const source = entry.item.provenance?.source ?? "";
      return ((entry.item.content.type === "structured" || entry.item.content.type === "adaptive") && source.startsWith("emit_") && source.endsWith("_card")) ||
        (entry.item.content.type === "adaptive" && source === "adaptive_library");
    };
    const cardCopyText = (entry: { item: OutputItem }) => {
      const readable = entry.item.fallback_text.replace(/```json[\s\S]*?```/g, "").trim();
      if (readable.split("\n").filter(Boolean).length > 1 || entry.item.content.type !== "structured") return readable;
      const payload = entry.item.content.output.payload;
      const values = Object.entries(payload).filter(([key]) => key !== "title").map(([key, value]) =>
        `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`);
      return [readable, ...values].filter(Boolean).join("\n");
    };
    const sameResult = (left: OutputItem, right: OutputItem) => {
      const a = left.outcome?.result_id;
      const b = right.outcome?.result_id;
      return !a || !b || a === b;
    };
    const entries = visible();
    // The Agent may emit a card, revise its draft answer, and then write the
    // final answer. Keep the latest card of each semantic type with it.
    let finalAnswerIndex = -1;
    for (let i = 0; i < entries.length; i += 1) {
      const item = entries[i].item;
      if (item.role === "assistant" && (
        item.content.type === "document" ||
        (item.content.type === "outcome" && Boolean(item.content.document))
      )) finalAnswerIndex = i;
    }
    const finalCards = new Map<string, { item: OutputItem; node: JSX.Element }>();
    for (let i = 0; i < finalAnswerIndex; i += 1) {
      const entry = entries[i];
      const content = entry.item.content;
      if (content.type !== "structured" && content.type !== "adaptive") continue;
      const key = content.type === "structured" ? content.output.semantic_type : `adaptive:${content.tree.spec_id}`;
      finalCards.set(key, entry);
    }
    const carriedCards = [...finalCards.values()];
    for (let index = 0; index < entries.length; index += 1) {
      const { item, node } = entries[index];
      if (index < finalAnswerIndex && (item.content.type === "structured" || item.content.type === "adaptive")) continue;
      if (grouping && index < finalAnswerIndex && item.role === "assistant" && (
        item.content.type === "document" || (item.content.type === "outcome" && Boolean(item.content.document))
      )) continue;
      const cardLike =
        item.role !== "user" &&
        (item.content.type === "structured" || item.content.type === "adaptive" || item.kind === "artifact");
      if (grouping && cardLike) {
        if (lead.length > 0 && !sameResult(lead[0].item, item)) flush();
        lead.push({ item, node });
        continue;
      }
      const answer =
        item.role === "assistant" && item.content.type === "document"
          ? item.content.document
          : item.role === "assistant" && item.content.type === "outcome"
            ? item.content.document
            : undefined;
      if (grouping && answer) {
        if (lead.length > 0 && !sameResult(lead[0].item, item)) flush();
        const material = index === finalAnswerIndex ? [...carriedCards, ...lead] : [...lead];
        // Structured material can arrive on either side of the answer in
        // the ledger. Keep adjacent cards with their result in both cases.
        while (index + 1 < entries.length) {
          const next = entries[index + 1];
          if (next.item.role === "user" || !(
            next.item.content.type === "structured" ||
            next.item.content.type === "adaptive" ||
            next.item.kind === "artifact"
          ) || !sameResult(item, next.item)) break;
          material.push(next);
          index += 1;
        }
        const hasCard = material.some(isPrimaryCard);
        const displayText = hasCard
          ? [
              ...material.filter(isPrimaryCard).map(cardCopyText),
              answer.metadata.card_note,
            ].filter(Boolean).join("\n\n")
          : answer.source_markdown;
        const result = <AssistantMessage sessionId={props.sessionId} text={displayText}>
            <article
              class="primary-result"
              data-result-id={item.outcome?.result_id ?? item.id}
              aria-label="Agent result"
            >
              <Show when={item.status !== "succeeded"}><div class="primary-result-caution" role="status"><Icon name="warning" size={14} />The requested outcome is not verified. Check the evidence before relying on completion claims.</div></Show>
              <Show when={material.some((entry) => entry.item.kind !== "artifact")}>
                <div class="primary-result-material" aria-label="Result material">
                  {material.filter((entry) => entry.item.kind !== "artifact").map((entry) => entry.node)}
                </div>
              </Show>
              <Show when={hasCard}
                fallback={<div class="primary-result-answer"><PresentationDocumentView document={answer} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} /></div>}>
                <Show when={answer.metadata.card_note}>{(note) => <div class="primary-result-note"><MarkdownView text={note()} /></div>}</Show>
              </Show>
              <ResultFiles files={material.filter((entry) => entry.item.kind === "artifact").map((entry) => entry.item)} answer={item} sessionId={props.sessionId} />
              <ResultEvidence item={item} />
              <ResultActions answer={item} material={material.map((entry) => entry.item)} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? uniqueResultId()} />
            </article>
          </AssistantMessage>;
        out.push(result);
        lead = [];
        continue;
      }
      flush();
      out.push(node);
    }
    flush();
    return out;
  });
  return (
    <section class="semantic-turn" data-turn={props.id}>
      <Show
        when={visible().length > 0}
        fallback={
          <Show when={props.items.length > 0}>
            <section class="semantic-recovery" role="status">
              <Icon name="warning" size={15} />
              <div>
                <strong>No result</strong>
                <p>This task finished without producing a visible result. Check task details for what happened.</p>
              </div>
            </section>
          </Show>
        }
      >
        <For each={blocks()}>{(node) => node}</For>
      </Show>
    </section>
  );
}

export function AdaptiveTreeView(props: { tree: import("../types").AdaptiveRenderTree; fallback: string; sessionId?: string; resultId?: string }) {
  const root = () => {
    const node = props.tree.root;
    // Earlier saved travel trees predate the declarative options variant.
    if (props.tree.spec_id === "seed.travel-options" && props.tree.revision < 7 && node.primitive === "table") {
      return { ...node, props: { ...node.props, variant: "options" } };
    }
    return props.tree.spec_id.startsWith("seed.") && node.primitive === "entity"
      ? { ...node, props: { ...node.props, kind: props.tree.spec_id.slice(5).replaceAll("-", " ") } }
      : node;
  };
  return (
    <section class="adaptive-presentation" aria-label={props.tree.accessibility_summary ?? "Adaptive presentation"}>
      <PresentationInteractionContext.Provider value={optionInteractionFor(props.sessionId, props.resultId)}>
        <GenericSpecRenderer node={root()} />
      </PresentationInteractionContext.Provider>
    </section>
  );
}

export default function PresentationTimelineView(props: { timeline: OutputTimeline; sessionId: string; allowContinuation?: boolean; hideUser?: boolean }) {
  const sameItems = (previous: OutputItem[], next: OutputItem[]) =>
    previous.length === next.length && previous.every((item, index) => item === next[index]);
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
    return order;
  });
  return <div class="semantic-timeline">
    <Show when={showOperatorChrome() && props.timeline.goal}>{(goal) => <details class="goal-state" open={goal().control !== "active"}>
      <summary><span class="goal-state-label">{goal().control === "active" ? "Follow-up goal" : "Goal record"}</span><span class={`goal-state-control ${goal().control}`}>{goal().control === "active" ? "available" : goal().control}</span><span class="goal-state-revision">rev {goal().revision}</span></summary>
      <p class="goal-state-objective">{goal().objective}</p>
      <Show when={goal().additions.length}><ul><For each={goal().additions}>{(addition) => <li>{addition}</li>}</For></ul></Show>
      <Show when={goal().superseded_revisions.length}><small>Superseded revisions: {goal().superseded_revisions.join(", ")}</small></Show>
    </details>}</Show>
    <For each={turns()}>{(id) => {
      // Live frames replace the timeline even when this settled turn has not
      // changed. Reuse its item list so Turn does not rebuild every result.
      const items = createMemo(() => props.timeline.items.filter((item) =>
        item.turn_id === id && !(props.hideUser && item.role === "user")), undefined, { equals: sameItems });
      return <Turn id={id} items={items()} sessionId={props.sessionId} allowContinuation={props.allowContinuation ?? false} />;
    }}</For>
  </div>;
}
