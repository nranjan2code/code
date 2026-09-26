import { For, Show, createContext, createEffect, createMemo, createSignal, onCleanup, useContext } from "solid-js";
import type { AdaptiveRenderNode } from "../../types";
import { chartGeometry, downloadCsv, type ChartData, type ChartPoint, type ChartSeries } from "./data";
import { safeUrl, sandboxedSrcdoc } from "../../safeUrl";
import { openInEditor, openComponentPreview, openArtifactCanvas, technicalDetails, uiPreferences } from "../../store";
import { artifactPreviewHtml } from "../../artifactPreview";
import * as api from "../../api";
import Icon from "../Icon";

/**
 * The generic, declarative renderer for presentation packs. Every key in
 * STRUCTURED_RENDERERS (PresentationRenderer.tsx) routes through this one
 * component via a `buildXSpec`/`renderX` pair per primitive — there are no
 * more per-semantic_type bespoke components.
 *
 * This intentionally reuses the exact node shape the Rust `vak-presentation`
 * crate already emits on the wire as `OutputContent::Adaptive` — see
 * `AdaptiveRenderNode` in ../../types.ts, which mirrors
 * `vak_presentation::RenderNode` (primitive/props/children). That pipeline
 * exists end-to-end (crates/vak-presentation/src/lib.rs `compile()` ->
 * crates/vak-delivery/src/presentation.rs `OutputContent::Adaptive` ->
 * PresentationRenderer.tsx). Certified active packs render real emitted
 * cards through this component. Other semantic types retain their structured
 * renderer until their definitions carry the complete content.
 *
 * Rather than inventing a second, parallel schema, this component consumes
 * the SAME `AdaptiveRenderNode` shape. The `buildXSpec` adapters below turn
 * raw payloads into that shape client-side for structured outputs. The server
 * compiler emits the same node shape for selected packs.
 *
 * `surface` is the one thing that varies per rendering context (main chat
 * timeline vs. a denser panel/compact view). It is deliberately NOT part of
 * the node schema — the schema stays surface-agnostic — but a top-level prop
 * on this component, so a second surface can be added later by extending the
 * switch below rather than by writing a second bespoke component.
 */
export type RenderSurface = "full" | "compact";

export interface GenericSpecRendererProps {
  node: AdaptiveRenderNode;
  surface?: RenderSurface;
  /** aria-label for the enclosing section; falls back to a generic label. */
  label?: string;
}

export const PresentationInteractionContext = createContext<{ onOptionSelect(label: string): void }>();

// Chart axis labels are drawn at fixed positions in a fixed-width SVG; a long
// category ("Week of September 14–18, 2026") otherwise runs off the chart and
// collides with the label at the other end. The full text stays available as
// the label's <title> tooltip.
const AXIS_LABEL_MAX = 14;
function axisLabel(value: unknown): string {
  const text = String(value);
  return text.length > AXIS_LABEL_MAX ? `${text.slice(0, AXIS_LABEL_MAX - 1)}…` : text;
}

function str(props: Record<string, unknown>, key: string): string | undefined {
  const value = props[key];
  return typeof value === "string" ? value : typeof value === "number" ? String(value) : undefined;
}

/**
 * Render one primitive node. Every registered primitive (timeline, metric,
 * metric_grid, table, comparison, recipe, research, diff, terminal,
 * test_matrix, chart, ui_preview, media, universal_card) gets bespoke,
 * surface-aware treatment here; anything unrecognized or malformed degrades
 * through the same safe generic fallback the existing `AdaptiveTreeView`
 * (PresentationRenderer.tsx) already uses, so it never renders blank and
 * never throws.
 */
function renderNode(node: AdaptiveRenderNode | null | undefined, surface: RenderSurface): ReturnType<typeof renderTimeline> {
  if (!node || typeof node !== "object" || typeof node.primitive !== "string") {
    return <></>;
  }
  const props = node.props && typeof node.props === "object" ? node.props : {};
  const children = Array.isArray(node.children) ? node.children : [];
  const safeNode: AdaptiveRenderNode = { primitive: node.primitive, props, children };

  switch (safeNode.primitive.toLowerCase()) {
    case "timeline":
      return renderTimeline(safeNode, surface);
    case "metric":
      return renderMetric(safeNode, surface);
    case "metric_grid":
      return renderMetricGrid(safeNode, surface);
    case "table":
    case "comparison":
      return renderTable(safeNode, surface);
    case "recipe":
      return renderRecipe(safeNode, surface);
    case "research":
      return renderResearch(safeNode, surface);
    case "diff":
      return renderDiff(safeNode, surface);
    case "terminal":
      return renderTerminal(safeNode, surface);
    case "test_matrix":
      return renderTestMatrix(safeNode, surface);
    case "chart":
      return renderChart(safeNode, surface);
    case "ui_preview":
      return renderUiPreview(safeNode, surface);
    case "media":
      return renderMedia(safeNode, surface);
    case "universal_card":
      return renderUniversalCard(safeNode, surface);
    case "map":
    case "calendar":
    case "board":
    case "entity":
    case "evidence":
    case "graph":
    case "form":
    case "alert":
    case "conversation":
    case "transaction": {
      // `kind` names the card itself (its accessible label), not a field.
      const entries = Object.entries(props).filter(([key]) => !["title", "summary", "semantic_type", "kind"].includes(key));
      return renderUniversalCard({ primitive: "universal_card", props: { ...props, kind: str(props, "kind") ?? safeNode.primitive.replaceAll("_", " "), entries }, children }, surface);
    }
    default:
      return renderFallback(safeNode, surface);
  }
}

/** The alternatives a plan step offers (its typed `options`), keeping only
 * well-formed entries. */
function stepOptions(props: Record<string, unknown>): TimelineOption[] {
  const raw = props.options;
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((option): TimelineOption[] => {
    if (!option || typeof option !== "object") return [];
    const record = option as Record<string, unknown>;
    if (typeof record.label !== "string" || !record.label.trim()) return [];
    return [{
      label: record.label,
      detail: typeof record.detail === "string" ? record.detail : undefined,
      facts: Array.isArray(record.facts) ? record.facts.filter((fact): fact is string => typeof fact === "string") : [],
    }];
  });
}

/**
 * A plan, or a list of options (`variant: "options"`). A plan whose steps
 * offer typed `options` is drawn with those options beside it (doc 70
 * screen 2): the step says how many there are, and each option can be used
 * in the plan. A narrow column stacks them.
 */
