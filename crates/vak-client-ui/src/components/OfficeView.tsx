import { createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import * as api from "../api";
import Redline from "./OfficeRedline";
import { cellAddress, columnName, parseCell } from "../officeCells";

// The Canvas views of an Office file (docs/design/72, P4, U1–U4): a
// Document, Workbook or Deck view of the reader's own projection, and the
// package's Structure. Everything drawn here is text with anchors and
// labels from the server's worker; the browser never unzips a package or
// mounts document markup. Selecting a unit offers to ask the Agent about
// that exact place, by its anchor.

type Tab = "content" | "structure";

const MAIN_TAB: Record<api.OfficeProjection["vocabulary"], string> = {
  word: "Document",
  excel: "Workbook",
  power_point: "Deck",
  visio: "Drawing",
};

function unitId(anchor: string): string {
  return `office-unit-${anchor.replace(/[^A-Za-z0-9_-]/g, "-")}`;
}

function Labels(props: { labels: string[] }) {
  return (
    <Show when={props.labels.length > 0}>
      <span class="office-unit-labels">
        <For each={props.labels}>{(label) => <span class="office-label">{label}</span>}</For>
      </span>
    </Show>
  );
}

export default function OfficeView(props: {
  source: api.OfficeSource;
  fileName: string;
  /** Called with the anchor a person selects, or null. */
  onSelect?: (anchor: string | null) => void;
  /** A cited place to open at and select. */
  focus?: string;
}) {
  const [meta, setMeta] = createSignal<api.OfficeProjection | null>(null);
  const [units, setUnits] = createSignal<api.OfficeUnit[]>([]);
  const [windowStart, setWindowStart] = createSignal(0);
  const [next, setNext] = createSignal<number | null>(null);
  const [tab, setTab] = createSignal<Tab>("content");
  const [structure, setStructure] = createSignal<api.OfficeStructure | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);
  let request = 0;

  const select = (anchor: string | null) => {
    setSelected(anchor);
    setCopied(false);
    props.onSelect?.(anchor);
  };

  const load = async (from: number, replace: boolean, scrollTo?: string, at?: string) => {
    const generation = ++request;
    setLoading(true);
    setError(null);
    try {
      const page = await api.readOfficeProjection(props.source, from, at);
      if (generation !== request) return;
      setMeta(page);
      setUnits((current) => (replace ? page.units : [...current, ...page.units]));
      if (replace) setWindowStart(page.from);
      setNext(page.next);
      if (at) {
        if (page.focus) select(page.focus);
        else setError(`${props.fileName} has no place ${at}; it may have changed since it was cited.`);
      }
      const target = scrollTo ?? page.focus;
      if (target) queueMicrotask(() => document.getElementById(unitId(target))?.scrollIntoView({ block: "start" }));
    } catch (cause) {
      if (generation === request) setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (generation === request) setLoading(false);
    }
  };

  createEffect(on(() => [props.source.path, props.source.sessionId, props.source.candidateId, props.focus], () => {
    setMeta(null);
    setUnits([]);
    setStructure(null);
    setTab("content");
    select(null);
    void load(0, true, undefined, props.focus);
  }));

  createEffect(on(tab, (current) => {
    if (current !== "structure" || structure()) return;
    void api.readOfficeStructure(props.source)
      .then(setStructure)
      .catch((cause) => setError(cause instanceof Error ? cause.message : String(cause)));
  }));

  /** Shows the unit at `index`, loading the page it starts if it is not in view. */
  const jump = (entry: api.OfficeOutlineEntry) => {
    const start = windowStart();
    if (entry.first_unit >= start && entry.first_unit < start + units().length) {
      document.getElementById(unitId(units()[entry.first_unit - start].anchor))?.scrollIntoView({ block: "start", behavior: "smooth" });
      return;
    }
    void load(entry.first_unit, true, entry.anchor);
  };

  const ask = (anchor: string) => {
    window.dispatchEvent(new CustomEvent("vak:edit-prompt", {
      detail: { text: `About ${props.fileName} at ${anchor}: `, mode: "append" },
    }));
  };

  const copy = async (anchor: string) => {
    try {
      await navigator.clipboard.writeText(anchor);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  const unitProps = (anchor: string) => ({
    id: unitId(anchor),
    tabIndex: 0,
    "aria-current": selected() === anchor ? ("true" as const) : undefined,
    onClick: () => select(selected() === anchor ? null : anchor),
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        select(selected() === anchor ? null : anchor);
      }
    },
  });

  const statsLine = () => (meta()?.stats ?? [])
    .filter(([, count]) => count > 0)
    .map(([name, count]) => `${count.toLocaleString()} ${name}`)
    .join(" · ");

  return (
    <div class="office-view">
      <Show when={meta()}>{(info) => (
        <>
          <div class="office-view-facts">
            <strong>{info().kind}</strong>
            <Show when={statsLine()}><span>{statsLine()}</span></Show>
            <For each={info().sensitivity_labels}>{(label) => <span class="office-label office-label-sensitivity">Label: {label}</span>}</For>
          </div>
          <Show when={info().flags.length > 0}>
            <div class="office-view-flags" role="note">
              <span>{info().flags.join(" · ")}</span>
              <button type="button" class="btn sm" onClick={() => setTab("structure")}>Show structure</button>
            </div>
          </Show>
          <div class="office-view-tabs" role="tablist" aria-label="Views of this file">
            <button type="button" role="tab" aria-selected={tab() === "content"} classList={{ active: tab() === "content" }} onClick={() => setTab("content")}>{MAIN_TAB[info().vocabulary]}</button>
            <button type="button" role="tab" aria-selected={tab() === "structure"} classList={{ active: tab() === "structure" }} onClick={() => setTab("structure")}>Structure</button>
          </div>
        </>
      )}</Show>

      <Show when={error()}>{(message) => <p role="alert" class="inline-error">{message()}</p>}</Show>
      <Show when={loading() && units().length === 0}><p class="office-view-empty">Reading the file…</p></Show>

      <Show when={tab() === "content" && meta()}>{(info) => (
        <>
          <Show when={info().vocabulary === "excel"} fallback={
            <Show when={info().vocabulary === "power_point"} fallback={
              <DocumentUnits outline={info().outline} units={units()} jump={jump} unitProps={unitProps} />
            }>
              <DeckUnits outline={info().outline} units={units()} jump={jump} unitProps={unitProps} />
            </Show>
          }>
            <WorkbookGrid outline={info().outline} units={units()} jump={jump} selected={selected()} select={select} />
          </Show>
          <div class="office-view-paging">
            <Show when={windowStart() > 0}>
              <button type="button" class="btn sm" disabled={loading()} onClick={() => void load(0, true)}>Back to the start</button>
            </Show>
            <Show when={next() !== null}>
              <button type="button" class="btn sm" disabled={loading()} onClick={() => void load(next()!, false)}>{loading() ? "Loading…" : "Show more"}</button>
              <span>Showing {units().length.toLocaleString()} of {info().total_units.toLocaleString()} parts of this file</span>
            </Show>
          </div>
          <Show when={info().not_read.length > 0}>
            <p class="office-view-not-read">Not shown yet: {info().not_read.join("; ")}.</p>
          </Show>
        </>
      )}</Show>

      <Show when={tab() === "structure"}>
        <Show when={structure()} fallback={<p class="office-view-empty">Reading the structure…</p>}>{(parts) => <StructureView structure={parts()} />}</Show>
      </Show>

      <Show when={selected()}>{(anchor) => (
        <div class="office-view-selection" role="toolbar" aria-label="Selected place">
          <code>{anchor()}</code>
          <button type="button" class="btn primary sm" onClick={() => ask(anchor())}>Ask Vak about this</button>
          <button type="button" class="btn sm" onClick={() => void copy(anchor())}>{copied() ? "Copied" : "Copy anchor"}</button>
          <button type="button" class="btn sm" aria-label="Clear selection" onClick={() => select(null)}>Clear</button>
        </div>
      )}</Show>
    </div>
  );
}

