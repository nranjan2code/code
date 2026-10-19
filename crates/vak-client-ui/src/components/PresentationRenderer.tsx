import { validateAdaptiveTree } from "../adaptiveValidation";
import AnswerValuesScope, { conversationValuesKey } from "./AnswerValuesScope";
import { answerDocument, answerSegments, consolidateFilePreviews, groupAnswerEntries } from "../answerComposition";
import { createContext, createEffect, createMemo, createResource, createSignal, ErrorBoundary, For, onCleanup, Show, useContext } from "solid-js";
import type { JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type {
  ArtifactRef,
  DocumentBlock,
  InlineNode,
  OutputContent,
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
  openMailCalendarCitation,
  setReplyTarget,
  isRunning,
  presentationOf,
  technicalDetails,
  health,
  sessions,
  openInLibrary,
  openAbilitySheet,
  appendUser,
  setNotice,
} from "../store";
import MarkdownView from "./MarkdownView";
import MessageActions from "./MessageActions";
import { approve, sendPrompt } from "../App";
import Icon from "./Icon";
import { Pill, stateOf } from "./fileState";
import { safeUrl, isLocalArtifactPath, sandboxedSrcdoc } from "../safeUrl";
import { parseMailCalendarCitation } from "../mailCalendarCitation";
import { fileSubject, inlineTitle, runOrigin } from "../canvasSubject";
import { artifactPreviewHtml, type ArtifactPreviewReader } from "../artifactPreview";
import { fileKind, newestWaitingDraft, statusWords } from "../resultCard";
import { relAgo } from "../time";
import Skeleton from "./Skeleton";
import AutomationCard from "./AutomationCard";
import { approvalNetworkNote, approvalSubjects, approvalTitle, mcpServerName } from "../approvalSubject";
import { mcpServerNetwork } from "../mcpNetwork";
import * as api from "../api";
import { host } from "../host";
import { isDocumentPath, parseOfficeCitation } from "../officeFiles";
import { officeFactsLine, officeFlagsLabel } from "../officeFacts";
import { highlight, languageForFence } from "../highlight";
import DiffInspector from "./presentation/DiffInspector";
import MermaidViewer from "./presentation/MermaidViewer";

// Technical details are a disclosure preference only; they never change the
// result, route, authority, or which safety states remain visible.
const showOperatorChrome = () => technicalDetails();
import { CardIdContext, FormView, PackTree, PresentationInteractionContext, PresentationPageResourceContext, PresentationSessionContext, downloadCsv, formNames } from "./cards";
import AgentMark from "./AgentMark";
import { assistantParts, groupAssistantParts, parseVakFence, stripControlScaffolding } from "../structured";
import type { AssistantPart } from "../structured";
import { themedPreview } from "../previewTheme";

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

/** What an Agent's learned ability did in this turn, as one quiet line
 * under the answer (docs/design/93 §9.1); it opens the ability's sheet. */
function AbilityLine(props: { content: Extract<OutputContent, { type: "ability" }> }) {
  const words = () => {
    const name = <strong>{props.content.skill}</strong>;
    switch (props.content.event) {
      case "used": return <>Used {name}</>;
      case "failed": return <>{name} failed</>;
      case "learned": return <>Learned {name}</>;
      case "improved": return <>Improved {name}<Show when={props.content.version}>{(version) => <> (version {version()})</>}</Show></>;
      case "waiting": return <>{name} is waiting for your review</>;
      case "offered": return <>{name} is ready for every Agent</>;
      case "turned_off": return <>{name} was turned off</>;
    }
  };
  return (
    <button type="button" class="ability-line" onClick={() => openAbilitySheet(props.content.skill)}>
      <Icon name="spark" size={13} /><span>{words()}</span>
    </button>
  );
}

/** The head of a turn an automation delivered (docs/design/94 §4): its
 * name and when it ran, in place of the person's bubble, because the
 * person did not write it now. */
export function ScheduledHead(props: { name: string; failed: boolean; timestamp?: string; title?: string }) {
  const at = () => {
    const time = Date.parse(props.timestamp ?? "");
    return Number.isNaN(time) ? "" : new Date(time).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });
  };
  return <div class="scheduled-turn-head" title={props.title}>
    <span class="turn-author-chip">Scheduled</span>
    <span>{props.name}</span>
    <Show when={at()}><span>· {at()}</span></Show>
    <Show when={props.failed}><span class="scheduled-turn-failed">· did not finish</span></Show>
  </div>;
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
            const mailCitation = parseMailCalendarCitation(node.code);
            if (mailCitation) return <button type="button" class="ic mail-calendar-cite" title="Open cited conversation" onClick={() => openMailCalendarCitation(mailCitation)}>{node.code}</button>;
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
  const pageResources = useContext(PresentationPageResourceContext);
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
    openArtifactCanvas({ kind: "inline", title: inlineTitle(props.content, props.filename), html: props.content, sessionId: pageResources ? undefined : activeId() ?? undefined });
  };

  return (
    <div class="semantic-code" classList={{ diff: !!props.diff }}>
      <div class="semantic-code-head">
        <span>{props.filename ?? props.language ?? (props.diff ? "diff" : "text")}</span>
        <div style={{ display: "flex", gap: "6px", "margin-left": "auto" }}>
          <Show when={canPreview()}>
            <button type="button" onClick={openPreview} title="Open in Canvas">Preview</button>
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
                  <span style={{ "font-size": "var(--fs-caption)", "margin-left": "4px", opacity: "0.6" }}>
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
  const pageResources = useContext(PresentationPageResourceContext);
  const path = () => artifact.path ?? null;
  const status = artifact.status;
  const inFolder = status?.state === "in_folder";
  const saved = status && status.state !== "in_folder" ? status.saved_as : null;
  // A file already in the folder is read from the folder: the call that wrote
  // it left nothing in scratch.
  const executionId = inFolder ? undefined : item?.provenance?.tool_call_id;
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
      const { bytes, mime } = pageResources
        ? await pageResources.readFileBytes(saved?.path ?? value)
        : saved && sessionId
        ? await api.draftReader(sessionId, saved.version_id).readFileBytes(saved.path)
        : executionId && sessionId
          ? await api.readExecutionArtifactBytes(sessionId, executionId, value)
          : await api.readFileBytes(value, sessionId ?? (() => { throw new Error("Open a conversation to save its files."); })());
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
  /** Opens the Library on the artifact a saved draft is a version of. */
  const inLibrary = !pageResources && saved && sessionId
    ? () => void api.versionOf(sessionId, saved.version_id, saved.path)
        .then((at) => openInLibrary(at.artifact))
        .catch((cause) => setProblem(cause instanceof Error ? cause.message : String(cause)))
    : null;
  return { path, facts, flags, problem, download, openWith, canOpenWith, inLibrary };
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
        {(value) => <button type="button" class="artifact-open" title={isPreviewableArtifact(value()) ? "Open in Canvas" : undefined} onClick={() => openFile(props.item)}>Open</button>}
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
  const pageResources = useContext(PresentationPageResourceContext);
  const artifact = () => (props.item.content.type === "artifact" ? props.item.content.artifact : null);
  const source = createMemo(() => {
    const value = artifact();
    if (!value?.path) return null;
    const status = value.status;
    const saved = status && status.state !== "in_folder" ? status.saved_as : null;
    const run = status?.state === "in_folder" ? undefined : props.item.provenance?.tool_call_id;
    const reader: ArtifactPreviewReader = pageResources
      ? pageResources
      : saved
      ? api.draftReader(props.sessionId, saved.version_id)
      : run
        ? { readFile: (file) => api.readExecutionArtifact(props.sessionId, run, file), readFileRaw: (file) => api.readExecutionArtifactRaw(props.sessionId, run, file) }
      : api.workspaceReader(props.sessionId);
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
      <Show when={page()}>{(html) => <div class="result-card-page"><iframe title="" tabIndex={-1} sandbox="allow-scripts" srcdoc={themedPreview(html())} /></div>}</Show>
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
  const pageResources = useContext(PresentationPageResourceContext);
  if (props.item.content.type !== "artifact") return null;
  const artifact = props.item.content.artifact;
  const file = useFileActions(artifact, props.item, props.sessionId);
  const words = () => statusWords(artifact.status);
  // What the version this card shows is today, from the Library's record
  // (doc 90 §5); the words above are only what the turn knew.
  const call = () => props.item.provenance?.tool_call_id ?? undefined;
  const [live] = createResource(
    () => (props.sessionId && call() ? [props.sessionId, call()!] as const : null),
    ([session, made]) => api.madeByCall(session, made).catch(() => null),
  );
  const liveState = () => stateOf(live()?.now, "call");
  const behind = () => live()?.now?.is === "waiting" && (live()!.now as { behind: boolean }).behind;
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
        <Show
          when={liveState()}
          fallback={<Show when={words()}>{(value) => <p class="result-card-status" classList={{ waiting: value().waiting }}><Show when={value().waiting}><span class="result-card-dot" aria-hidden="true" /></Show>{value().headline}</p>}</Show>}
        >
          {(state) => (
            <p class="result-card-states">
              <Pill mark={state().mark}>{state().word}</Pill>
              <Show when={behind()}><Pill mark="changed">Made before your edit</Pill></Show>
              <Show when={(live()?.newer ?? 0) > 0}><span class="result-card-since">since then: {live()!.newer} more version{live()!.newer === 1 ? "" : "s"}</span></Show>
            </p>
          )}
        </Show>
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
          {(action) => <button type="button" class={primary() ? "btn primary" : "btn"} aria-label="Review changes" onClick={() => openCandidateReview(action().data.execution_id, props.sessionId, action().data.candidate_id)}>Review</button>}
        </Show>
        <Show when={file.path() && !pageResources}><button type="button" class="btn" onClick={() => openFile(props.item, props.sessionId)}>Open</button></Show>
        <Show when={!words()?.waiting && file.path()}>{(value) => <button type="button" class="file-action-icon" aria-label="Download" title="Download" onClick={() => void file.download(value())}><Icon name="download" size={17} /></button>}</Show>
        <Show when={file.canOpenWith() && file.path()}>{(value) => <button type="button" class="file-action-icon" aria-label="Open with another app" title="Open with another app" onClick={() => void file.openWith(value())}><Icon name="monitor" size={17} /></button>}</Show>
        <Show
          when={live()}
          fallback={<Show when={file.inLibrary}>{(open) => <button type="button" class="file-action-icon" aria-label="Open in Library" title="Open in Library" onClick={open()}><Icon name="history" size={17} /></button>}</Show>}
        >
          {(made) => <button type="button" class="file-action-icon" aria-label="Open history" title="Open history" onClick={() => openInLibrary(made().artifact)}><Icon name="history" size={17} /></button>}
        </Show>
        <Show when={props.askForChanges && props.resultId}><button type="button" class="file-action-icon" aria-label="Ask for changes" title="Ask for changes" onClick={askForChanges}><Icon name="pencil" size={17} /></button></Show>
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
  const [mcpNetwork] = createResource(() => mcpServerName(content.args_json), (server) => mcpServerNetwork(server, agentForSession(props.sessionId).id));
  const argsPretty = () => { try { return JSON.stringify(JSON.parse(content.args_json), null, 2); } catch { return content.args_json; } };
  return (
    <section
      class="approval semantic-approval"
      role={pending() ? "alert" : "status"}
      aria-live={pending() ? "assertive" : "polite"}
      aria-atomic="true"
      aria-label={`${pending() ? "Approval requested" : "Approval resolved"} for ${content.tool}`}
    >
      <div class="ap-head">Vakyartha wants to use {approvalTitle(content.tool, content.args_json)}</div>
      <For each={approvalSubjects(content.args_json, sessions().find((session) => session.session_id === props.sessionId)?.cwd)}>{(subject) => <code class="ap-primary"><span class="ap-primary-key">{subject.key}</span>{subject.value}</code>}</For>
      <div class="ap-reason">This needs your approval before it can continue.</div>
      <Show when={approvalNetworkNote(content.tool, health()?.permission_mode, health()?.sandbox, mcpNetwork())}>{(note) => <div class="ap-reason ap-network-note">{note()}</div>}</Show>
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
      await api.recordOutcomeReview(props.sessionId, verdict, turn || undefined);
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
 * card carries it (`ResultCard`). A page has no conversation to ask in, so
 * it has none (docs/design/98 §12). */
function ResultActions(props: { answer: OutputItem; material: OutputItem[]; sessionId: string; turnId: string; resultId?: string }) {
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
  return <Show when={files() !== 1 && props.resultId && props.sessionId}>
    <nav class="primary-result-actions" aria-label="Result actions">
      <button type="button" onClick={revise}>{isPlan() ? "Adjust plan" : "Ask for changes"}</button>
      <Show when={files() === 0}>
        <Show when={!useContext(InCanvasContext)}>
          <button type="button" onClick={() => openArtifactCanvas({ kind: "answer", title: "Answer", sessionId: props.sessionId, turnId: props.turnId })}>Open in Canvas</button>
        </Show>
        <SaveAnswerToLibrary sessionId={props.sessionId} turnId={props.turnId} />
      </Show>
    </nav>
  </Show>;
}

/** A result's files as result cards, after its answer. Files from one run
 * share one draft, so only the first offers Review changes. */
function ResultFiles(props: { files: OutputItem[]; answer?: OutputItem; sessionId: string; resultId?: string }) {
  const reviewed = new Set<string>();
  return <Show when={props.files.length > 0}>
    <div class="primary-result-files">
      {props.files.map((item) => {
        const execution = item.actions.find((action) => action.verb === "review_draft")?.data.execution_id;
        const showReview = !execution || !reviewed.has(execution);
        if (execution) reviewed.add(execution);
        return <ResultCard item={item} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? props.resultId ?? props.answer?.outcome?.result_id ?? undefined} showReview={showReview} askForChanges={props.files.length === 1} />;
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

/** Keeps an answer in the Library as one document (docs/design/82-library.md §3).
 *  An answer is saved whole, never card by card; one that delivered a file
 *  has that file in the Library already and shows no Save. */
function SaveAnswerToLibrary(props: { sessionId: string; turnId: string }) {
  const [status, setStatus] = createSignal("");
  return <>
    <button
      type="button"
      disabled={status() === "Saving…" || status() === "Saved to Library"}
      onClick={async () => {
        setStatus("Saving…");
        try {
          await api.librarySaveAnswer(props.sessionId, props.turnId);
          setStatus("Saved to Library");
        } catch {
          setStatus("Could not save");
        }
      }}
    >
      {status() === "Saved to Library" ? "Saved to Library" : "Save to Library"}
    </button>
    <Show when={status() === "Could not save"}><small role="status">{status()}</small></Show>
  </>;
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

/** A card's actions, recorded on the card (docs/design/96, Q3): a choice
 *  starts a turn whose directive is the person's message; a tick changes
 *  the card's state. The composer is left alone. */
function cardInteractionFor(sessionId?: string, presentationId?: string, state?: import("../types").CardState | null) {
  if (!sessionId || !presentationId) return undefined;
  return {
    onOptionSelect: (label: string) => {
      void api.postCardAction(sessionId, { presentation_id: presentationId, action: "choose", values: { kind: "option", label } })
        .then((result) => { if (result.directive) appendUser(sessionId, result.directive); })
        .catch((error) => setNotice({ kind: "error", text: error instanceof Error ? error.message : String(error) }));
    },
    onValues: (values: Record<string, string>) => {
      void api.postCardAction(sessionId, { presentation_id: presentationId, action: "choose", values: { kind: "values", values } })
        .then((result) => { if (result.directive) appendUser(sessionId, result.directive); })
        .catch((error) => setNotice({ kind: "error", text: error instanceof Error ? error.message : String(error) }));
    },
    marked: state?.marked ?? [],
    onMark: (index: number, label: string, checked: boolean) =>
      api.postCardAction(sessionId, { presentation_id: presentationId, action: "mark", values: { kind: "item", index, label, checked } })
        .then(() => true)
        .catch((error) => { setNotice({ kind: "error", text: error instanceof Error ? error.message : String(error) }); return false; }),
  };
}

export function StructuredView(props: { output: import("../types").StructuredOutput; fallback?: string; sessionId?: string; resultId?: string; presentationId?: string; cardState?: import("../types").CardState | null }) {
  const [showOriginal, setShowOriginal] = createSignal(false);
  const fallback = () => props.fallback && !parseVakFence(props.fallback)
    ? <MarkdownView text={props.fallback} />
    : <details class="tool-details"><summary>View original result</summary><pre>{props.fallback || JSON.stringify(props.output.payload, null, 2)}</pre></details>;
  return <ErrorBoundary fallback={() => <section><p role="status">Rich presentation unavailable. Original content:</p>{fallback()}</section>}>
    <Show when={uiPreferences.richPreviews && props.output.schema_version === 2 && props.output.payload && typeof props.output.payload === "object"} fallback={fallback()}>
      <Show when={showOriginal() && showOperatorChrome()} fallback={
        <>
          <PresentationSessionContext.Provider value={props.sessionId}>
            <PresentationInteractionContext.Provider value={cardInteractionFor(props.sessionId, props.presentationId, props.cardState)}>
              <CardIdContext.Provider value={props.presentationId}>
                <div class="presentation-content"><StructuredRenderer output={props.output} /></div>
              </CardIdContext.Provider>
            </PresentationInteractionContext.Provider>
          </PresentationSessionContext.Provider>
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

/** The forms this client draws (docs/design/92-card-forms.md). */
export const structuredRendererTypes = formNames;

function StructuredRenderer(props: { output: import("../types").StructuredOutput }) {
  return <FormView block={props.output.block ?? ""} payload={props.output.payload} fallback={JSON.stringify(props.output.payload, null, 2)} />;
}


/** The form a card item is drawn as, as the host recorded it. */
function blockOf(item: OutputItem): string {
  return item.content.type === "structured" ? item.content.output.block ?? "" : "";
}

/** Lays out an answer's blocks as one document (docs/design/87-composed-answers.md §2):
 *  the summary leads, then the plan's sections in the order the model declared them,
 *  then every block that named no section, in the order it was produced. Nothing is
 *  placed in a section it did not name. A block the model placed beside the one
 *  before it shares that block's row where the answer is wide enough. */
function composeAnswer(
  entries: { item: OutputItem; node: JSX.Element }[],
  outline: import("../types").AnswerOutline | undefined,
): JSX.Element {
  const paired = new Set<string>();
  const block = (entry: { item: OutputItem; node: JSX.Element }) => (
    <div class="answer-block" data-block={blockOf(entry.item) || undefined} data-paired={paired.has(entry.item.id) ? "" : undefined}>
      {entry.node}
    </div>
  );
  // The first summary leads; a later one (a correction, a closing word)
  // stays where the model put it.
  const first = entries.find((entry) => blockOf(entry.item) === "summary");
  const lead = first ? [first] : [];
  const body = entries.filter((entry) => entry !== first);
  const sections = (outline?.sections ?? [])
    .map((section) => ({ name: section.name, entries: body.filter((entry) => section.items.includes(entry.item.id)) }))
    .filter((section) => section.entries.length > 0);
  const placed = new Set(sections.flatMap((section) => section.entries.map((entry) => entry.item.id)));
  // A row holds two blocks: one placed beside a block already in a row
  // takes the full width.
  const shown = [...lead, ...sections.flatMap((section) => section.entries), ...body.filter((entry) => !placed.has(entry.item.id))];
  const beside = new Set(outline?.beside ?? []);
  shown.forEach((entry, index) => {
    const previous = shown[index - 1];
    if (previous && beside.has(entry.item.id) && !paired.has(previous.item.id)) {
      paired.add(previous.item.id);
      paired.add(entry.item.id);
    }
  });
  return <>
    {lead.map(block)}
    {sections.map((section) => <section class="answer-section" aria-label={section.name}>{section.entries.map(block)}</section>)}
    {body.filter((entry) => !placed.has(entry.item.id)).map(block)}
  </>;
}

/** When cards mounted, so cards that land together come in one after
 *  another rather than all at once. */
const cardArrivals: number[] = [];

function arrivalDelay(): number {
  const now = performance.now();
  while (cardArrivals.length && now - cardArrivals[0] > 400) cardArrivals.shift();
  cardArrivals.push(now);
  return (cardArrivals.length - 1) * 90;
}

/** A running turn's answer as it forms (docs/design/96, live turn): each
 *  card as soon as it is recorded, coming in as it lands; the plan's
 *  progress is the presence wheel's (`AgentPresence`, `livePlan`). The
 *  settled turn replaces it. */
export function LiveAnswer(props: { sessionId: string }) {
  // A card already settled in the timeline belongs to its turn, not to this one.
  const cards = () => {
    const items = presentationOf(props.sessionId)?.items ?? [];
    const settled = new Set(items.filter((item) => item.provenance?.source !== "live_event")
      .map((item) => item.provenance?.presentation_id).filter(Boolean));
    return items.filter((item) => item.provenance?.source === "live_event" && item.kind === "card"
      && item.content.type === "structured" && !settled.has(item.provenance?.presentation_id));
  };
  return <>
    <Show when={cards().length > 0}>
      {/* The settled turn's frame (timeline column, indented body, result
          width), so the answer does not move when the settled turn replaces it. */}
      <div class="semantic-timeline"><div class="assistant-turn-body"><article class="primary-result">
      <div class="primary-result-material live-answer" aria-label="Answer so far">
        <For each={[...new Set(cards().map((item) => item.turn_id))]}>{(turn) =>
          <AnswerValuesScope scopeKey={conversationValuesKey(props.sessionId, turn)} items={cards().filter((item) => item.turn_id === turn)}>
          <For each={cards().filter((item) => item.turn_id === turn)}>{(item) => (
          <div class="answer-block card-arrive" style={{ "animation-delay": `${arrivalDelay()}ms` }} data-block={blockOf(item) || undefined}>
            <Show when={item.content.type === "structured" && item.content.output}>{(output) =>
              <StructuredView output={output()} fallback={item.fallback_text} sessionId={props.sessionId} presentationId={item.provenance?.presentation_id ?? undefined} />}
            </Show>
          </div>
          )}</For>
          </AnswerValuesScope>
        }</For>
      </div>
      </article></div></div>
    </Show>
  </>;
}

function Turn(props: { id: string; items: OutputItem[]; sessionId: string; allowContinuation: boolean; outline?: import("../types").AnswerOutline }) {
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

    // The server has already removed typed runtime replies and rejected
    // drafts. Projected prose stays visible regardless of its wording.
    return filtered;
  };

  const OrderedItem = (item: OutputItem): JSX.Element | null => {
    if (item.content.type === "ability") return <AbilityLine content={item.content} />;
    if (item.role === "user" && item.content.type === "document" && item.content.document.metadata.scheduled_name) {
      const document = item.content.document;
      return <ScheduledHead name={document.metadata.scheduled_name} failed={document.metadata.scheduled_failed === "true"} timestamp={item.timestamp} title={document.source_markdown} />;
    }
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
    if (item.content.type === "structured") return <StructuredView output={item.content.output} fallback={item.fallback_text} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? uniqueResultId() ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} cardState={item.card_state} />;
    if (item.content.type === "adaptive") return <AdaptiveTreeView tree={item.content.tree} fallback={item.content.fallback_text} sessionId={props.sessionId} resultId={item.outcome?.result_id ?? uniqueResultId() ?? item.id} presentationId={item.provenance?.presentation_id ?? undefined} cardState={item.card_state} />;
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
  const visible = createMemo(() => {
    // Card-only turns also need fresh nodes when the layout changes;
    // the prior layout disposed their reactive bodies.
    void showOperatorChrome();
    return ordered().map((item) => ({ item, node: OrderedItem(item) })).filter((entry) => entry.node !== null);
  });
  // Keep one author header and one set of result actions, while preserving
  // every projected prose/card boundary in source order.
  const blocks = createMemo(() => {
    const entries = visible();
    if (showOperatorChrome()) return entries.map((entry) => entry.node);
    return groupAnswerEntries(entries).map((group) => {
      if (group.kind === "item") return group.entry.node;
      const material = consolidateFilePreviews(group.entries);
      const anchor = [...material].reverse().find((entry) => answerDocument(entry.item))?.item
        ?? material.find((entry) => entry.item.outcome?.result_id)?.item ?? material[0].item;
      const resultId = group.resultId ?? uniqueResultId();
      const copyText = material.filter((entry) => entry.item.kind !== "artifact").map(({ item }) =>
        answerDocument(item)?.source_markdown ?? item.fallback_text).filter(Boolean).join("\n\n");
      return <AssistantMessage sessionId={props.sessionId} text={copyText}>
        <article class="primary-result" data-result-id={resultId ?? anchor.id} aria-label="Agent result">
          <Show when={anchor.status !== "succeeded"}><div class="primary-result-caution" role="status"><Icon name="warning" size={14} />The requested outcome is not verified. Check the evidence before relying on completion claims.</div></Show>
          {answerSegments(material).map((segment) => {
            const entry = segment[0];
            const document = answerDocument(entry.item);
            return document
              ? <div class="primary-result-answer"><PresentationDocumentView document={document} sessionId={props.sessionId} resultId={entry.item.outcome?.result_id ?? resultId ?? entry.item.id} presentationId={entry.item.provenance?.presentation_id ?? undefined} /></div>
              : <div class="primary-result-material" aria-label="Result material">{composeAnswer(segment, props.outline)}</div>;
          })}
          <ResultFiles files={material.filter((entry) => entry.item.kind === "artifact").map((entry) => entry.item)} answer={anchor} sessionId={props.sessionId} resultId={resultId} />
          <ResultEvidence item={anchor} />
          <ResultActions turnId={props.id} answer={anchor} material={material.map((entry) => entry.item)} sessionId={props.sessionId} resultId={resultId} />
        </article>
      </AssistantMessage>;
    });
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

export function AdaptiveTreeView(props: { tree: import("../types").AdaptiveRenderTree; fallback: string; sessionId?: string; resultId?: string; presentationId?: string; cardState?: import("../types").CardState | null }) {
  const original = () => <section aria-label="Original result"><p role="status">Rich presentation unavailable. Original content:</p><MarkdownView text={props.fallback} /></section>;
  return <ErrorBoundary fallback={original}>
    <Show when={uiPreferences.richPreviews && validateAdaptiveTree(props.tree?.root)} fallback={original()}>
      <section class="adaptive-presentation" aria-label={props.tree.accessibility_summary ?? "Adaptive presentation"}>
        <PresentationSessionContext.Provider value={props.sessionId}>
          <PresentationInteractionContext.Provider value={cardInteractionFor(props.sessionId, props.presentationId, props.cardState)}>
            <CardIdContext.Provider value={props.presentationId}>
              <PackTree node={props.tree.root} />
            </CardIdContext.Provider>
          </PresentationInteractionContext.Provider>
        </PresentationSessionContext.Provider>
      </section>
    </Show>
  </ErrorBoundary>;
}

/** True when an answer is drawn in the Canvas, where it is already open. */
export const InCanvasContext = createContext(false);

export default function PresentationTimelineView(props: { timeline: OutputTimeline; sessionId: string; allowContinuation?: boolean; hideUser?: boolean; valueScope?: string }) {
  // The automations a turn made or changed (docs/design/94 §5), read again
  // whenever the conversation grows, so a card follows its record.
  // A page has no conversation, so no automations of its own and nothing
  // to ask the owner's server for (docs/design/98 §12).
  const [fetchedAutomations, { refetch: refetchAutomations }] = createResource(
    () => (props.sessionId ? `${props.sessionId}:${props.timeline.items.length}` : null),
    () => api.listTriggers().then((list) => list.triggers.filter((trigger) => trigger.made_by?.kind === "turn" && trigger.made_by.session === props.sessionId)).catch(() => []),
  );
  // Kept by id, so a refresh updates a card in place.
  const [automationList, setAutomationList] = createStore<import("../types").Trigger[]>([]);
  createEffect(() => { const next = fetchedAutomations(); if (next) setAutomationList(reconcile(next, { key: "id" })); });
  const automations = () => automationList;
  const [ownerZone] = createResource(() => props.sessionId || null, () => api.getConfig().then((config) => config.owner?.timezone).catch(() => undefined));
  const automationsOf = (turn: string) => {
    const directive = props.timeline.items.find((item) => item.turn_id === turn && item.role === "user")?.provenance?.entry_id;
    return directive ? (automations() ?? []).filter((trigger) => trigger.made_by?.kind === "turn" && trigger.made_by.turn === directive) : [];
  };
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
      const outline = createMemo(() => props.timeline.answers?.find((answer) => answer.turn_id === id));
      return <>
        <AnswerValuesScope scopeKey={props.valueScope ?? conversationValuesKey(props.sessionId, id)} items={items()}>
          <Turn id={id} items={items()} outline={outline()} sessionId={props.sessionId} allowContinuation={props.allowContinuation ?? false} />
        </AnswerValuesScope>
        <For each={automationsOf(id)}>{(trigger) => <AutomationCard trigger={trigger} zone={ownerZone()} onChanged={() => void refetchAutomations()} />}</For>
      </>;
    }}</For>
  </div>;
}