function renderTimeline(node: AdaptiveRenderNode, surface: RenderSurface) {
  const title = str(node.props, "title") ?? "Overview";
  const kicker = str(node.props, "kicker") ?? "Plan";
  const options = str(node.props, "variant") === "options";
  const interaction = useContext(PresentationInteractionContext);
  const items = Array.isArray(node.children) ? node.children : [];
  const compact = surface === "compact";
  const itemPropsOf = (item: AdaptiveRenderNode) => (item && typeof item === "object" && item.props && typeof item.props === "object" ? item.props : {});
  const choices = options ? [] : items.flatMap((item) => {
    const itemProps = itemPropsOf(item);
    const offered = stepOptions(itemProps);
    return offered.length > 0 ? [{ label: str(itemProps, "label") ?? str(itemProps, "title") ?? "Step", options: offered }] : [];
  });
  // A status every step shares ("suggested" on each) tells the reader
  // nothing; a step that offers options already says so.
  const statuses = new Set(items.filter((item) => stepOptions(itemPropsOf(item)).length === 0).map((item) => str(itemPropsOf(item), "status")?.trim().toLowerCase() ?? ""));
  const uniformStatus = items.length > 1 && statuses.size === 1;
  const list = <For each={items}>
    {(item, index) => {
      const itemProps = itemPropsOf(item);
      const offered = options ? [] : stepOptions(itemProps);
      const label = str(itemProps, "label") ?? str(itemProps, "title") ?? "Item";
      const detail = str(itemProps, "detail");
      const status = str(itemProps, "status");
      const time = str(itemProps, "time");
      const normalizedStatus = status?.trim().toLowerCase().replace(/[ _-]+/g, "");
      const complete = normalizedStatus === "complete" || normalizedStatus === "completed" || normalizedStatus === "done";
      const showStatus = Boolean(status && !["pending", "todo", "notstarted"].includes(normalizedStatus ?? "") && offered.length === 0 && (complete || !uniformStatus));
      return <li classList={{ complete, "is-option": options }}>
        <Show when={!options}><span class="adaptive-timeline-marker" aria-hidden="true">{index() + 1}</span></Show>
        <div>
          <strong>{label}</strong>
          <Show when={time}><small class="adaptive-timeline-time">{time}</small></Show>
          <Show when={detail && !compact}><p>{detail}</p></Show>
          <Show when={offered.length > 0}><small class="adaptive-timeline-choice">{offered.length === 1 ? "1 option" : `${offered.length} options`} to choose from</small></Show>
          <Show when={showStatus}><small class="adaptive-timeline-status">{complete ? "Done" : status}</small></Show>
        </div>
        <Show when={options && interaction && !compact}>
          <button type="button" class="adaptive-option-action" onClick={() => interaction?.onOptionSelect(label)} aria-label={`Use ${label}`}>Use this</button>
        </Show>
      </li>;
    }}
  </For>;
  const timeline = (
    <section class="adaptive-timeline" classList={{ "adaptive-timeline-compact": compact, "adaptive-options": options }} aria-label={title}>
      <header class="adaptive-timeline-head">
        <Show when={!options}><span class="adaptive-timeline-kicker">{kicker}</span></Show>
        <h3>{title}</h3>
      </header>
      <Show when={options} fallback={<ol>{list}</ol>}><ul>{list}</ul></Show>
    </section>
  );
  if (choices.length === 0 || compact) return timeline;
  return (
    <div class="adaptive-plan">
      <div class="adaptive-plan-grid">
        {timeline}
        <div class="adaptive-plan-choices">
          <For each={choices}>
            {(step) => (
              <section class="adaptive-timeline adaptive-options" aria-label={`Options for ${step.label}`}>
                <header class="adaptive-timeline-head">
                  <span class="adaptive-timeline-kicker">Options</span>
                  <h3>{step.label}</h3>
                </header>
                <ul>
                  <For each={step.options}>
                    {(option) => (
                      <li class="is-option">
                        <div>
                          <strong>{option.label}</strong>
                          <Show when={option.detail}><p>{option.detail}</p></Show>
                          <Show when={option.facts.length > 0}><small>{option.facts.join(" · ")}</small></Show>
                        </div>
                        <Show when={interaction}>
                          <button type="button" class="adaptive-option-action" onClick={() => interaction?.onOptionSelect(option.label)} aria-label={`Use ${option.label}`}>Use this</button>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </section>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

function renderMetric(node: AdaptiveRenderNode, surface: RenderSurface) {
  // A declarative pack may bind the whole validated metric payload to this
  // primitive. The emit tool permits both one value and a small grid; keep
  // the latter visible instead of showing a single-value dash.
  if (node.props.value === undefined || node.props.value === null) {
    const grid = buildMetricSpec(node.props);
    if (grid.primitive === "metric_grid") return renderMetricGrid(grid, surface);
  }
  const label = str(node.props, "label") ?? "Metric";
  const value = node.props["value"];
  const unit = str(node.props, "unit");
  const displayValue = typeof value === "string" || typeof value === "number" ? String(value) : "—";
  if (surface === "compact") {
    // Compact surfaces (e.g. an inline chip in a dense panel) collapse the
    // label/value pair onto one line instead of the stacked <small>/<strong>.
    return (
      <div class="adaptive-node adaptive-metric adaptive-metric-compact">
        <span>{label}: <strong>{displayValue}{unit ? ` ${unit}` : ""}</strong></span>
      </div>
    );
  }
  return (
    <div class="canvas-card rich-metric">
      <small>{label}</small>
      <strong>{displayValue}{unit ? ` ${unit}` : ""}</strong>
    </div>
  );
}

/**
 * Multi-field metric payloads (e.g. weather: temp/humidity/wind) render as a
 * small grid instead of one label/value pair — this mirrors the legacy
 * "metric-grid-card" branch of the inline metric handler in
 * PresentationRenderer.tsx exactly, including its CSS class names, so the
 * migration causes no visual regression for that shape.
 */
function renderMetricGrid(node: AdaptiveRenderNode, surface: RenderSurface) {
  const header = str(node.props, "title");
  const compact = surface === "compact";
  const entries = Array.isArray(node.children) ? node.children : [];
  return (
    <div class="canvas-card metric-grid-card" classList={{ "metric-grid-card-compact": compact }}>
      <Show when={header}>
        <div class="metric-grid-header">{header}</div>
      </Show>
      <div class="metric-grid-container">
        <For each={entries}>
          {(entry) => {
            const entryProps = entry && typeof entry === "object" && entry.props && typeof entry.props === "object" ? entry.props : {};
            const key = str(entryProps, "label") ?? "";
            const value = entryProps["value"];
            return (
              <div class="metric-grid-item">
                <small class="metric-grid-label">{key}</small>
                <strong class="metric-grid-value">{typeof value === "string" || typeof value === "number" ? String(value) : JSON.stringify(value ?? "—")}</strong>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}

interface SpecColumn {
  key: string;
  label: string;
  isNumeric?: boolean;
}

function specColumns(node: AdaptiveRenderNode): SpecColumn[] {
  const raw = node.props["columns"];
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((c): c is Record<string, unknown> => !!c && typeof c === "object")
    .map((c) => ({
      key: String(c.key ?? ""),
      label: String(c.label ?? c.key ?? ""),
      isNumeric: Boolean(c.isNumeric),
    }));
}

function specRows(node: AdaptiveRenderNode): Record<string, unknown>[] {
  const children = Array.isArray(node.children) ? node.children : [];
  return children.map((child) =>
    child && typeof child === "object" && child.props && typeof child.props === "object" ? child.props : {},
  );
}

/**
 * `table` / `comparison` primitive. Mirrors the legacy DataGrid component's
 * markup, class names and interactions (search, per-column sort, CSV export)
 * exactly, so migrating a registry key causes no visual or behavioral change.
 */
function renderTable(node: AdaptiveRenderNode, surface: RenderSurface) {
  const [search, setSearch] = createSignal("");
  const [sortCol, setSortCol] = createSignal<string | null>(null);
  const [sortAsc, setSortAsc] = createSignal<boolean>(true);
  const [copied, setCopied] = createSignal(false);
  const compact = surface === "compact";
  const firstColumn = specColumns(node)[0];
  const options = str(node.props, "variant") === "options" || (
    specRows(node).length > 0 && specRows(node).length <= 8 &&
    Boolean(firstColumn && /^(option|choice)$/i.test(firstColumn.key.trim()))
  );
  const quiet = options || (specRows(node).length <= 5 && specColumns(node).length <= 6);
  const interaction = useContext(PresentationInteractionContext);

  const columns = () => specColumns(node);
  const rows = () => specRows(node);

  const handleSort = (colKey: string) => {
    if (sortCol() === colKey) {
      setSortAsc(!sortAsc());
    } else {
      setSortCol(colKey);
      setSortAsc(true);
    }
  };

  const filteredAndSortedRows = createMemo(() => {
    let list = [...rows()];
    const q = search().toLowerCase().trim();
    if (q) {
      list = list.filter((row) => Object.values(row || {}).some((val) => String(val).toLowerCase().includes(q)));
    }
    const col = sortCol();
    if (col) {
      const isNum = columns().find((c) => c.key === col)?.isNumeric;
      list.sort((a, b) => {
        const valA = a[col];
        const valB = b[col];
        if (isNum) {
          const numA = valA == null || valA === "" ? NaN : Number(valA);
          const numB = valB == null || valB === "" ? NaN : Number(valB);
          if (!Number.isFinite(numA)) return Number.isFinite(numB) ? 1 : 0;
          if (!Number.isFinite(numB)) return -1;
          return sortAsc() ? numA - numB : numB - numA;
        }
        const strA = String(valA ?? "").toLowerCase();
        const strB = String(valB ?? "").toLowerCase();
        return sortAsc() ? strA.localeCompare(strB) : strB.localeCompare(strA);
      });
    }
    return list;
  });

  const handleExportCsv = () => {
    const cols = columns();
    downloadCsv("dataset.csv", [cols.map((c) => c.label), ...filteredAndSortedRows().map((row) => cols.map((c) => row[c.key]))]);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div class="canvas-card data-grid-wrap" classList={{ "data-grid-wrap-compact": compact, "data-grid-options": options }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">{options ? "Options" : quiet ? "Table" : "Data Grid"}</span>
          <span class="card-subtitle">{str(node.props, "title") ?? "Dataset Records"}</span>
          <Show when={!quiet}><span class="card-badge" style={{ opacity: "0.8" }}>
            {filteredAndSortedRows().length} of {rows().length} rows
          </span></Show>
        </div>
        <Show when={!quiet}><div class="card-actions">
          <input
            type="text"
            class="grid-search-input"
            placeholder="Search records..."
            aria-label="Search dataset records"
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
          <button type="button" class="pill-action-btn" onClick={handleExportCsv}>
            {copied() ? "Downloaded" : "Download CSV"}
          </button>
        </div></Show>
      </div>

      <div class="grid-table-container">
        <table class="sleek-grid">
          <thead>
            <tr>
              <For each={columns()}>
                {(col) => (
                  <th
                    class={col.isNumeric ? "cell-numeric" : ""}
                    scope="col"
                    aria-sort={sortCol() === col.key ? (sortAsc() ? "ascending" : "descending") : "none"}
                  >
                    <Show when={!quiet} fallback={<span>{col.label}</span>}>
                      <button type="button" onClick={() => handleSort(col.key)}>
                        {col.label} {sortCol() === col.key ? (sortAsc() ? "↑" : "↓") : "↕"}
                      </button>
                    </Show>
                  </th>
                )}
              </For>
              <Show when={options && interaction}><th scope="col"><span class="sr-only">Choose</span></th></Show>
            </tr>
          </thead>
          <tbody>
            <For each={filteredAndSortedRows()}>
              {(row) => (
                <tr>
                  <For each={columns()}>
                    {(col) => {
                      const val = String(row[col.key] ?? "");
                      const isStatus = col.key.toLowerCase().includes("status");
                      return (
                        <td
                          class={col.isNumeric ? "cell-numeric" : ""}
                          style={{
                            "font-weight": isStatus ? "600" : undefined,
                          }}
                        >
                          {val}
                        </td>
                      );
                    }}
                  </For>
                  <Show when={options && interaction}>{(() => {
                    const firstColumn = columns()[0]?.key;
                    const label = String((firstColumn && row[firstColumn]) ?? "this option");
                    return <td class="option-table-action"><button type="button" class="adaptive-option-action" onClick={() => interaction?.onOptionSelect(label)} aria-label={`Use ${label}`}>Use this</button></td>;
                  })()}</Show>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}

function childrenOf(node: AdaptiveRenderNode, primitive: string): AdaptiveRenderNode[] {
  const group = (Array.isArray(node.children) ? node.children : []).find(
    (c) => c && typeof c === "object" && c.primitive === primitive,
  );
  return group && Array.isArray(group.children) ? group.children : [];
}

function num(props: Record<string, unknown>, key: string): number | undefined {
  const value = props[key];
  return typeof value === "number" ? value : undefined;
}

/**
 * `recipe` primitive. Mirrors the legacy RecipeCard: scalable servings
 * stepper, checkable ingredient rows, numbered directions and per-step
 * countdown timers, with identical markup and class names.
 */
function renderRecipe(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const baseServings = num(node.props, "servings") ?? 1;
  const hasServings = num(node.props, "servings") !== undefined;
  const cookTime = num(node.props, "cook_time_minutes");
  const [servings, setServings] = createSignal(baseServings);
  const [activeTimers, setActiveTimers] = createSignal<Record<number, number>>({});
  const [timerIntervals, setTimerIntervals] = createSignal<Record<number, ReturnType<typeof setInterval>>>({});
  const scale = () => servings() / baseServings;

  const toggleTimer = (idx: number, totalSeconds: number) => {
    const intervals = timerIntervals();
    if (intervals[idx]) {
      clearInterval(intervals[idx]);
      const newIntervals = { ...intervals };
      delete newIntervals[idx];
      setTimerIntervals(newIntervals);
      const newTimers = { ...activeTimers() };
      delete newTimers[idx];
      setActiveTimers(newTimers);
      return;
    }
    setActiveTimers({ ...activeTimers(), [idx]: totalSeconds });
    const deadline = Date.now() + totalSeconds * 1000;
    const interval = setInterval(() => {
      setActiveTimers((prev) => {
        const current = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
        if (current === 0) {
          clearInterval(interval);
          const nextIntervals = { ...timerIntervals() };
          delete nextIntervals[idx];
          setTimerIntervals(nextIntervals);
          return { ...prev, [idx]: 0 };
        }
        return { ...prev, [idx]: current };
      });
    }, 1000);
    setTimerIntervals({ ...intervals, [idx]: interval });
  };

  onCleanup(() => {
    for (const id of Object.values(timerIntervals())) clearInterval(id);
  });

  const formatTimer = (seconds: number) => {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return seconds === 0 ? "Timer complete · Restart" : `${m}:${s < 10 ? "0" : ""}${s} · Stop`;
  };

  return (
    <div class="canvas-card culinary-card-wrap" classList={{ "culinary-card-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-amber">Culinary Recipe</span>
          <span class="card-subtitle">
            {str(node.props, "title") ?? "Recipe"}
            <Show when={cookTime !== undefined}> · {cookTime} min</Show>
          </span>
        </div>
        <div class="card-actions">
          <Show when={hasServings}>
            <span style={{ "font-size": "12px", color: "var(--text-muted)" }}>Servings:</span>
            <div class="servings-stepper">
              <button type="button" aria-label="Decrease servings" disabled={servings() <= 1} onClick={() => setServings(Math.max(1, servings() - 1))}>-</button>
              <span class="servings-num">{servings()}</span>
              <button type="button" aria-label="Increase servings" disabled={servings() >= 10000} onClick={() => setServings(servings() + 1)}>+</button>
            </div>
          </Show>
        </div>
      </div>

      <div class="culinary-stage">
        <div class="ingredients-column">
          <span class="column-subhead">Ingredients</span>
          <For each={childrenOf(node, "ingredient_list")}>
            {(item) => {
              const itemProps = item && typeof item === "object" && item.props && typeof item.props === "object" ? item.props : {};
              if (itemProps["raw"] === true) {
                return (
                  <label class="ingredient-checkbox-row">
                    <input type="checkbox" />
                    <span>{str(itemProps, "text") ?? ""}</span>
                  </label>
                );
              }
              const name = () => str(itemProps, "name") ?? "";
              const amount = num(itemProps, "amount");
              const unit = str(itemProps, "unit");
              const scaledAmount = () => (amount === undefined ? null : Math.round(amount * scale() * 100) / 100);
              return (
                <label class="ingredient-checkbox-row">
                  <input type="checkbox" />
                  <span>
                    {scaledAmount() === null ? "" : `${scaledAmount()} `}
                    {unit ? `${unit} ` : ""}
                    {name()}
                  </span>
                </label>
              );
            }}
          </For>
        </div>

        <div class="directions-column">
          <span class="column-subhead">Directions &amp; Timers</span>
          <For each={childrenOf(node, "step_list")}>
            {(step, idx) => {
              const stepProps = step && typeof step === "object" && step.props && typeof step.props === "object" ? step.props : {};
              const text = str(stepProps, "text") ?? "";
              const timerSecs = num(stepProps, "timer_seconds") ?? null;
              const isRunning = () => activeTimers()[idx()] !== undefined;
              return (
                <div class="timer-action-card" classList={{ "active-timer": isRunning() }}>
                  <div>
                    <div style={{ "font-size": "13px", "font-weight": "600", color: "var(--text-main)" }}>
                      {idx() + 1}. {text}
                    </div>
                  </div>
                  {timerSecs && (
                    <button class="timer-trigger-btn" classList={{ running: isRunning() }} onClick={() => toggleTimer(idx(), timerSecs)}>
                      {isRunning() ? formatTimer(activeTimers()[idx()]) : `Start ${Math.floor(timerSecs / 60)}:${String(timerSecs % 60).padStart(2, "0")} timer`}
                    </button>
                  )}
                </div>
              );
            }}
          </For>
        </div>
      </div>
    </div>
  );
}

/**
 * `research` primitive. Mirrors the legacy ResearchCards: numbered takeaways
 * with hoverable inline citations resolved against the source shelf, and a
 * copy-synthesis action. URLs still pass through `safeUrl` before they reach
 * an href, exactly as the legacy component does.
 */
function renderResearch(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [copied, setCopied] = createSignal(false);
  const takeaways = () => childrenOf(node, "takeaway_list");
  const sources = () => childrenOf(node, "source_list");
  const sourceProps = (index: number): Record<string, unknown> | undefined => {
    const source = sources()[index];
    return source && typeof source === "object" && source.props && typeof source.props === "object" ? source.props : undefined;
  };

  const handleCopy = async () => {
    const text = takeaways()
      .map((t) => (t?.props && typeof t.props === "object" ? (str(t.props, "text") ?? "") : ""))
      .join("\n\u2022 ");
    try {
      await navigator.clipboard.writeText(`\u2022 ${text}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div class="canvas-card research-card-wrap" classList={{ "research-card-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">{str(node.props, "title") ?? "Research Synthesis"}</span>
          <span class="card-subtitle">{sources().length} sources</span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "\u2713 Copied" : "Copy Synthesis"}
          </button>
        </div>
      </div>

      <div class="research-grid">
        <div class="takeaways-list">
          <For each={takeaways()}>
            {(item, idx) => {
              const itemProps = item && typeof item === "object" && item.props && typeof item.props === "object" ? item.props : {};
              const text = str(itemProps, "text") ?? "";
              const rawCites = itemProps["citation_indices"];
              const cites = Array.isArray(rawCites) ? (rawCites as number[]) : [];
              return (
                <div class="takeaway-row">
                  <div class="takeaway-bullet">{idx() + 1}</div>
                  <div class="takeaway-content">
                    <span>{text}</span>
                    <For each={cites}>
                      {(citeIdx) => {
                        const source = sourceProps(Number(citeIdx) - 1);
                        return (
                          <span class="inline-cite">
                            [{citeIdx}]
                            <Show when={source}>
                              <div class="cite-popover">
                                <div class="cite-source-badge">{(source && str(source, "source_name")) ?? "Source"}</div>
                                <strong>{source && str(source, "title")}</strong>
                                <Show when={source && str(source, "snippet")}>
                                  <p class="cite-snippet">"{source && str(source, "snippet")}"</p>
                                </Show>
                              </div>
                            </Show>
                          </span>
                        );
                      }}
                    </For>
                  </div>
                </div>
              );
            }}
          </For>
        </div>

        <Show when={sources().length > 0}>
          <div class="source-shelf">
            <For each={sources()}>
              {(src) => {
                const p = src && typeof src === "object" && src.props && typeof src.props === "object" ? src.props : {};
                const url = str(p, "url") ?? "";
                return (
                  <a class="source-tile" href={safeUrl(url) ? url : undefined} target="_blank" rel="noreferrer noopener">
                    <div class="source-favicon">🌐</div>
                    <div class="source-info">
                      <strong>{str(p, "title")}</strong>
                      <small>{str(p, "source_name") ?? url}</small>
                    </div>
                  </a>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}

/**
 * `diff` primitive (maps to `vak_presentation::Primitive::Diff`). Mirrors the
 * legacy DiffInspector: a per-file sidebar (only when more than one file), a
 * unified/side-by-side toggle, copy and open-in-editor actions, and a gutter
 * with reconstructed old/new line numbers. Markup and class names are
 * identical to the old component's.
 */
function renderDiff(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [activeIdx, setActiveIdx] = createSignal(0);
  const [splitMode, setSplitMode] = createSignal(false);
  const [copied, setCopied] = createSignal(false);

  const files = () => (Array.isArray(node.children) ? node.children : []).map((child) =>
    child && typeof child === "object" && child.props && typeof child.props === "object" ? child.props : {},
  );
  const activeFile = () => files()[activeIdx()] ?? files()[0];

  const parsedLines = createMemo(() => {
    const text = str(activeFile() ?? {}, "hunks") ?? "";
    const lines = text.split("\n");
    let oldLine = 1;
    let newLine = 1;
    return lines.map((line) => {
      if (line.startsWith("@@")) {
        const match = /@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
        if (match) {
          oldLine = parseInt(match[1], 10);
          newLine = parseInt(match[2], 10);
        }
        return { type: "hunk" as const, text: line, oldNum: "...", newNum: "..." };
      }
      if (line.startsWith("+") && !line.startsWith("+++")) {
        const item = { type: "add" as const, text: line, oldNum: "", newNum: String(newLine) };
        newLine++;
        return item;
      }
      if (line.startsWith("-") && !line.startsWith("---")) {
        const item = { type: "del" as const, text: line, oldNum: String(oldLine), newNum: "" };
        oldLine++;
        return item;
      }
      const item = { type: "normal" as const, text: line, oldNum: String(oldLine), newNum: String(newLine) };
      oldLine++;
      newLine++;
      return item;
    });
  });

  const handleCopy = () => {
    void navigator.clipboard
      .writeText(str(activeFile() ?? {}, "hunks") ?? "")
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => setCopied(false));
  };

  const handleOpen = () => {
    const fn = str(activeFile() ?? {}, "filename");
    if (fn) openInEditor(fn);
  };

  return (
    <div class="canvas-card diff-inspector-wrap" classList={{ "diff-inspector-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Diff Inspector</span>
          <span class="card-subtitle">{str(activeFile() ?? {}, "filename")}</span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={() => setSplitMode(!splitMode())}>
            {splitMode() ? "Unified" : "Side-by-Side"}
          </button>
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : "Copy Diff"}
          </button>
          <button type="button" class="pill-action-btn" onClick={handleOpen}>
            Open in Editor
          </button>
        </div>
      </div>

      <div class="diff-workspace">
        <Show when={files().length > 1}>
          <div class="diff-file-sidebar">
            <For each={files()}>
              {(f, idx) => (
                <div class="diff-file-item" classList={{ active: idx() === activeIdx() }} onClick={() => setActiveIdx(idx())}>
                  <span class="diff-file-name">{str(f, "filename")}</span>
                  <span class="diff-delta-pill">
                    <span style={{ color: "var(--emerald-bright)" }}>+{num(f, "additions") ?? 0}</span>{" "}
                    <span style={{ color: "var(--rose-bright)" }}>-{num(f, "deletions") ?? 0}</span>
                  </span>
                </div>
              )}
            </For>
          </div>
        </Show>

        <div class="diff-code-area" classList={{ "split-mode": splitMode() }}>
          <For each={parsedLines()}>
            {(row) => (
              <div class={`diff-row ${row.type}`}>
                <span class="diff-num">{row.oldNum || " "}</span>
                <span class="diff-num">{row.newNum || " "}</span>
                <span class="diff-line-text">{row.text}</span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

/**
 * `terminal` primitive (maps to `vak_presentation::Primitive::Terminal`).
 * Mirrors the legacy TerminalConsole exactly. The old component is a plain
 * static transcript view (no xterm.js — `@xterm/xterm` is only used by the
 * live TerminalPane), so there is no emulator to delegate to here.
 */
function renderTerminal(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [copied, setCopied] = createSignal(false);
  const [copyError, setCopyError] = createSignal(false);
  const output = () => str(node.props, "output") ?? "";
  const command = () => str(node.props, "command");
  const exitCode = () => num(node.props, "exit_code");
  const durationMs = () => num(node.props, "duration_ms");

  const handleCopy = async () => {
    setCopyError(false);
    try {
      await navigator.clipboard.writeText(output());
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopyError(true);
    }
  };

  const isSuccess = () => exitCode() === 0;

  return (
    <div class="canvas-card terminal-wrap" classList={{ "terminal-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class={`card-badge ${isSuccess() ? "badge-indigo" : "badge-rose"}`}>Terminal</span>
          <span class="card-subtitle">
            {exitCode() === undefined ? "Exit status unavailable" : `Exit ${exitCode()}`}
            <Show when={durationMs() !== undefined}>
              {" "}· {durationMs()}ms
            </Show>
          </span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : copyError() ? "Copy failed" : "Copy Output"}
          </button>
        </div>
      </div>

      <div class="sleek-terminal">
        <Show when={command()}>
          <div>
            <span class="term-line-cmd">$</span> {command()}
          </div>
        </Show>
        <pre class="term-output-text">{output()}</pre>
      </div>
    </div>
  );
}

/**
 * `test_matrix` primitive (maps to `vak_presentation::Primitive::TestMatrix`).
 * Mirrors the legacy TestMatrix: all/failed filter chips, a success-rate ring,
 * pass/fail counters and per-test cards with an optional traceback drawer.
 */
function renderTestMatrix(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [filter, setFilter] = createSignal<"all" | "failed">("all");

  const tests = () => (Array.isArray(node.children) ? node.children : []).map((child) =>
    child && typeof child === "object" && child.props && typeof child.props === "object" ? child.props : {},
  );

  const passedCount = createMemo(() => num(node.props, "passed") ?? tests().filter((t) => str(t, "status") === "passed").length);
  const failedCount = createMemo(() => num(node.props, "failed") ?? tests().filter((t) => str(t, "status") === "failed").length);
  const totalCount = createMemo(() => num(node.props, "total") ?? tests().length);
  const hasReportedOutcome = createMemo(() =>
    totalCount() > 0 && (passedCount() > 0 || failedCount() > 0 || tests().some((t) => {
      const status = str(t, "status");
      return status === "passed" || status === "failed" || status === "skipped";
    })),
  );
  const successPercent = createMemo(() => {
    const tot = totalCount();
    if (!hasReportedOutcome()) return 0;
    return Math.round((passedCount() / tot) * 100);
  });
  const filteredTests = createMemo(() =>
    filter() === "failed" ? tests().filter((t) => str(t, "status") === "failed") : tests(),
  );

  return (
    <div class="canvas-card test-matrix-wrap" classList={{ "test-matrix-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class={`card-badge ${failedCount() > 0 ? "badge-rose" : hasReportedOutcome() ? "badge-emerald" : ""}`}>Test Suite</span>
          <span class="card-subtitle">
            {str(node.props, "suite_name") ?? "Test Execution"} ({num(node.props, "duration_ms") ?? 0}ms)
          </span>
        </div>
        <div class="test-tab-filter">
          <button class="filter-chip" classList={{ active: filter() === "all" }} onClick={() => setFilter("all")}>
            All ({totalCount()})
          </button>
          <button class="filter-chip" classList={{ active: filter() === "failed" }} onClick={() => setFilter("failed")}>
            Failed ({failedCount()})
          </button>
        </div>
      </div>

      <div class="test-dashboard">
        <div class="test-stats-row">
          <div class="test-progress-group">
            <div class="test-ring-wrapper">
              <svg viewBox="0 0 36 36">
                <path
                  d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                  fill="none"
                  stroke="var(--rose-wash, rgba(244, 63, 94, 0.25))"
                  stroke-width="3.5"
                />
                <path
                  d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                  fill="none"
                  stroke={hasReportedOutcome() ? "var(--emerald-bright)" : "var(--text-muted)"}
                  stroke-width="3.5"
                  stroke-dasharray={`${successPercent()}, 100`}
                  stroke-linecap="round"
                />
              </svg>
            </div>
            <div>
              <div style={{ "font-size": "14px", "font-weight": "700", color: "var(--text-main)" }}>
                {hasReportedOutcome() ? `${successPercent()}% Success Rate` : totalCount() === 0 ? "No tests reported" : "Results unavailable"}
              </div>
              <div style={{ "font-size": "11.5px", color: "var(--text-muted)" }}>
                {passedCount()} of {totalCount()} tests verified
              </div>
            </div>
          </div>
          <div style={{ display: "flex", gap: "14px", "font-size": "12px", "font-weight": "600" }}>
            <Show when={hasReportedOutcome()}>
              <span style={{ color: "var(--emerald-bright)" }}>● {passedCount()} Passed</span>
            </Show>
            <Show when={failedCount() > 0}>
              <span style={{ color: "var(--rose-bright)" }}>✕ {failedCount()} Failed</span>
            </Show>
          </div>
        </div>

        <div class="test-grid-list">
          <For each={filteredTests()}>
            {(t) => {
              const status = str(t, "status");
              const passed = status === "passed";
              const failed = status === "failed";
              const duration = num(t, "duration_ms");
              const detail = str(t, "traceback") ?? str(t, "message");
              return (
                <div class="test-item-card" classList={{ "fail-card": failed }}>
                  <div style={{ width: "100%" }}>
                    <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
                      <div>
                        <span
                          style={{
                            color: failed ? "var(--rose-bright)" : passed ? "var(--emerald-bright)" : "var(--text-muted)",
                            "font-weight": "bold",
                            "margin-right": "6px",
                          }}
                        >
                          {failed ? "✕" : passed ? "✓" : "•"}
                        </span>
                        <strong style={{ color: "var(--text-main)" }}>{str(t, "name")}</strong>
                      </div>
                      <Show when={duration !== undefined}>
                        <span style={{ color: "var(--text-muted)", "font-size": "11px" }}>{duration}ms</span>
                      </Show>
                    </div>
                    <Show when={detail}>
                      <div class="traceback-drawer">{detail}</div>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </div>
    </div>
  );
}

/**
 * Reconstruct the `ChartData` shape the shared charting helpers in ./data.ts
 * expect from a `chart` primitive node. Only the wrapper is genericized — the
 * scales/segments/valueAt math still comes from `chartGeometry`, exactly as the
 * legacy UniversalChart used it.
 */
function chartDataOf(node: AdaptiveRenderNode): ChartData {
  const rawType = str(node.props, "chart_type");
  const chart_type: ChartData["chart_type"] = rawType === "bar" || rawType === "area" ? rawType : "line";
  const series: ChartSeries[] = (Array.isArray(node.children) ? node.children : [])
    .filter((c): c is AdaptiveRenderNode => !!c && typeof c === "object")
    .map((c) => {
      const p = c.props && typeof c.props === "object" ? c.props : {};
      const rawPoints = Array.isArray(p["points"]) ? (p["points"] as unknown[]) : [];
      return {
        name: str(p, "name") ?? "Series",
        points: rawPoints.filter((pt): pt is ChartPoint => !!pt && typeof pt === "object" && "x" in (pt as object)),
      };
    });
  return {
    chart_type,
    title: str(node.props, "title"),
    x_label: str(node.props, "x_label"),
    y_label: str(node.props, "y_label"),
    series,
    accessible_summary: str(node.props, "accessible_summary") ?? "",
  };
}

/**
 * `chart` primitive. A direct port of the legacy UniversalChart: same SVG
 * geometry (via the shared `chartGeometry` helper), same gridlines, line/area/
 * bar marks, point inspector, value list, data table and CSV export, and the
 * same "No data points supplied." empty state — so a payload with a missing or
 * malformed `series` renders a graceful placeholder card rather than nothing.
 */
function renderChart(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const data = createMemo(() => chartDataOf(node));
  const geometry = createMemo(() => chartGeometry(data()));
  const [selected, setSelected] = createSignal(0);
  const key = () => geometry().keys[Math.min(selected(), geometry().keys.length - 1)];
  const colors = ["var(--accent-bright)", "var(--cyan)", "var(--green)", "var(--muted)"];
  const line = (points: ChartPoint[]) =>
    points.map((point, i) => `${i ? "L" : "M"}${geometry().x(point.x)},${geometry().y(point.y!)}`).join(" ");
  const number = (value: number | null) =>
    value === null ? "Missing" : new Intl.NumberFormat(undefined, { maximumFractionDigits: 4 }).format(value);
  const title = () => data().title ?? "Chart";
  const exportData = () =>
    downloadCsv("chart.csv", [
      [data().x_label ?? "X", ...data().series.map((series) => series.name)],
      ...geometry().keys.map((x) => [x, ...data().series.map((series) => geometry().valueAt(series, x))]),
    ]);

  return (
    <figure class="semantic-chart" classList={{ "semantic-chart-compact": compact }}>
      <figcaption>
        <strong>{title()}</strong>
        <p>{data().accessible_summary}</p>
      </figcaption>
      <Show when={geometry().keys.length} fallback={<p>No data points supplied.</p>}>
        <svg viewBox="0 0 680 264" role="img" aria-label={data().accessible_summary}>
          <title>{title()}</title>
          <desc>{data().accessible_summary} Use the point selector or data table to inspect exact values.</desc>
          <For each={[0, 0.5, 1]}>
            {(fraction) => {
              const value = () => geometry().minY + fraction * (geometry().maxY - geometry().minY);
              return (
                <g>
                  <line x1="54" x2="626" y1={geometry().y(value())} y2={geometry().y(value())} stroke="var(--border)" />
                  <text x="46" y={geometry().y(value()) + 4} text-anchor="end">{number(value())}</text>
                </g>
              );
            }}
          </For>
          <For each={data().series}>
            {(series, index) => (
              <g>
                <Show when={data().chart_type !== "bar"}>
                  <For each={geometry().segments(series)}>
                    {(points) => (
                      <>
                        <Show when={data().chart_type === "area" && points.length > 1}>
                          <path
                            d={`${line(points)} L${geometry().x(points[points.length - 1].x)},${geometry().y(0)} L${geometry().x(points[0].x)},${geometry().y(0)} Z`}
                            fill={colors[index() % colors.length]}
                            opacity="var(--chart-area-opacity, 0.16)"
                          />
                        </Show>
                        <path
                          d={line(points)}
                          fill="none"
                          stroke={colors[index() % colors.length]}
                          stroke-width="2"
                          stroke-dasharray={index() ? `${index() + 2} 3` : undefined}
                        />
                        <For each={points}>
                          {(point) => (
                            <circle cx={geometry().x(point.x)} cy={geometry().y(point.y!)} r="3" fill={colors[index() % colors.length]}>
                              <title>{series.name}: {point.x}, {number(point.y)}</title>
                            </circle>
                          )}
                        </For>
                      </>
                    )}
                  </For>
                </Show>
                <Show when={data().chart_type === "bar"}>
                  <For each={series.points.filter((point) => point.y !== null)}>
                    {(point) => {
                      const width = () => Math.min(28, 420 / Math.max(1, geometry().keys.length * data().series.length));
                      return (
                        <rect
                          x={geometry().x(point.x) + (index() - data().series.length / 2) * width()}
                          y={Math.min(geometry().y(0), geometry().y(point.y!))}
                          width={width() - 1}
                          height={Math.abs(geometry().y(point.y!) - geometry().y(0))}
                          fill={colors[index() % colors.length]}
                        >
                          <title>{series.name}: {point.x}, {number(point.y)}</title>
                        </rect>
                      );
                    }}
                  </For>
                </Show>
              </g>
            )}
          </For>
          <text x="54" y="249">{axisLabel(geometry().keys[0])}<title>{String(geometry().keys[0])}</title></text>
          <text x="626" y="249" text-anchor="end">{axisLabel(geometry().keys.at(-1))}<title>{String(geometry().keys.at(-1))}</title></text>
        </svg>
        <p class="semantic-chart-axes">
          {data().x_label ?? "X"}
          {data().y_label ? ` · ${data().y_label}` : ""}
        </p>
        <label class="semantic-chart-selector">
          Inspect point: {String(key())}
          <input
            type="range"
            min="0"
            max={Math.max(0, geometry().keys.length - 1)}
            value={selected()}
            onInput={(event) => setSelected(Number(event.currentTarget.value))}
            aria-valuetext={String(key())}
          />
        </label>
        <dl class="semantic-chart-values">
          <For each={data().series}>
            {(series) => (
              <div>
                <dt>{series.name}</dt>
                <dd>{number(geometry().valueAt(series, key()))}</dd>
              </div>
            )}
          </For>
        </dl>
      </Show>
      <details>
        <summary>Data table</summary>
        <div class="semantic-table-wrap">
          <table class="semantic-table">
            <thead>
              <tr>
                <th scope="col">{data().x_label ?? "X"}</th>
                <For each={data().series}>{(series) => <th scope="col">{series.name}</th>}</For>
              </tr>
            </thead>
            <tbody>
              <For each={geometry().keys}>
                {(x) => (
                  <tr>
                    <th scope="row">{String(x)}</th>
                    <For each={data().series}>{(series) => <td>{number(geometry().valueAt(series, x))}</td>}</For>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </details>
      <button type="button" onClick={exportData}>Download CSV</button>
    </figure>
  );
}

/**
 * `ui_preview` primitive. A direct port of the legacy UIPreviewCard: the same
 * preview/source toggle, reload, dock and canvas actions, and the same
 * quarantine pipeline — inline HTML or a workspace file read through
 * `api.readFile`, then always passed through `artifactPreviewHtml` /
 * `sandboxedSrcdoc` before it reaches the sandboxed iframe's srcdoc. Those
 * sandboxing calls are unchanged, as is the `sandbox` attribute default.
 */
function renderUiPreview(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [viewMode, setViewMode] = createSignal<"preview" | "source">("preview");
  const inlineHtml = () => str(node.props, "html");
  const [htmlContent, setHtmlContent] = createSignal<string>(inlineHtml() ?? "");
  const [loading, setLoading] = createSignal(!inlineHtml());
  const [error, setError] = createSignal<string | null>(null);
  const [previewHtml, setPreviewHtml] = createSignal("");
  let request = 0;
  onCleanup(() => {
    request += 1;
  });
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);

  const path = () => str(node.props, "artifact_path") ?? "";

  const loadContent = async () => {
    const generation = ++request;
    const p = path();
    const inline = inlineHtml();
    setLoading(true);
    setError(null);
    try {
      const html = inline ?? (p ? (await api.readFile(p)).content : undefined);
      if (html === undefined) throw new Error("Preview file is unavailable. Reload to try again.");
      const prepared = p
        ? await artifactPreviewHtml(p, html, str(node.props, "connect_src"))
        : sandboxedSrcdoc(html, str(node.props, "connect_src") ?? "'none'");
      if (generation !== request) return;
      setHtmlContent(html);
      setPreviewHtml(prepared);
    } catch (e) {
      if (generation === request) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (generation === request) setLoading(false);
    }
  };

  createEffect(() => {
    void path();
    void loadContent();
  });

  const reloadPreview = () => {
    void loadContent();
  };

  const previewPayload = () => ({
    id: str(node.props, "preview_id") ?? path(),
    title: str(node.props, "title") ?? "Component Preview",
    artifactPath: path(),
    html: inlineHtml() || (htmlContent().trim().length > 0 ? htmlContent() : undefined),
    previewId: str(node.props, "preview_id"),
    sandbox: str(node.props, "sandbox"),
    connectSrc: str(node.props, "connect_src"),
    timestamp: Date.now(),
  });

  const openInCanvas = () => openArtifactCanvas(previewPayload());
  const openInDock = () => openComponentPreview(previewPayload());

  const handleCopySource = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(htmlContent());
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
      setCopyFailed(true);
    }
  };

  const title = () => str(node.props, "title") || "Interactive Component Preview";

  return (
    <div class="canvas-card ui-preview-card" classList={{ "ui-preview-card-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">UI Preview</span>
          <strong style="font-size: var(--fs-control); color: var(--text);">{title()}</strong>

        </div>
        <div class="card-actions" style="display: flex; gap: 6px; align-items: center;">
          <button
            class="pill-action-btn"
            classList={{ active: viewMode() === "source" }}
            onClick={() => setViewMode((m) => (m === "preview" ? "source" : "preview"))}
            title="Toggle between live preview and source markup"
          >
            {viewMode() === "preview" ? "Source" : "Preview"}
          </button>
          <button class="pill-action-btn" onClick={reloadPreview} title="Reload live preview">
            Reload
          </button>
          <button class="pill-action-btn" onClick={openInDock} title="Open in dock panel">
            Dock
          </button>
          <button class="open-canvas-btn" onClick={openInCanvas} title="Open immersive canvas preview">
            <Icon name="preview" size={14} /> Open Canvas
          </button>
        </div>
      </div>

      <Show when={loading()}>
        <div style="padding: 24px; text-align: center; color: var(--muted); font-size: var(--fs-meta);">
          Loading sandboxed preview artifact…
        </div>
      </Show>

      <Show when={error()}>
        <div style="padding: 16px; color: var(--red); background: color-mix(in srgb, var(--red) 8%, transparent); font-size: var(--fs-meta);">
          Failed to load preview: {error()}
        </div>
      </Show>

      <Show when={!loading() && !error()}>
        <Show when={viewMode() === "preview"}>
          <div
            class="ui-preview-stage"
            style="position: relative; width: 100%; height: 390px; background: var(--surface); overflow: hidden; border-top: 1px solid var(--border-soft);"
          >
            <iframe
              srcdoc={previewHtml()}
              title={title()}
              sandbox={str(node.props, "sandbox") ?? "allow-scripts"}
              style="width: 100%; height: 100%; border: 0; display: block;"
            />
          </div>
        </Show>

        <Show when={viewMode() === "source"}>
          <div style="padding: 10px 14px; background: var(--bg); border-top: 1px solid var(--border-soft);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
              <span style="font-size: var(--fs-caption); color: var(--faint); font-family: var(--mono);">
                Quarantined: {path()}
              </span>
              <button type="button" class="pill-action-btn" onClick={handleCopySource}>
                {copied() ? "✓ Copied" : copyFailed() ? "Copy failed" : "Copy Source"}
              </button>
            </div>
            <pre style="margin: 0; max-height: 280px; overflow: auto; font-family: var(--mono); font-size: max(12px, calc(12px * var(--code-scale, 1))); line-height: 1.55; color: var(--text-soft); padding: 8px; border-radius: 6px; background: var(--surface);">
              <code>{htmlContent()}</code>
            </pre>
          </div>
        </Show>
      </Show>
    </div>
  );
}

/**
 * `media` primitive, covering the former inline `link.preview` / `media.image`
 * / `media.video` / `media.audio` registry cases. `kind` selects the element.
 *
 * The privacy/security gating is deliberately evaluated HERE rather than in the
 * adapter, and is byte-for-byte the same predicate the inline cases used:
 * `uiPreferences.externalMedia` must be on AND the URL must pass
 * `safeUrl(url, true)` (media mode, which additionally allows `data:image/`)
 * before any `src` is emitted; `link.preview`'s href passes through
 * `safeUrl(url)` in non-media mode and is left undefined when it fails, while
 * its thumbnail is gated exactly like an image. Keeping it in the renderer also
 * preserves reactivity to the preference being toggled at runtime.
 */
function renderMedia(node: AdaptiveRenderNode, _surface: RenderSurface) {
  const kind = (str(node.props, "kind") ?? "link").toLowerCase();
  const source = () => str(node.props, "source") ?? "";
  const alt = () => str(node.props, "alt") ?? "";
  const externalAllowed = () => uiPreferences.externalMedia && source().length > 0 && safeUrl(source(), true);

  if (kind === "image") {
    return (
      <Show when={externalAllowed()}>
        <figure class="rich-media">
          <img src={source()} alt={alt()} />
          <Show when={alt()}>
            <figcaption>{alt()}</figcaption>
          </Show>
        </figure>
      </Show>
    );
  }
  if (kind === "video") {
    return (
      <Show when={externalAllowed()}>
        <video
          class="rich-video"
          src={source()}
          controls
          preload="metadata"
          autoplay={uiPreferences.autoplayMedia}
          aria-label={str(node.props, "alt") ?? "Video"}
        />
      </Show>
    );
  }
  if (kind === "audio") {
    return (
      <Show when={externalAllowed()}>
        <audio class="rich-audio" src={source()} controls preload="metadata" aria-label={str(node.props, "alt") ?? "Audio"} />
      </Show>
    );
  }

  const url = () => str(node.props, "url") ?? "";
  const imageUrl = () => str(node.props, "image_url");
  return (
    <a class="rich-link-card" href={safeUrl(url()) ? url() : undefined} target="_blank" rel="noreferrer noopener">
      <Show when={uiPreferences.externalMedia && imageUrl() !== undefined && safeUrl(imageUrl()!, true)}>
        <img src={imageUrl()!} alt="" loading="lazy" />
      </Show>
      <span>
        <strong>{str(node.props, "title") ?? url()}</strong>
        <small>{str(node.props, "description") ?? str(node.props, "site_name") ?? url()}</small>
      </span>
    </a>
  );
}

/** Same safe default the existing AdaptiveTreeView uses for unmapped primitives. */
/**
 * Lossless key/value display for "universal semantic shape" payloads.
 *
 * Direct port of `UniversalCard` / its `PresentationValue` sub-component
 * (presentation/UniversalCard.tsx) so the DOM/CSS is byte-identical: same
 * `canvas-card universal-card` section, same `universal-card-row` rows,
 * `universal-card-nested` nested `<dl>`s, `universal-card-list` arrays and
 * `universal-card-empty` placeholder. Ported rather than imported so the
 * legacy component can be deleted in the cleanup pass.
 */
function universalFieldLabel(key: string): string {
  return key.replace(/[_-]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function UniversalValue(props: { value: unknown }) {
  const value = () => props.value;
  return (
    <Show when={value() !== null && value() !== undefined && value() !== ""} fallback={<span class="universal-card-empty">Unavailable</span>}>
      <Show when={Array.isArray(value())} fallback={
        <Show when={typeof value() === "object"} fallback={
          <Show when={typeof value() === "string" && safeUrl(value() as string)} fallback={<>{typeof value() === "boolean" ? (value() ? "Yes" : "No") : String(value())}</>}>
            <a href={value() as string} target="_blank" rel="noreferrer noopener">{value() as string}</a>
          </Show>
        }>
          <dl class="universal-card-nested">
            <For each={Object.entries(value() as Record<string, unknown>)}>{([key, nested]) => (
              <div><dt>{universalFieldLabel(key)}</dt><dd><UniversalValue value={nested} /></dd></div>
            )}</For>
          </dl>
        </Show>
      }>
        <ul class="universal-card-list">
          <For each={value() as unknown[]}>{(item) => <li><UniversalValue value={item} /></li>}</For>
        </ul>
      </Show>
    </Show>
  );
}

function renderUniversalCard(node: AdaptiveRenderNode, _surface: RenderSurface) {
  const kind = str(node.props, "kind") ?? "Result";
  const title = str(node.props, "title");
  const summary = str(node.props, "summary");
  const rawEntries = node.props.entries;
  // A field that only restates what kind of card this is ("kind: entity" on
  // an entity card) is detail, shown with technical details on.
  const restatesKind = ([key, value]: [string, unknown]) =>
    ["kind", "type", "semantic_type"].includes(key.toLowerCase()) && String(value).toLowerCase() === kind.toLowerCase();
  const entries = (): [string, unknown][] => (Array.isArray(rawEntries)
    ? (rawEntries as unknown[]).filter(
        (e): e is [string, unknown] => Array.isArray(e) && e.length >= 1 && typeof e[0] === "string",
      )
    : []).filter((entry) => technicalDetails() || !restatesKind(entry));
  return (
    <section class="canvas-card universal-card" aria-label={`${kind} result`}>
      <Show when={title}><h3>{title}</h3></Show>
      <Show when={summary}><p>{summary}</p></Show>
      <Show when={entries().length > 0}>
        <dl>
          <For each={entries()}>{([key, value]) => (
            <div class="universal-card-row">
              <dt>{universalFieldLabel(key)}</dt>
              <dd><UniversalValue value={value} /></dd>
            </div>
          )}</For>
        </dl>
      </Show>
    </section>
  );
}

function renderFallback(node: AdaptiveRenderNode, _surface: RenderSurface) {
  const text = str(node.props, "text");
  const label = str(node.props, "label");
  const title = str(node.props, "title");
  const children = Array.isArray(node.children) ? node.children : [];
  const details = Object.entries(node.props).filter(([key]) => !["title", "summary", "text", "label", "semantic_type"].includes(key));
  return (
    <div class={`adaptive-node adaptive-${node.primitive.toLowerCase()}`}>
      {title && <h4 class="adaptive-node-title">{title}</h4>}
      {label && <strong class="adaptive-node-label">{label}</strong>}
      {text && <span class="adaptive-node-text">{text}</span>}
      {str(node.props, "summary") && <p>{str(node.props, "summary")}</p>}
      <Show when={details.length > 0}><dl class="universal-card-nested"><For each={details}>{([key, value]) => <div><dt>{universalFieldLabel(key)}</dt><dd><UniversalValue value={value} /></dd></div>}</For></dl></Show>
      {children.map((child) => renderNode(child, _surface))}
    </div>
  );
}

export default function GenericSpecRenderer(props: GenericSpecRendererProps) {
  const surface = () => props.surface ?? "full";
  return <>{renderNode(props.node, surface())}</>;
}

// ---------------------------------------------------------------------------
// Adapters: raw legacy payload -> AdaptiveRenderNode.
//
// These stand in for a server-side `vak_presentation::compile()` call. They
// intentionally reuse the same tolerant parsing rules as the legacy
// `normalizeTimeline`/inline metric handler in PresentationRenderer.tsx
// (try several known field names, degrade to an empty/placeholder value,
// never throw on missing or malformed input) so behavior does not regress.
// ---------------------------------------------------------------------------

interface TimelineOption {
  label: string;
  detail?: string;
  facts: string[];
}

interface RawTimelineItem {
  label: string;
  detail?: string;
  status?: string;
  time?: string;
  options?: unknown;
}

const TIMELINE_ARRAY_FIELDS = ["items", "steps", "milestones", "slots", "agenda", "tasks", "options", "choices", "questions", "qa", "entries"] as const;

function timelineArrayField(record: Record<string, unknown>): string | undefined {
  return TIMELINE_ARRAY_FIELDS.find((key) => Array.isArray(record[key]));
}

function normalizeTimelineItems(data: unknown): RawTimelineItem[] {
  if (!data || typeof data !== "object") return [];
  const record = data as Record<string, unknown>;
  let rawItems: unknown[] = [];
  let matchedArrayField = false;
  const arrayField = timelineArrayField(record);
  if (arrayField) { rawItems = record[arrayField] as unknown[]; matchedArrayField = true; }
  if (!matchedArrayField && Array.isArray(data)) {
    rawItems = data as unknown[];
    matchedArrayField = true;
  }
  if (!matchedArrayField) {
    const entries = Object.entries(record).filter(([k]) => !["title", "semantic_type", "label", "summary"].includes(k));
    if (entries.length > 0) {
      rawItems = entries.map(([k, v]) => ({
        label: k.replace(/_/g, " "),
        detail: typeof v === "object" ? JSON.stringify(v) : String(v ?? ""),
      }));
    }
  }
  return rawItems.map((it) => {
    if (typeof it === "string" || typeof it === "number") return { label: String(it) };
    if (it && typeof it === "object") {
      const o = it as Record<string, unknown>;
      const label = String(o.label ?? o.title ?? o.name ?? o.question ?? o.task ?? o.text ?? o.choice ?? o.activity ?? "Item");
      const detail = o.detail ?? o.description ?? o.answer ?? o.notes ?? o.time ?? o.snippet ?? (o.reason ? String(o.reason) : undefined);
      const status = o.status ?? (typeof o.done === "boolean" ? (o.done ? "complete" : "pending") : undefined);
      const time = o.time ?? o.when ?? o.window ?? o.duration;
      return {
        label,
        detail: detail != null && detail !== time ? String(detail) : undefined,
        status: status != null ? String(status) : undefined,
        time: time != null ? String(time) : undefined,
        options: o.options,
      };
    }
    return { label: "Item" };
  });
}

/** Build a `timeline` primitive node from a raw, possibly malformed payload. */
export function buildTimelineSpec(data: unknown, defaultTitle = "Plan", kicker = "Plan"): AdaptiveRenderNode {
  const record = data && typeof data === "object" && !Array.isArray(data) ? (data as Record<string, unknown>) : {};
  const title = typeof record.title === "string" ? record.title : typeof record.label === "string" ? record.label : typeof record.name === "string" ? record.name : defaultTitle;
  const items = normalizeTimelineItems(data);
  return {
    primitive: "timeline",
    props: { title, kicker, variant: timelineArrayField(record) === "options" ? "options" : "sequence" },
    children: items.map((item) => ({
      primitive: "section",
      props: { label: item.label, detail: item.detail ?? "", status: item.status ?? "", time: item.time ?? "", options: item.options ?? [] },
      children: [],
    })),
  };
}

function tableNode(title: string, columns: SpecColumn[], rows: unknown[]): AdaptiveRenderNode {
  return {
    primitive: "table",
    props: { title, columns },
    children: rows.map((row) => ({
      primitive: "row",
      props: row && typeof row === "object" ? (row as Record<string, unknown>) : { value: String(row ?? "") },
      children: [],
    })),
  };
}

/**
 * Build a `table` primitive node from a raw, possibly malformed payload.
 * Field-guessing is a direct port of `normalizeDataGrid` in
 * PresentationRenderer.tsx (explicit columns/rows, pros/cons, left/right
 * comparison, array-of-objects, then flat key/value fallback).
 */
export function buildTableSpec(data: unknown, defaultTitle = "Dataset"): AdaptiveRenderNode {
  if (!data || typeof data !== "object") {
    return tableNode(defaultTitle, [], []);
  }
  const d = data as any;
  const title = String(d.title ?? d.label ?? d.name ?? defaultTitle);

  if (Array.isArray(d.columns) && Array.isArray(d.rows)) {
    const columns: SpecColumn[] = d.columns.map((c: any) =>
      c && typeof c === "object"
        ? { key: String(c.key ?? c.name ?? c.label), label: String(c.label ?? c.name ?? c.key), isNumeric: Boolean(c.isNumeric || c.is_numeric) }
        : { key: String(c), label: String(c) },
    );
    const rows = d.rows.map((row: unknown) => {
      if (!Array.isArray(row)) return row;
      return Object.fromEntries(columns.map((column, index) => [column.key, row[index] ?? ""]));
    });
    return tableNode(title, columns, rows);
  }

  if (Array.isArray(d.pros) || Array.isArray(d.cons)) {
    const rows = [
      ...(d.pros || []).map((p: any) => ({ type: "Pro", point: typeof p === "string" ? p : (p?.text ?? p?.point ?? JSON.stringify(p)) })),
      ...(d.cons || []).map((c: any) => ({ type: "Con", point: typeof c === "string" ? c : (c?.text ?? c?.point ?? JSON.stringify(c)) })),
    ];
    return tableNode(title, [
      { key: "type", label: "Type" },
      { key: "point", label: "Point" },
    ], rows);
  }

  if (d.left != null || d.right != null) {
    const leftLabel = String(d.left_label ?? d.option_a ?? "Option A");
    const rightLabel = String(d.right_label ?? d.option_b ?? "Option B");
    const rows: Record<string, unknown>[] = [];
    if (typeof d.left === "object" && typeof d.right === "object") {
      const keys = Array.from(new Set([...Object.keys(d.left || {}), ...Object.keys(d.right || {})]));
      for (const k of keys) {
        rows.push({
          aspect: k.replace(/_/g, " "),
          left: typeof d.left[k] === "object" ? JSON.stringify(d.left[k]) : String(d.left[k] ?? "—"),
          right: typeof d.right[k] === "object" ? JSON.stringify(d.right[k]) : String(d.right[k] ?? "—"),
        });
      }
    } else {
      rows.push({ aspect: "Value", left: String(d.left ?? "—"), right: String(d.right ?? "—") });
    }
    return tableNode(title, [
      { key: "aspect", label: "Aspect" },
      { key: "left", label: leftLabel },
      { key: "right", label: rightLabel },
    ], rows);
  }

  const rawRows: any[] = Array.isArray(d.rows) ? d.rows : Array.isArray(data) ? (data as any[]) : Array.isArray(d.items) ? d.items : [];
  if (rawRows.length > 0 && typeof rawRows[0] === "object") {
    const keys = Array.from(new Set(rawRows.flatMap((r) => Object.keys(r || {}))));
    const columns: SpecColumn[] = keys.map((k) => ({
      key: k,
      label: k.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
      isNumeric: rawRows.some((r) => typeof r?.[k] === "number"),
    }));
    return tableNode(title, columns, rawRows);
  }

  const entries = Object.entries(d).filter(([k]) => !["title", "semantic_type", "label"].includes(k));
  if (entries.length > 0) {
    const rows = entries.map(([k, v]) => ({
      metric: k.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
      value: typeof v === "object" ? JSON.stringify(v) : v,
    }));
    return tableNode(title, [
      { key: "metric", label: "Metric / Item" },
      { key: "value", label: "Value", isNumeric: entries.some(([, v]) => typeof v === "number") },
    ], rows);
  }

  return tableNode(title, [], []);
}

export function buildOptionsTableSpec(data: unknown, defaultTitle = "Options"): AdaptiveRenderNode {
  const node = buildTableSpec(data, defaultTitle);
  return { ...node, props: { ...node.props, variant: "options" } };
}

/**
 * Build a `recipe` primitive node from a raw, possibly malformed payload.
 * Field-guessing is a direct port of `normalizeRecipe` in
 * PresentationRenderer.tsx (ingredients/items; steps/instructions/
 * directions/method; timer_seconds/timer_minutes/duration_minutes).
 */
export function buildRecipeSpec(data: unknown): AdaptiveRenderNode {
  const d = data && typeof data === "object" ? (data as any) : null;
  if (!d) {
    return {
      primitive: "recipe",
      props: { title: "Recipe" },
      children: [
        { primitive: "ingredient_list", props: {}, children: [] },
        { primitive: "step_list", props: {}, children: [] },
      ],
    };
  }

  const rawIngredients: any[] = Array.isArray(d.ingredients) ? d.ingredients : Array.isArray(d.items) ? d.items : [];
  const ingredients: AdaptiveRenderNode[] = rawIngredients.map((item: any) => {
    if (typeof item === "string") {
      return { primitive: "ingredient", props: { raw: true, text: item }, children: [] };
    }
    if (item && typeof item === "object") {
      const name = String(item.name ?? item.item ?? item.ingredient ?? item.label ?? "");
      const amount = typeof item.amount === "number" ? item.amount : typeof item.quantity === "number" ? item.quantity : undefined;
      const unit = item.unit ?? item.measurement ?? (typeof item.quantity === "string" ? item.quantity : undefined);
      const props: Record<string, unknown> = { name };
      if (amount !== undefined) props.amount = amount;
      if (unit != null) props.unit = String(unit);
      return { primitive: "ingredient", props, children: [] };
    }
    return { primitive: "ingredient", props: { raw: true, text: String(item) }, children: [] };
  });

  const rawSteps: any[] = Array.isArray(d.steps)
    ? d.steps
    : Array.isArray(d.instructions)
    ? d.instructions
    : Array.isArray(d.directions)
    ? d.directions
    : Array.isArray(d.method)
    ? d.method
    : [];
  const steps: AdaptiveRenderNode[] = rawSteps.map((step: any) => {
    if (typeof step === "string") {
      return { primitive: "step", props: { text: step }, children: [] };
    }
    if (step && typeof step === "object") {
      const text = String(step.text ?? step.step ?? step.instruction ?? step.description ?? step.action ?? "");
      let timerSeconds = typeof step.timer_seconds === "number" ? step.timer_seconds : undefined;
      if (timerSeconds === undefined && typeof step.timer_minutes === "number") timerSeconds = step.timer_minutes * 60;
      if (timerSeconds === undefined && typeof step.duration_minutes === "number") timerSeconds = step.duration_minutes * 60;
      const props: Record<string, unknown> = { text };
      if (timerSeconds !== undefined) props.timer_seconds = timerSeconds;
      return { primitive: "step", props, children: [] };
    }
    return { primitive: "step", props: { text: String(step) }, children: [] };
  });

  const props: Record<string, unknown> = { title: String(d.title ?? d.name ?? "Recipe") };
  const servings = typeof d.servings === "number" ? d.servings : typeof d.yield === "number" ? d.yield : undefined;
  if (servings !== undefined) props.servings = servings;
  const prep = typeof d.prep_time_minutes === "number" ? d.prep_time_minutes : typeof d.prep_time === "number" ? d.prep_time : undefined;
  if (prep !== undefined) props.prep_time_minutes = prep;
  const cook = typeof d.cook_time_minutes === "number" ? d.cook_time_minutes : typeof d.cook_time === "number" ? d.cook_time : undefined;
  if (cook !== undefined) props.cook_time_minutes = cook;

  return {
    primitive: "recipe",
    props,
    children: [
      { primitive: "ingredient_list", props: {}, children: ingredients },
      { primitive: "step_list", props: {}, children: steps },
    ],
  };
}

/**
 * Build a `research` primitive node from a raw, possibly malformed payload.
 * Field-guessing is a direct port of `normalizeResearch` in
 * PresentationRenderer.tsx (sources/references/citations; takeaways/
 * findings/points/items).
 */
export function buildResearchSpec(data: unknown): AdaptiveRenderNode {
  const d = data && typeof data === "object" ? (data as any) : null;
  if (!d) {
    return {
      primitive: "research",
      props: {},
      children: [
        { primitive: "takeaway_list", props: {}, children: [] },
        { primitive: "source_list", props: {}, children: [] },
      ],
    };
  }

  const rawSources: any[] = Array.isArray(d.sources)
    ? d.sources
    : Array.isArray(d.references)
    ? d.references
    : Array.isArray(d.citations)
    ? d.citations
    : [];
  const sources: AdaptiveRenderNode[] = rawSources.map((s: any) => {
    if (typeof s === "string") return { primitive: "source", props: { title: s, url: s }, children: [] };
    const props: Record<string, unknown> = {
      title: String(s?.title ?? s?.name ?? s?.url ?? "Source"),
      url: String(s?.url ?? ""),
    };
    if (s?.snippet != null) props.snippet = String(s.snippet);
    if (s?.source_name != null) props.source_name = String(s.source_name);
    if (s?.published_at != null) props.published_at = String(s.published_at);
    return { primitive: "source", props, children: [] };
  });

  const rawTakeaways: any[] = Array.isArray(d.takeaways)
    ? d.takeaways
    : Array.isArray(d.findings)
    ? d.findings
    : Array.isArray(d.points)
    ? d.points
    : Array.isArray(d.items)
    ? d.items
    : [];
  const takeaways: AdaptiveRenderNode[] = rawTakeaways.map((t: any) => {
    if (typeof t === "string") return { primitive: "takeaway", props: { text: t }, children: [] };
    if (t && typeof t === "object") {
      const props: Record<string, unknown> = { text: String(t.text ?? t.point ?? t.claim ?? t.label ?? "") };
      if (Array.isArray(t.citation_indices)) props.citation_indices = t.citation_indices;
      return { primitive: "takeaway", props, children: [] };
    }
    return { primitive: "takeaway", props: { text: String(t) }, children: [] };
  });

  const props: Record<string, unknown> = {};
  if (d.title != null) props.title = String(d.title);

  return {
    primitive: "research",
    props,
    children: [
      { primitive: "takeaway_list", props: {}, children: takeaways },
      { primitive: "source_list", props: {}, children: sources },
    ],
  };
}

/** Build a `metric` (or `metric_grid`) primitive node from a raw, possibly malformed payload. */
export function buildMetricSpec(data: unknown): AdaptiveRenderNode {
  const record = data && typeof data === "object" && !Array.isArray(data) ? (data as Record<string, unknown>) : {};
  const hasSingleValue = typeof record.value === "string" || typeof record.value === "number";
  if (hasSingleValue) {
    const label = typeof record.label === "string" ? record.label : "Metric";
    const value = typeof record.value === "string" || typeof record.value === "number" ? record.value : undefined;
    const unit = typeof record.unit === "string" ? record.unit : undefined;
    return {
      primitive: "metric",
      props: { label, value: value ?? "—", unit: unit ?? "" },
      children: [],
    };
  }
  const title = (typeof record.title === "string" && record.title) || (typeof record.label === "string" && record.label) || (typeof record.location === "string" && record.location) || "";
  const entries = Object.entries(record).filter(([k]) => k !== "title" && k !== "semantic_type" && !(k === "label" && title === record.label) && !(k === "location" && title === record.location));
  if (entries.length > 0) {
    return {
      primitive: "metric_grid",
      props: { title },
      children: entries.map(([key, val]) => ({
        primitive: "metric",
        props: { label: key.replace(/_/g, " "), value: typeof val === "string" || typeof val === "number" ? val : JSON.stringify(val) },
        children: [],
      })),
    };
  }
  // No usable fields at all: degrade to the same placeholder the legacy
  // handler falls back to, rather than rendering nothing.
  return { primitive: "metric", props: { label: "Metric", value: "—", unit: "" }, children: [] };
}

/**
 * Build a `diff` primitive node from a raw, possibly malformed payload.
 * Mirrors DiffInspector's own `files()` memo: an explicit non-empty `files`
 * array wins; otherwise a single synthetic file is derived from a raw patch
 * string, with +/- counts computed the same way and the same
 * "changes.diff" default filename.
 */
export function buildDiffSpec(data: unknown, rawDiff?: string, filename?: string): AdaptiveRenderNode {
  const d = data && typeof data === "object" ? (data as any) : null;
  const rawFiles: any[] = d && Array.isArray(d.files) ? d.files : [];
  const files = rawFiles
    .filter((f) => f && typeof f === "object")
    .map((f) => ({
      primitive: "diff_file",
      props: {
        filename: String(f.filename ?? f.file ?? f.path ?? f.name ?? "changes.diff"),
        additions: typeof f.additions === "number" ? f.additions : 0,
        deletions: typeof f.deletions === "number" ? f.deletions : 0,
        hunks: typeof f.hunks === "string" ? f.hunks : typeof f.patch === "string" ? f.patch : typeof f.diff === "string" ? f.diff : "",
      },
      children: [],
    }));

  if (files.length > 0) {
    return { primitive: "diff", props: {}, children: files };
  }

  // Same fallback the legacy component used for a raw patch string. The extra
  // payload keys checked here (diff/patch/raw_diff/content) are a strict
  // improvement: the old component rendered an empty card for those shapes.
  const raw =
    rawDiff ??
    (d && (typeof d.diff === "string" ? d.diff : typeof d.patch === "string" ? d.patch : typeof d.raw_diff === "string" ? d.raw_diff : typeof d.content === "string" ? d.content : undefined)) ??
    "";
  const adds = (raw.match(/^\+[^+]/gm) || []).length;
  const dels = (raw.match(/^-[^-]/gm) || []).length;
  const name = filename ?? (d && (d.filename ?? d.file ?? d.path)) ?? "changes.diff";
  return {
    primitive: "diff",
    props: {},
    children: [
      { primitive: "diff_file", props: { filename: String(name), additions: adds, deletions: dels, hunks: raw }, children: [] },
    ],
  };
}

/**
 * Build a `terminal` primitive node from a raw, possibly malformed payload.
 * `exit_code`/`duration_ms` are only set when they really are numbers, so the
 * legacy "Exit status unavailable" / no-duration branches still trigger.
 */
export function buildTerminalSpec(data: unknown): AdaptiveRenderNode {
  const d = data && typeof data === "object" ? (data as any) : null;
  const props: Record<string, unknown> = {
    output: d && typeof d.output === "string" ? d.output : d && typeof d.stdout === "string" ? d.stdout : d && typeof d.text === "string" ? d.text : "",
  };
  if (d && typeof d.command === "string") props.command = d.command;
  else if (d && typeof d.cmd === "string") props.command = d.cmd;
  if (d && typeof d.exit_code === "number") props.exit_code = d.exit_code;
  else if (d && typeof d.exitCode === "number") props.exit_code = d.exitCode;
  if (d && typeof d.duration_ms === "number") props.duration_ms = d.duration_ms;
  return { primitive: "terminal", props, children: [] };
}

/**
 * Build a `chart` primitive node from a raw, possibly malformed payload.
 *
 * The legacy registry guarded every chart key with
 * `Array.isArray(data?.series) ? <UniversalChart .../> : <></>`, so a payload
 * whose `series` was missing, null or a non-array silently rendered nothing at
 * all. This adapter instead normalizes any shape to an empty series list, and
 * `renderChart` then shows the same "No data points supplied." placeholder the
 * component already had for an empty chart — never a blank node.
 *
 * `chartTypeOverride` preserves the `bar_chart` key's forced `chart_type: "bar"`.
 */
export function buildChartSpec(data: unknown, chartTypeOverride?: "line" | "area" | "bar"): AdaptiveRenderNode {
  const d = data && typeof data === "object" && !Array.isArray(data) ? (data as any) : null;
  const rawSeries: any[] = d && Array.isArray(d.series) ? d.series : [];
  const series: AdaptiveRenderNode[] = rawSeries
    .filter((s: any) => s && typeof s === "object")
    .map((s: any) => {
      const rawPoints: any[] = Array.isArray(s.points) ? s.points : [];
      const points: ChartPoint[] = rawPoints
        .map((pt: any): ChartPoint | null => {
          if (Array.isArray(pt) && pt.length >= 2) {
            return { x: typeof pt[0] === "number" ? pt[0] : String(pt[0] ?? ""), y: typeof pt[1] === "number" ? pt[1] : null };
          }
          if (pt && typeof pt === "object") {
            const x = pt.x ?? pt.label ?? pt.key;
            if (x === undefined || x === null) return null;
            return { x: typeof x === "number" ? x : String(x), y: typeof pt.y === "number" ? pt.y : typeof pt.value === "number" ? pt.value : null };
          }
          return null;
        })
        .filter((pt): pt is ChartPoint => pt !== null);
      return { primitive: "series", props: { name: String(s.name ?? s.label ?? "Series"), points }, children: [] };
    });

  const rawType = d && typeof d.chart_type === "string" ? d.chart_type : undefined;
  const props: Record<string, unknown> = {
    chart_type: chartTypeOverride ?? (rawType === "bar" || rawType === "area" || rawType === "line" ? rawType : "line"),
    accessible_summary: d && typeof d.accessible_summary === "string" ? d.accessible_summary : "",
  };
  if (d && typeof d.title === "string") props.title = d.title;
  if (d && typeof d.x_label === "string") props.x_label = d.x_label;
  if (d && typeof d.y_label === "string") props.y_label = d.y_label;

  return { primitive: "chart", props, children: series };
}

/**
 * Build a `media` primitive node from a raw, possibly malformed payload.
 *
 * Only string URLs are carried through (the legacy inline cases each tested
 * `typeof ... === "string"` before rendering), so a non-string `source` /
 * `image_url` still yields no `src` at all. No URL validation happens here —
 * `safeUrl` gating stays in `renderMedia` so it re-runs with the live
 * `uiPreferences` values.
 */
export function buildMediaSpec(data: unknown, kind: "link" | "image" | "video" | "audio"): AdaptiveRenderNode {
  const d = data && typeof data === "object" && !Array.isArray(data) ? (data as any) : null;
  const props: Record<string, unknown> = { kind };
  if (kind === "link") {
    if (d && typeof d.url === "string") props.url = d.url;
    if (d && typeof d.image_url === "string") props.image_url = d.image_url;
    for (const key of ["title", "description", "site_name"]) {
      if (d && (typeof d[key] === "string" || typeof d[key] === "number")) props[key] = d[key];
    }
  } else {
    if (d && typeof d.source === "string") props.source = d.source;
    if (d && (typeof d.alt === "string" || typeof d.alt === "number")) props.alt = d.alt;
  }
  return { primitive: "media", props, children: [] };
}

/**
 * Build a `ui_preview` primitive node from a raw, possibly malformed payload.
 * Only string fields are carried over, so a non-string `html`/`artifact_path`
 * degrades to the renderer's normal "Preview file is unavailable." error
 * branch instead of being fed to the sandbox pipeline.
 */
export function buildUiPreviewSpec(data: unknown): AdaptiveRenderNode {
  const d = data && typeof data === "object" && !Array.isArray(data) ? (data as any) : null;
  const props: Record<string, unknown> = {};
  for (const key of ["status", "preview_id", "title", "artifact_path", "sandbox", "connect_src", "html"]) {
    if (d && typeof d[key] === "string") props[key] = d[key];
  }
  return { primitive: "ui_preview", props, children: [] };
}

/**
 * Build a `test_matrix` primitive node from a raw, possibly malformed payload.
 * Counts stay optional so the renderer falls back to deriving them from the
 * test list, exactly as the legacy component did. A missing or non-array
 * `tests` field yields an empty list instead of throwing (the old component
 * called `.filter` on it unconditionally).
 */
export function buildTestMatrixSpec(data: unknown): AdaptiveRenderNode {
  const d = data && typeof data === "object" ? (data as any) : null;
  const rawTests: any[] = d && Array.isArray(d.tests) ? d.tests : d && Array.isArray(d.cases) ? d.cases : d && Array.isArray(d.results) ? d.results : [];
  const tests: AdaptiveRenderNode[] = rawTests.map((t: any) => {
    if (typeof t === "string") return { primitive: "test_case", props: { name: t, status: "" }, children: [] };
    const o = t && typeof t === "object" ? t : {};
    // Status is deliberately NOT defaulted to "passed": the legacy component
    // treated an absent/garbage status as neither passed nor failed, and the
    // pass/fail counters derived from the list must keep matching.
    const rawStatus = o.status ?? o.result;
    const props: Record<string, unknown> = {
      name: String(o.name ?? o.test ?? o.title ?? ""),
      status: typeof rawStatus === "string" ? rawStatus : "",
    };
    if (typeof o.duration_ms === "number") props.duration_ms = o.duration_ms;
    if (o.message != null) props.message = String(o.message);
    if (o.traceback != null) props.traceback = String(o.traceback);
    return { primitive: "test_case", props, children: [] };
  });

  const props: Record<string, unknown> = {};
  if (d && (typeof d.suite_name === "string" || typeof d.suite_name === "number")) props.suite_name = d.suite_name;
  for (const key of ["total", "passed", "failed", "skipped", "duration_ms"]) {
    if (d && typeof d[key] === "number") props[key] = d[key];
  }
  return { primitive: "test_matrix", props, children: tests };
}

/**
 * Build a `universal_card` primitive node from a raw payload. Mirrors
 * `UniversalCard`'s own entry selection (title/summary/semantic_type are
 * lifted out of the key/value list and rendered as the heading) so the
 * output is identical for every payload, including malformed ones.
 */
export function buildUniversalCardSpec(data: unknown, kind: string): AdaptiveRenderNode {
  const record = data && typeof data === "object" ? (data as Record<string, unknown>) : {};
  const title = record.title ? String(record.title) : undefined;
  const summary = record.summary ? String(record.summary) : undefined;
  const entries = Object.entries(record).filter(([key]) => !["semantic_type", "title", "summary"].includes(key));
  return {
    primitive: "universal_card",
    props: { kind, ...(title ? { title } : {}), ...(summary ? { summary } : {}), entries },
    children: [],
  };
}