type UnitProps = (anchor: string) => Record<string, unknown>;

function OutlineRail(props: { outline: api.OfficeOutlineEntry[]; jump: (entry: api.OfficeOutlineEntry) => void; label: string }) {
  return (
    <Show when={props.outline.length > 0}>
      <nav class="office-outline" aria-label={props.label}>
        <For each={props.outline}>{(entry) => (
          <button type="button" style={{ "padding-left": `${8 + Math.max(0, entry.level - 1) * 10}px` }} onClick={() => props.jump(entry)}>{entry.title || entry.anchor}</button>
        )}</For>
      </nav>
    </Show>
  );
}

function DocumentUnits(props: { outline: api.OfficeOutlineEntry[]; units: api.OfficeUnit[]; jump: (entry: api.OfficeOutlineEntry) => void; unitProps: UnitProps }) {
  return (
    <div class="office-view-body">
      <OutlineRail outline={props.outline} jump={props.jump} label="Headings" />
      <div class="office-units" role="list">
        <For each={props.units}>{(unit) => (
          <div role="listitem" class={`office-unit office-unit-${unit.kind}`} {...props.unitProps(unit.anchor)}>
            <Show when={unit.kind === "heading" || unit.kind === "page"} fallback={
              <p><Redline text={unit.text} /></p>
            }>
              <p class="office-unit-heading" role="heading" aria-level={Math.min(6, Math.max(2, unit.level + 1))}><Redline text={unit.text} /></p>
            </Show>
            <span class="office-unit-anchor">{unit.anchor}</span>
            <Labels labels={unit.labels} />
          </div>
        )}</For>
      </div>
    </div>
  );
}

/** The reader's break between the paragraphs of one shape (vak_ooxml::read::PARAGRAPH_BREAK). */
const PARAGRAPH_BREAK = " ¶ ";

function DeckUnits(props: { outline: api.OfficeOutlineEntry[]; units: api.OfficeUnit[]; jump: (entry: api.OfficeOutlineEntry) => void; unitProps: UnitProps }) {
  const slides = createMemo(() => {
    const groups: { anchor: string; units: api.OfficeUnit[] }[] = [];
    for (const unit of props.units) {
      const slide = unit.anchor.split("/")[0];
      const last = groups[groups.length - 1];
      if (last && last.anchor === slide) last.units.push(unit);
      else groups.push({ anchor: slide, units: [unit] });
    }
    // The slide's heading already reads "Slide N: <title>", so the title
    // placeholder's own text is not repeated beneath it.
    return groups.map((group) => {
      const heading = group.units.find((unit) => unit.kind === "slide")?.text ?? "";
      const title = heading.replace(/^Slide \d+: /, "");
      const repeatsTitle = (unit: api.OfficeUnit) => unit.kind === "shape" && unit.labels.length === 0 && unit.text === title;
      const first = group.units.findIndex(repeatsTitle);
      return { ...group, units: group.units.filter((_, index) => index !== first) };
    });
  });
  return (
    <div class="office-view-body">
      <OutlineRail outline={props.outline} jump={props.jump} label="Slides" />
      <div class="office-units" role="list">
        <For each={slides()}>{(slide) => (
          <section class="office-slide" aria-label={slide.units.find((unit) => unit.kind === "slide")?.text ?? slide.anchor}>
            <For each={slide.units}>{(unit) => (
              <div role="listitem" class={`office-unit office-unit-${unit.kind}`} {...props.unitProps(unit.anchor)}>
                <Show when={unit.kind === "notes"}><span class="office-unit-kind">Speaker notes</span></Show>
                <For each={unit.text.split(PARAGRAPH_BREAK)}>{(paragraph) => (
                  <p classList={{ "office-unit-heading": unit.kind === "slide" }}><Redline text={paragraph} /></p>
                )}</For>
                <span class="office-unit-anchor">{unit.anchor}</span>
                <Labels labels={unit.labels} />
              </div>
            )}</For>
          </section>
        )}</For>
      </div>
    </div>
  );
}

function WorkbookGrid(props: {
  outline: api.OfficeOutlineEntry[];
  units: api.OfficeUnit[];
  jump: (entry: api.OfficeOutlineEntry) => void;
  selected: string | null;
  select: (anchor: string | null) => void;
}) {
  const [sheet, setSheet] = createSignal<api.OfficeOutlineEntry | null>(null);
  const current = () => {
    const chosen = sheet();
    if (chosen && props.outline.some((entry) => entry.anchor === chosen.anchor)) return chosen;
    const first = props.units.find((unit) => unit.kind === "sheet_row");
    return props.outline.find((entry) => first && first.anchor.startsWith(entry.anchor)) ?? props.outline[0] ?? null;
  };
  const rows = createMemo(() => {
    const entry = current();
    if (!entry) return [] as api.OfficeUnit[];
    return props.units.filter((unit) => unit.kind === "sheet_row" && unit.anchor.startsWith(entry.anchor));
  });
  const grid = createMemo(() => {
    let maxColumn = 0;
    const byRow: { number: number; labels: string[]; cells: Map<number, string> }[] = [];
    for (const row of rows()) {
      const cells = new Map<number, string>();
      let number = 0;
      for (const [address, value] of row.cells ?? []) {
        const at = cellAddress(address);
        if (!at) continue;
        number = at.row;
        cells.set(at.column, value);
        maxColumn = Math.max(maxColumn, at.column);
      }
      if (number > 0) byRow.push({ number, labels: row.labels, cells });
    }
    return { columns: Array.from({ length: maxColumn }, (_, index) => index + 1), rows: byRow };
  });
  const selectedCell = () => {
    const entry = current();
    const anchor = props.selected;
    if (!entry || !anchor || !anchor.startsWith(entry.anchor)) return null;
    const address = anchor.slice(entry.anchor.length);
    const at = cellAddress(address);
    if (!at) return null;
    const value = grid().rows.find((row) => row.number === at.row)?.cells.get(at.column) ?? "";
    return { address, cell: parseCell(value) };
  };
  const choose = (entry: api.OfficeOutlineEntry) => {
    setSheet(entry);
    props.select(null);
    if (!props.units.some((unit) => unit.anchor.startsWith(entry.anchor))) props.jump(entry);
  };
  return (
    <div class="office-workbook">
      <div class="office-formula-bar" aria-live="polite">
        <Show when={selectedCell()} fallback={<span class="office-formula-hint">Select a cell to see its value or formula.</span>}>{(info) => (
          <>
            <code>{info().address}</code>
            <Show when={info().cell.formula} fallback={<span>{info().cell.shown}</span>}>
              <span class="office-formula">{info().cell.formula}</span>
              <span class="office-formula-value">{info().cell.notCalculated ? "not calculated yet" : `= ${info().cell.shown}`}</span>
              <Show when={info().cell.stale}><span class="office-label office-label-stale">stale until opened in Excel</span></Show>
            </Show>
          </>
        )}</Show>
      </div>
      <div class="office-grid-scroll">
        <Show when={grid().rows.length > 0} fallback={<p class="office-view-empty">No cells on this part of the sheet.</p>}>
          <table class="office-grid">
            <thead>
              <tr><th scope="col" /><For each={grid().columns}>{(column) => <th scope="col">{columnName(column)}</th>}</For></tr>
            </thead>
            <tbody>
              <For each={grid().rows}>{(row) => (
                <tr classList={{ "office-grid-flagged": row.labels.length > 0 }} title={row.labels.join(", ") || undefined}>
                  <th scope="row">{row.number}</th>
                  <For each={grid().columns}>{(column) => {
                    const value = row.cells.get(column);
                    const address = `${columnName(column)}${row.number}`;
                    const anchor = () => `${current()?.anchor ?? ""}${address}`;
                    const cell = value === undefined ? null : parseCell(value);
                    return (
                      <td
                        tabIndex={value === undefined ? -1 : 0}
                        classList={{ selected: props.selected === anchor(), stale: Boolean(cell?.stale), formula: Boolean(cell?.formula) }}
                        aria-label={value === undefined ? undefined : `${address}: ${cell?.shown ?? ""}`}
                        onClick={() => value !== undefined && props.select(props.selected === anchor() ? null : anchor())}
                        onKeyDown={(event) => {
                          if (value !== undefined && (event.key === "Enter" || event.key === " ")) {
                            event.preventDefault();
                            props.select(anchor());
                          }
                        }}
                      >{cell?.shown ?? ""}</td>
                    );
                  }}</For>
                </tr>
              )}</For>
            </tbody>
          </table>
        </Show>
      </div>
      <div class="office-sheet-tabs" role="tablist" aria-label="Sheets">
        <For each={props.outline}>{(entry) => (
          <button type="button" role="tab" aria-selected={current()?.anchor === entry.anchor} classList={{ active: current()?.anchor === entry.anchor }} onClick={() => choose(entry)}>{entry.title}</button>
        )}</For>
      </div>
    </div>
  );
}

function StructureView(props: { structure: api.OfficeStructure }) {
  const size = (bytes: number) => bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${bytes} B`;
  return (
    <div class="office-structure">
      <p>Main part <code>{props.structure.main_part}</code> · {props.structure.parts.length} parts · {props.structure.relationships.length} relationships · sha256 <code>{props.structure.sha256.slice(0, 16)}…</code></p>
      <Show when={props.structure.untyped_parts.length > 0}>
        <p class="office-view-flags" role="note">Parts without a content type (Office refuses these): {props.structure.untyped_parts.join(", ")}</p>
      </Show>
      <h4>Relationships</h4>
      <div class="office-grid-scroll">
        <table class="office-table">
          <thead><tr><th scope="col">From</th><th scope="col">Kind</th><th scope="col">Target</th></tr></thead>
          <tbody>
            <For each={props.structure.relationships}>{(relationship) => (
              <tr classList={{ "office-external": relationship.external }}>
                <td><code>{relationship.source || "package"}</code></td>
                <td>{relationship.kind}</td>
                <td><code>{relationship.target}</code><Show when={relationship.external}> <span class="office-label office-label-external">external, never followed</span></Show></td>
              </tr>
            )}</For>
          </tbody>
        </table>
      </div>
      <h4>Parts</h4>
      <div class="office-grid-scroll">
        <table class="office-table">
          <thead><tr><th scope="col">Part</th><th scope="col">Content type</th><th scope="col">Size</th></tr></thead>
          <tbody>
            <For each={props.structure.parts}>{(part) => (
              <tr>
                <td><code>{part.name}</code></td>
                <td>{part.content_type ?? <span class="office-label office-label-external">none</span>}</td>
                <td>{size(part.size)}</td>
              </tr>
            )}</For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
