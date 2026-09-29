import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import * as api from "../api";
import Redline from "./OfficeRedline";
import { cellAddress, cellRange, columnName, parseCell, parseCellInput } from "../officeCells";
import { countOf } from "../officeFacts";
import { technicalDetails } from "../store";

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
  pdf: "Document",
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
  /** Human edits are submitted as typed OpenXML operations to the shared draft. */
  canEdit?: boolean;
  onEdit?: (operation: api.OfficeEditOp) => Promise<void> | void;
}) {
  const [meta, setMeta] = createSignal<api.OfficeProjection | null>(null);
  const [units, setUnits] = createSignal<api.OfficeUnit[]>([]);
  const [windowStart, setWindowStart] = createSignal(0);
  const [next, setNext] = createSignal<number | null>(null);
  const [tab, setTab] = createSignal<Tab>("content");
  const [showOutline, setShowOutline] = createSignal(false);
  const [structure, setStructure] = createSignal<api.OfficeStructure | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);
  const [editText, setEditText] = createSignal("");
  const [savingEdit, setSavingEdit] = createSignal(false);
  const [editError, setEditError] = createSignal<string | null>(null);
  const [showPresentation, setShowPresentation] = createSignal(false);
  const [presentationIndex, setPresentationIndex] = createSignal(0);
  const [presentationUnits, setPresentationUnits] = createSignal<api.OfficeUnit[]>([]);
  const [presentationLoading, setPresentationLoading] = createSignal(false);
  const [presentationError, setPresentationError] = createSignal<string | null>(null);
  const [showNotes, setShowNotes] = createSignal(false);
  let presentationRoot: HTMLDivElement | undefined;
  let presentationRequest = 0;
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
        // A workbook citation names cells; select those, not the whole row.
        if (page.focus) select(page.vocabulary === "excel" && cellRange(at.slice(at.lastIndexOf("!") + 1)) ? at.trim() : page.focus);
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
    presentationRequest += 1;
    setShowPresentation(false);
    setMeta(null);
    setUnits([]);
    setStructure(null);
    setTab("content");
    setShowOutline(false);
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
    .map(([name, count]) => countOf(name, count))
    .join(" · ");

  const editable = () => {
    const info = meta();
    const anchor = selected();
    if (!props.canEdit || !props.onEdit || !info || !anchor) return null;
    if (info.vocabulary === "excel") {
      const marker = anchor.lastIndexOf("!");
      if (marker < 0) return null;
      const sheet = anchor.slice(0, marker).replace(/^'/, "").replace(/'$/, "").replace(/''/g, "'");
      const address = anchor.slice(marker + 1);
      const range = cellRange(address);
      if (!sheet || !range || range.first.column !== range.last.column || range.first.row !== range.last.row) return null;
      const row = units().find((unit) => unit.kind === "sheet_row" && unit.anchor.startsWith(`${anchor.slice(0, marker + 1)}`));
      if (row?.labels.some((label) => /hidden|very hidden/i.test(label))) return null;
      const raw = row?.cells?.find(([cell]) => cell === address)?.[1] ?? "";
      const cell = parseCell(raw);
      return { value: cell.formula || cell.shown, operation: (text: string): api.OfficeEditOp => ({ op: "set_cells", sheet, cells: { [address]: parseCellInput(text) } }) };
    }
    const unit = units().find((entry) => entry.anchor === anchor);
    if (!unit) return null;
    if (info.vocabulary === "word" && (unit.kind === "paragraph" || unit.kind === "heading")) {
      if (unit.labels.some((label) => /hidden|white text|tracked changes/i.test(label))) return null;
      const value = unit.text.replace(/\[(?:inserted by|deleted by) [^:]+: ([^\]]*)\]/g, "$1").replace(/\[(?:hidden|white text): ([^\]]*)\]/g, "$1");
      return { value, operation: (text: string): api.OfficeEditOp => ({ op: "replace_paragraph_text", anchor, text }) };
    }
    if (info.vocabulary === "pdf" && unit.kind === "paragraph" && unit.labels.length === 0) {
      return { value: unit.text, operation: (text: string): api.OfficeEditOp => ({ op: "replace_paragraph_text", anchor, text }) };
    }
    if (info.vocabulary === "power_point" && unit.kind === "shape" && unit.labels.length === 0) {
      return { value: unit.text.replace(/ ¶ /g, "\n"), operation: (text: string): api.OfficeEditOp => ({ op: "set_placeholder_text", anchor, text }) };
    }
    return null;
  };

  createEffect(on(() => [selected(), units(), meta()?.vocabulary], () => {
    const target = editable();
    setEditText(target?.value ?? "");
    setEditError(null);
  }));

  const saveEdit = async () => {
    const target = editable();
    if (!target || savingEdit()) return;
    setSavingEdit(true);
    setEditError(null);
    try { await props.onEdit?.(target.operation(editText())); }
    catch (cause) { setEditError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setSavingEdit(false); }
  };

  const deckSlides = createMemo(() => {
    const groups: { anchor: string; title: string; units: api.OfficeUnit[] }[] = [];
    for (const unit of presentationUnits()) {
      const anchor = unit.anchor.split("/")[0];
      let slide = groups[groups.length - 1];
      if (!slide || slide.anchor !== anchor) {
        slide = { anchor, title: "", units: [] };
        groups.push(slide);
      }
      slide.units.push(unit);
      if (unit.kind === "slide") slide.title = unit.text.replace(/^Slide \d+: /, "");
    }
    return groups;
  });
  const showSlide = (index: number) => {
    const slides = deckSlides();
    const target = Math.max(0, Math.min(index, slides.length - 1));
    setPresentationIndex(target);
  };
  const startPresentation = async () => {
    setShowPresentation(true);
    setPresentationIndex(0);
    setShowNotes(false);
    setPresentationError(null);
    setPresentationUnits([]);
    setPresentationLoading(true);
    const generation = ++presentationRequest;
    // Mobile browsers often reject fullscreen for an element. The viewport
    // presentation remains available there without changing browser chrome.
    if (!window.matchMedia("(max-width: 600px)").matches) {
      void presentationRoot?.requestFullscreen?.().catch(() => undefined);
    }
    try {
      const collected: api.OfficeUnit[] = [];
      let from: number | null = 0;
      while (from !== null) {
        const page: api.OfficeProjection = await api.readOfficeProjection(props.source, from);
        if (generation !== presentationRequest) return;
        collected.push(...page.units);
        from = page.next;
      }
      setPresentationUnits(collected);
    } catch (cause) {
      if (generation === presentationRequest) setPresentationError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (generation === presentationRequest) setPresentationLoading(false);
    }
  };
  const stopPresentation = () => {
    presentationRequest += 1;
    setShowPresentation(false);
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined);
  };
  createEffect(() => {
    if (!showPresentation()) return;
    const onKey = (event: KeyboardEvent) => {
      if (["ArrowRight", "PageDown", " "].includes(event.key)) { event.preventDefault(); showSlide(presentationIndex() + 1); }
      else if (["ArrowLeft", "PageUp"].includes(event.key)) { event.preventDefault(); showSlide(presentationIndex() - 1); }
      else if (event.key === "Home") { event.preventDefault(); showSlide(0); }
      else if (event.key === "End") { event.preventDefault(); showSlide(deckSlides().length - 1); }
      else if (event.key === "Escape") stopPresentation();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div classList={{ "office-view": true, "office-presentation-active": showPresentation(), "office-outline-open": showOutline() }} ref={presentationRoot}>
      <Show when={showPresentation()}>
        {(() => {
          const slides = () => deckSlides();
          const slide = () => slides()[presentationIndex()];
          return <section class="office-presentation" aria-label="Presentation slide">
            <header><span>{props.fileName}</span><span>{slides().length ? `Slide ${presentationIndex() + 1} of ${slides().length}` : "Presentation"}</span><button type="button" onClick={stopPresentation}>Exit presentation</button></header>
            <main onClick={() => showSlide(presentationIndex() + 1)}>
              <Show when={presentationLoading()}><p role="status">Preparing slides…</p></Show>
              <Show when={presentationError()}>{(message) => <p role="alert">Could not prepare the presentation: {message()}</p>}</Show>
              <Show when={slide()}>{(current) => <>
                <Show when={current().title}><h1>{current().title}</h1></Show>
                <For each={current().units.filter((unit) => unit.kind !== "slide" && unit.kind !== "notes" && !(unit.kind === "shape" && unit.text === current().title))}>
                  {(unit) => <div class="office-presentation-shape"><For each={unit.text.split(PARAGRAPH_BREAK)}>{(paragraph) => <p><Redline text={paragraph} /></p>}</For></div>}
                </For>
              </>}</Show>
            </main>
            <Show when={showNotes() && slide()?.units.some((unit) => unit.kind === "notes")}><aside class="office-presentation-notes"><For each={slide()?.units.filter((unit) => unit.kind === "notes")}>{(unit) => <p>{unit.text}</p>}</For></aside></Show>
            <footer><button type="button" disabled={presentationIndex() === 0} onClick={() => showSlide(presentationIndex() - 1)}>Previous</button><button type="button" disabled={presentationIndex() >= slides().length - 1} onClick={() => showSlide(presentationIndex() + 1)}>Next</button><button type="button" onClick={() => setShowNotes(!showNotes())}>{showNotes() ? "Hide notes" : "Show notes"}</button><span>Use ← and → to navigate · Esc to exit</span></footer>
          </section>;
        })()}
      </Show>
      <Show when={meta()}>{(info) => (
        <>
          <div class="office-view-facts">
            <strong>{info().kind}</strong>
            <Show when={statsLine()}><span>{statsLine()}</span></Show>
            <For each={info().sensitivity_labels}>{(label) => <span class="office-label office-label-sensitivity">Label: {label}</span>}</For>
            <Show when={info().vocabulary === "power_point"}><button type="button" class="btn sm office-present-action" onClick={() => void startPresentation()}>Present</button></Show>
          </div>
          <Show when={info().flags.length > 0}>
            <div class="office-view-flags" role="note">
              <span>{info().flags.join(" · ")}</span>
              <Show when={info().vocabulary !== "pdf"}><button type="button" class="btn sm" onClick={() => setTab("structure")}>Show structure</button></Show>
            </div>
          </Show>
          <div class="office-view-tabs" role="tablist" aria-label="Views of this file">
            <button type="button" role="tab" aria-selected={tab() === "content"} classList={{ active: tab() === "content" }} onClick={() => setTab("content")}>{MAIN_TAB[info().vocabulary]}</button>
            <Show when={info().vocabulary !== "pdf"}><button type="button" role="tab" aria-selected={tab() === "structure"} classList={{ active: tab() === "structure" }} onClick={() => setTab("structure")}>Structure</button></Show>
            <Show when={info().outline.length > 0 && tab() === "content" && info().vocabulary !== "excel"}><button type="button" class="office-outline-toggle" aria-expanded={showOutline()} onClick={() => setShowOutline((open) => !open)}>{showOutline() ? "Hide outline" : "Outline"}</button></Show>
          </div>
        </>
      )}</Show>

      <Show when={error()}>{(message) => <p role="alert" class="inline-error">{message()}</p>}</Show>
      <Show when={loading() && units().length === 0}><p class="office-view-empty">Reading the file…</p></Show>

      <Show when={tab() === "content" && meta()}>{(info) => (
        <>
          <Show when={info().vocabulary === "excel"} fallback={
            <Show when={info().vocabulary === "power_point"} fallback={
              <DocumentUnits outline={info().outline} units={units()} media={info().media ?? []} jump={jump} unitProps={unitProps} />
            }>
              <DeckUnits outline={info().outline} units={units()} media={info().media ?? []} jump={jump} unitProps={unitProps} />
            </Show>
          }>
            <WorkbookGrid outline={info().outline} units={units()} media={info().media ?? []} cellStyles={info().cell_styles ?? {}} tableStyles={info().table_styles ?? []} displayValues={info().display_values ?? {}} geometry={info().sheet_geometry ?? { default_column_widths: {}, default_row_heights: {}, column_widths: {}, row_heights: {} }} jump={jump} selected={selected()} select={select} />
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
            <details class="office-view-not-read"><summary>Some content may be missing from this view</summary><p>{info().not_read.join("; ")}.</p></details>
          </Show>
        </>
      )}</Show>

      <Show when={tab() === "structure"}>
        <Show when={structure()} fallback={<p class="office-view-empty">Reading the structure…</p>}>{(parts) => <StructureView structure={parts()} />}</Show>
      </Show>

      <Show when={selected()}>{(anchor) => (
        <div class="office-view-selection" role="toolbar" aria-label="Selected place">
          <code>{anchor()}</code>
          <Show when={editable()}>
            <textarea aria-label="Edit selected Office content" value={editText()} onInput={(event) => setEditText(event.currentTarget.value)} rows={2} />
            <button type="button" class="btn primary sm" disabled={savingEdit()} onClick={() => void saveEdit()}>{savingEdit() ? "Saving draft…" : "Save to shared draft"}</button>
          </Show>
          <button type="button" class="btn primary sm" onClick={() => ask(anchor())}>Ask Vakyartha about this</button>
          <button type="button" class="btn sm" onClick={() => void copy(anchor())}>{copied() ? "Copied" : "Copy anchor"}</button>
          <button type="button" class="btn sm" aria-label="Clear selection" onClick={() => select(null)}>Clear</button>
        </div>
      )}</Show>
      <Show when={editError()}>{(message) => <p class="office-view-edit-error" role="alert">{message()}</p>}</Show>
    </div>
  );
}

type UnitProps = (anchor: string) => Record<string, unknown>;

type OfficeBlock =
  | { kind: "unit"; unit: api.OfficeUnit }
  | { kind: "table"; anchor: string; rows: api.OfficeUnit[] }
  | { kind: "chart"; anchor: string; rows: api.OfficeUnit[] };

function officeBlocks(units: api.OfficeUnit[]): OfficeBlock[] {
  const result: OfficeBlock[] = [];
  for (const unit of units) {
    if (unit.kind !== "table_row") {
      result.push({ kind: "unit", unit });
      continue;
    }
    const anchor = unit.anchor.replace(/\/r\d+$/, "");
    const chart = unit.labels.some((label) => label === "chart data; cached values");
    const last = result[result.length - 1];
    if ((last?.kind === "table" || last?.kind === "chart") && last.anchor === anchor) last.rows.push(unit);
    else result.push({ kind: chart ? "chart" : "table", anchor, rows: [unit] });
  }
  return result;
}

function OfficeChart(props: { anchor: string; rows: api.OfficeUnit[]; unitProps: UnitProps; placement?: string }) {
  const values = () => props.rows.map((row) => (row.row_cells ?? []).map((cell) => cell.map(([, text]) => text).join(" ")));
  const title = () => props.rows.flatMap((row) => row.labels).find((label) => label.startsWith("chart title: "))?.slice("chart title: ".length) ?? "Chart";
  const headers = () => values()[0] ?? [];
  const seriesColumn = () => Math.max(1, headers().findIndex((_, index) => index > 0));
  const points = () => values().slice(1).flatMap((row) => {
    const value = Number(row[seriesColumn()]);
    return Number.isFinite(value) ? [{ label: row[0] ?? "", value }] : [];
  });
  const maximum = () => Math.max(1, ...points().map((point) => Math.abs(point.value)));
  return <figure class="office-unit office-chart" role="listitem" {...props.unitProps(props.anchor)}>
    <figcaption>{title()}<Show when={props.placement}><span class="office-object-position">Placed at {props.placement}</span></Show></figcaption>
    <div class="office-chart-bars" role="img" aria-label={`${title()}, bar chart. ${points().map((point) => `${point.label}: ${point.value}`).join(", ")}`}>
      <For each={points().slice(0, 16)}>{(point) => <div class="office-chart-point">
        <span class="office-chart-value">{point.value}</span>
        <span class="office-chart-track"><span class="office-chart-bar" style={{ width: `${Math.max(2, Math.abs(point.value) / maximum() * 100)}%` }} /></span>
        <span class="office-chart-label">{point.label}</span>
      </div>}</For>
    </div>
    <Show when={points().length > 16}><p class="office-chart-more">Showing 16 of {points().length} categories.</p></Show>
    <details class="office-chart-source"><summary>Show chart data</summary>
      <div class="office-grid-scroll"><table class="office-table"><tbody><For each={props.rows}>{(row, index) => <tr>{<For each={values()[index()]}>{(cell) => index() === 0 ? <th scope="col">{cell}</th> : <td>{cell}</td>}</For>}</tr>}</For></tbody></table></div>
    </details>
    <Show when={technicalDetails()}><span class="office-unit-anchor">{props.anchor}</span></Show>
  </figure>;
}

function DocumentTable(props: { anchor: string; rows: api.OfficeUnit[]; unitProps: UnitProps; placement?: string }) {
  return <div class="office-unit office-unit-table" role="listitem" {...props.unitProps(props.anchor)}>
    <Show when={props.placement}><span class="office-object-position">{props.placement}</span></Show>
    <div class="office-grid-scroll">
      <table class="office-table" aria-label="Document table">
        <tbody><For each={props.rows}>{(row, rowIndex) => <tr {...props.unitProps(row.anchor)}>
          <For each={row.row_cells?.length ? row.row_cells : [[["", row.text] as [string, string]]]}>{(cell) => {
            const content = <For each={cell}>{([anchor, text]) => <p {...(anchor ? props.unitProps(anchor) : {})}>{text}</p>}</For>;
            return rowIndex() === 0 ? <th scope="col">{content}</th> : <td>{content}</td>;
          }}</For>
        </tr>}</For></tbody>
      </table>
    </div>
    <Show when={technicalDetails()}><span class="office-unit-anchor">{props.anchor}</span></Show>
  </div>;
}

function OfficeImage(props: { unit: api.OfficeUnit; media: api.OfficeMediaPreview[]; unitProps: UnitProps; compact?: boolean }) {
  const position = () => props.unit.labels.find((label) => label.startsWith("image position: "))?.slice("image position: ".length);
  const preview = () => {
    const cell = position()?.split("!").pop();
    const objectId = props.unit.anchor.match(/(?:image@|shape:)([^/]+)$/)?.[1];
    return props.media.find((entry) => entry.alt_text === props.unit.text
      && (!objectId || entry.object_id === objectId)
      && (!entry.cell || !cell || entry.cell === cell));
  };
  return <figure class="office-unit office-image" role="listitem" {...props.unitProps(props.unit.anchor)}>
    <Show when={preview()} fallback={<div class="office-image-unavailable">Image preview unavailable</div>}>
      {(image) => <img src={image().data_url} alt={props.unit.text} loading="lazy" />}
    </Show>
    <Show when={props.compact}>
      <span class="office-image-position-badge" aria-hidden="true">{position()?.split("!").pop()}</span>
    </Show>
    <Show when={!props.compact}><figcaption><span class="office-unit-kind">Image description</span><Show when={position()}>{(value) => <span class="office-object-position">Placed at {value()}</span>}</Show><p>{props.unit.text}</p><Labels labels={props.unit.labels.filter((label) => !label.startsWith("image position:"))} /></figcaption></Show>
    <Show when={technicalDetails()}><span class="office-unit-anchor">{props.unit.anchor}</span></Show>
  </figure>;
}

function OutlineRail(props: { outline: api.OfficeOutlineEntry[]; jump: (entry: api.OfficeOutlineEntry) => void; label: string }) {
  return (
    <Show when={props.outline.length > 0}>
      <nav class="office-outline" aria-label={props.label}>
        <For each={props.outline}>{(entry) => (
          <button type="button" style={{ "--depth": String(Math.max(0, entry.level - 1)) }} onClick={() => props.jump(entry)}>{entry.title || entry.anchor}</button>
        )}</For>
      </nav>
    </Show>
  );
}

function DocumentUnits(props: { outline: api.OfficeOutlineEntry[]; units: api.OfficeUnit[]; media: api.OfficeMediaPreview[]; jump: (entry: api.OfficeOutlineEntry) => void; unitProps: UnitProps }) {
  const blocks = createMemo(() => officeBlocks(props.units));
  return (
    <div class="office-view-body">
      <OutlineRail outline={props.outline} jump={props.jump} label="Headings" />
      <div class="office-units" role="list">
        <For each={blocks()}>{(block) => block.kind === "chart" ? (
          <OfficeChart anchor={block.anchor} rows={block.rows} placement={block.rows.flatMap((row) => row.labels).find((label) => label.startsWith("chart position: "))?.slice("chart position: ".length)} unitProps={props.unitProps} />
        ) : block.kind === "table" ? (
          <DocumentTable anchor={block.anchor} rows={block.rows} placement={block.rows.flatMap((row) => row.labels).find((label) => label.startsWith("table position: "))?.slice("table position: ".length)} unitProps={props.unitProps} />
        ) : (() => {
          const unit = block.unit;
          if (unit.kind === "image") return <OfficeImage unit={unit} media={props.media} unitProps={props.unitProps} />;
          return <div role="listitem" class={`office-unit office-unit-${unit.kind}`} {...props.unitProps(unit.anchor)}>
            <Show when={unit.kind === "heading" || unit.kind === "page"} fallback={<p><Redline text={unit.text} /></p>}>
              <p class="office-unit-heading" role="heading" aria-level={Math.min(6, Math.max(2, unit.level + 1))}><Redline text={unit.text} /></p>
            </Show>
            <Show when={technicalDetails()}><span class="office-unit-anchor">{unit.anchor}</span></Show>
            <Labels labels={unit.labels} />
          </div>;
        })()}</For>
      </div>
    </div>
  );
}

/** The reader's break between the paragraphs of one shape (vak_ooxml::read::PARAGRAPH_BREAK). */
const PARAGRAPH_BREAK = " ¶ ";

function DeckUnits(props: { outline: api.OfficeOutlineEntry[]; units: api.OfficeUnit[]; media: api.OfficeMediaPreview[]; jump: (entry: api.OfficeOutlineEntry) => void; unitProps: UnitProps }) {
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
      const units = group.units.filter((_, index) => index !== first);
      return { ...group, units, blocks: officeBlocks(units) };
    });
  });
  return (
    <div class="office-view-body">
      <OutlineRail outline={props.outline} jump={props.jump} label="Slides" />
      <div class="office-units office-deck-canvas" role="list">
        <For each={slides()}>{(slide) => (
          <section class="office-slide" aria-label={slide.units.find((unit) => unit.kind === "slide")?.text ?? slide.anchor}>
            <For each={slide.blocks}>{(block) => {
              const positionedUnit = block.kind === "unit" ? block.unit : block.rows[0];
              const positionLabel = positionedUnit.labels.find((label) => label.startsWith("chart position: ") || label.startsWith("table position: ") || label.startsWith("image position: ") || label.startsWith("shape position: "));
              const position = positionLabel?.replace(/^(chart|table|image|shape) position: /, "");
              const match = position?.match(/x ([\d.]+) in from left, y ([\d.]+) in from top(?:, width ([\d.]+) in, height ([\d.]+) in)?/);
              const positionStyle = match ? { left: `${Number(match[1]) * 72}px`, top: `${Number(match[2]) * 72 + 36}px`, width: match[3] ? `${Number(match[3]) * 72}px` : undefined, minHeight: match[4] ? `${Number(match[4]) * 72}px` : undefined } : undefined;
              const content = block.kind === "chart" ? (
                <OfficeChart anchor={block.anchor} rows={block.rows} placement={position} unitProps={props.unitProps} />
              ) : block.kind === "table" ? (
                <DocumentTable anchor={block.anchor} rows={block.rows} placement={position} unitProps={props.unitProps} />
              ) : (() => {
                const unit = block.unit;
                if (unit.kind === "image") return <OfficeImage unit={unit} media={props.media} unitProps={props.unitProps} />;
                return (
                  <div role="listitem" class={`office-unit office-unit-${unit.kind}`} {...props.unitProps(unit.anchor)}>
                    <Show when={unit.kind === "notes"}><span class="office-unit-kind">Speaker notes</span></Show>
                    <For each={unit.text.split(PARAGRAPH_BREAK)}>{(paragraph) => (
                      <p classList={{ "office-unit-heading": unit.kind === "slide" }}><Redline text={paragraph} /></p>
                    )}</For>
                    <Show when={technicalDetails()}><span class="office-unit-anchor">{unit.anchor}</span></Show>
                    <Labels labels={unit.kind === "notes" ? unit.labels.filter((label) => label !== "speaker notes") : unit.labels} />
                  </div>
                );
              })();
              return positionStyle ? <div class="office-slide-object" style={positionStyle}>{content}</div> : content;
            }}</For>
          </section>
        )}</For>
      </div>
    </div>
  );
}

function WorkbookGrid(props: {
  outline: api.OfficeOutlineEntry[];
  units: api.OfficeUnit[];
  media: api.OfficeMediaPreview[];
  cellStyles: Record<string, api.OfficeCellStyle>;
  tableStyles: api.OfficeTableStyleRange[];
  displayValues: Record<string, string>;
  geometry: { default_column_widths: Record<string, number>; default_row_heights: Record<string, number>; column_widths: Record<string, number>; row_heights: Record<string, number> };
  jump: (entry: api.OfficeOutlineEntry) => void;
  selected: string | null;
  select: (anchor: string | null) => void;
}) {
  const [sheet, setSheet] = createSignal<api.OfficeOutlineEntry | null>(null);
  const [objectPositions, setObjectPositions] = createSignal<Record<string, { left: string; top: string; width: string; height: string }>>({});
  let canvasRoot: HTMLDivElement | undefined;
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
  const dimensions = (labels: string[], kind: "chart" | "image") => {
    const match = labels.find((label) => label.startsWith(`${kind} size: `))?.match(/(\d+)x(\d+)px$/);
    return match ? { widthPx: Number(match[1]), heightPx: Number(match[2]) } : {};
  };
  const objectAnchors = () => [
    ...officeBlocks(props.units).flatMap((block) => block.kind === "chart" ? (() => {
      const labels = block.rows.flatMap((row) => row.labels);
      return [{
        cell: labels.find((label) => label.startsWith("chart position: "))?.split("!").pop(),
        endCell: labels.find((label) => label.startsWith("chart end cell: "))?.split("!").pop(),
        ...dimensions(labels, "chart"),
      }];
    })() : []),
    ...props.units.filter((unit) => unit.kind === "image").map((unit) => {
      const media = props.media.find((entry) => entry.alt_text === unit.text);
      return {
        cell: unit.labels.find((label) => label.startsWith("image position: "))?.split("!").pop() ?? media?.cell,
        endCell: unit.labels.find((label) => label.startsWith("image end cell: "))?.split("!").pop() ?? media?.end_cell,
        ...dimensions(unit.labels, "image"),
        widthPx: media?.width_px ?? dimensions(unit.labels, "image").widthPx,
        heightPx: media?.height_px ?? dimensions(unit.labels, "image").heightPx,
      };
    }),
  ];
  const objectCells = () => objectAnchors().flatMap(({ cell, endCell }) => [cell, endCell]).filter((cell): cell is string => Boolean(cell));
  const grid = createMemo(() => {
    let maxColumn = 0;
    const byRow: { number: number; anchor: string; labels: string[]; cells: Map<number, string>; styles: Map<number, api.OfficeCellStyle> }[] = [];
    const stylesByRow = new Map<number, Map<number, api.OfficeCellStyle>>();
    for (const [key, style] of Object.entries(props.cellStyles)) {
      const [sheetName, address] = key.split("!");
      if (`${sheetName}!` !== current()?.anchor) continue;
      const at = cellAddress(address);
      if (!at) continue;
      let styles = stylesByRow.get(at.row);
      if (!styles) stylesByRow.set(at.row, (styles = new Map()));
      styles.set(at.column, style);
    }
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
      if (number > 0) byRow.push({ number, anchor: row.anchor, labels: row.labels, cells, styles: stylesByRow.get(number) ?? new Map() });
    }
    const objectPoints = objectCells().map(cellAddress).filter((cell): cell is NonNullable<typeof cell> => Boolean(cell));
    const maxObjectColumn = objectPoints.reduce((max, cell) => Math.max(max, cell.column), 1);
    const maxObjectRow = objectPoints.reduce((max, cell) => Math.max(max, cell.row), 1);
    // Keep the familiar blank working area of a spreadsheet visible even
    // when the saved file has only a few used cells. Objects still extend
    // this viewport to their native anchor so their measured overlays land
    // on the correct cell.
    const lastRow = Math.max(byRow.at(-1)?.number ?? 1, maxObjectRow, 32);
    const populatedRows = new Map(byRow.map((row) => [row.number, row]));
    const expandedRows = Array.from({ length: lastRow }, (_, index) => populatedRows.get(index + 1) ?? ({ number: index + 1, anchor: `${current()?.anchor ?? ""}${index + 1}`, labels: [], cells: new Map<number, string>(), styles: stylesByRow.get(index + 1) ?? new Map<number, api.OfficeCellStyle>() }));
    return { columns: Array.from({ length: Math.max(maxColumn, maxObjectColumn, 16) }, (_, index) => index + 1), rows: expandedRows };
  });
  const selectedRange = () => {
    const entry = current();
    const anchor = props.selected;
    if (!entry || !anchor || !anchor.startsWith(entry.anchor)) return null;
    return cellRange(anchor.slice(entry.anchor.length));
  };
  const selectedCell = () => {
    const range = selectedRange();
    if (!range) return null;
    const address = `${columnName(range.first.column)}${range.first.row}`;
    const value = grid().rows.find((row) => row.number === range.first.row)?.cells.get(range.first.column) ?? "";
    return { address, cell: parseCell(value) };
  };
  const tableForCell = (column: number, row: number) => props.tableStyles.find((table) => {
    if (table.sheet_anchor !== current()?.anchor) return false;
    const range = cellRange(table.range);
    return Boolean(range && column >= range.first.column && column <= range.last.column && row >= range.first.row && row <= range.last.row);
  });
  const inSelection = (column: number, row: number) => {
    const range = selectedRange();
    return Boolean(range && column >= range.first.column && column <= range.last.column && row >= range.first.row && row <= range.last.row);
  };
  const columnWidth = (column: number) => {
    const sheet = current()?.anchor ?? "";
    const sheetName = sheet.endsWith("!") ? sheet.slice(0, -1) : sheet;
    return props.geometry.column_widths[`${sheet}${columnName(column)}`]
      ?? props.geometry.default_column_widths[sheetName]
      ?? 64;
  };
  const rowHeight = (row: number) => {
    const sheet = current()?.anchor ?? "";
    const sheetName = sheet.endsWith("!") ? sheet.slice(0, -1) : sheet;
    return props.geometry.row_heights[`${sheet}${row}`]
      ?? props.geometry.default_row_heights[sheetName]
      ?? 20;
  };
  const measureObjectPositions = () => {
    const canvas = canvasRoot;
    if (!canvas) return;
    const parent = canvas.getBoundingClientRect();
    const measured: Record<string, { left: string; top: string; width: string; height: string }> = {};
    for (const { cell, endCell, widthPx, heightPx } of objectAnchors()) {
      if (!cell) continue;
      const target = canvas.querySelector<HTMLElement>(`[data-cell="${cell.toUpperCase()}"]`);
      if (!target) continue;
      const box = target.getBoundingClientRect();
      const end = endCell ? canvas.querySelector<HTMLElement>(`[data-cell="${endCell.toUpperCase()}"]`)?.getBoundingClientRect() : undefined;
      measured[cell.toUpperCase()] = {
        left: `${box.left - parent.left}px`,
        top: `${box.top - parent.top}px`,
        width: end ? `${Math.max(48, end.left - box.left)}px` : `${widthPx ?? 640}px`,
        height: end ? `${Math.max(48, end.top - box.top)}px` : `${heightPx ?? 384}px`,
      };
    }
    setObjectPositions((previous) => {
      const keys = Object.keys(measured);
      if (keys.length === Object.keys(previous).length && keys.every((key) => previous[key]?.left === measured[key].left && previous[key]?.top === measured[key].top && previous[key]?.width === measured[key].width && previous[key]?.height === measured[key].height)) return previous;
      return measured;
    });
  };
  let scheduleMeasure = () => {};
  onMount(() => {
    const canvas = canvasRoot;
    if (!canvas) return;
    let frame = 0;
    scheduleMeasure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measureObjectPositions);
    };
    const observer = new ResizeObserver(scheduleMeasure);
    observer.observe(canvas);
    scheduleMeasure();
    onCleanup(() => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    });
  });
  createEffect(() => {
    grid();
    objectCells();
    scheduleMeasure();
  });
  const chartBlocks = () => officeBlocks(props.units).filter((block): block is Extract<OfficeBlock, { kind: "chart" }> => block.kind === "chart");
  const imageUnits = () => props.units.filter((unit) => unit.kind === "image");
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
            <Show when={info().cell.formula} fallback={<span>{props.displayValues[`${current()?.anchor ?? ""}${info().address}`] ?? info().cell.shown}</span>}>
              <span class="office-formula">{info().cell.formula}</span>
              <span class="office-formula-value">{info().cell.notCalculated ? "not calculated yet" : `= ${info().cell.shown}`}</span>
              <Show when={info().cell.stale}><span class="office-label office-label-stale">stale until opened in Excel</span></Show>
            </Show>
          </>
        )}</Show>
      </div>
      <div class="office-grid-scroll">
        <Show when={grid().rows.length > 0} fallback={<p class="office-view-empty">No cells on this part of the sheet.</p>}>
          <div class="office-sheet-canvas" ref={canvasRoot}><table class="office-grid">
            <colgroup>
              <col class="office-grid-row-number" />
              <For each={grid().columns}>{(column) => <col classList={{ "office-grid-hidden-column": columnWidth(column) === 0 }} style={{ width: `${columnWidth(column)}px` }} />}</For>
            </colgroup>
            <thead>
              <tr><th scope="col" /><For each={grid().columns}>{(column) => <th scope="col">{columnName(column)}</th>}</For></tr>
            </thead>
            <tbody>
              <For each={grid().rows}>{(row) => (
                <tr id={unitId(row.anchor)} classList={{ "office-grid-flagged": row.labels.length > 0, "office-grid-hidden-row": rowHeight(row.number) === 0 }} title={row.labels.join(", ") || undefined} style={{ height: `${rowHeight(row.number)}px` }}>
                  <th scope="row">{row.number}</th>
                  <For each={grid().columns}>{(column) => {
                    const value = row.cells.get(column);
                    const address = `${columnName(column)}${row.number}`;
                    const anchor = () => `${current()?.anchor ?? ""}${address}`;
                    const cell = value === undefined ? null : parseCell(value);
                    const shown = cell ? (cell.notCalculated && cell.formula ? cell.formula : cell.shown) : "";
                    const style = row.styles.get(column);
                    const horizontalAlignment = style?.horizontal_alignment;
                    const defaultAlignment = cell && cell.shown.trim() !== "" && Number.isFinite(Number(cell.shown)) ? "right" : "left";
                    const resolvedAlignment: "left" | "right" | "center" | "justify" | undefined = horizontalAlignment === "centerContinuous" ? "center"
                      : horizontalAlignment === "distributed" ? "justify"
                      : horizontalAlignment === "fill" ? "right"
                      : horizontalAlignment === "left" || horizontalAlignment === "right" || horizontalAlignment === "center" || horizontalAlignment === "justify" ? horizontalAlignment
                      : defaultAlignment;
                    const verticalAlignment = style?.vertical_alignment === "center" ? "middle"
                      : style?.vertical_alignment === "justify" || style?.vertical_alignment === "distributed" ? "middle"
                      : style?.vertical_alignment;
                    const table = tableForCell(column, row.number);
                    const tableRange = table ? cellRange(table.range) : null;
                    return (
                      <td
                        data-cell={address}
                        tabIndex={props.selected === anchor() || (!props.selected && address === "A1") ? 0 : -1}
                        classList={{
                          selected: props.selected === anchor() || inSelection(column, row.number),
                          stale: Boolean(cell?.stale),
                          formula: Boolean(cell?.formula),
                          "office-table-header": Boolean(tableRange && row.number === tableRange.first.row),
                          "office-table-row-band": Boolean(tableRange && table?.show_row_stripes && row.number > tableRange.first.row && (row.number - tableRange.first.row) % 2 === 0),
                          "office-table-column-band": Boolean(tableRange && table?.show_column_stripes && (column - tableRange.first.column) % 2 === 0),
                        }}
                        style={{
                          "text-align": resolvedAlignment,
                          "vertical-align": verticalAlignment,
                          "white-space": style?.wrap_text ? "pre-wrap" : undefined,
                          "background-color": style?.fill_color,
                          color: style?.font_color,
                          "font-weight": style?.bold ? "700" : undefined,
                          "font-style": style?.italic ? "italic" : undefined,
                        }}
                        title={cell?.notCalculated ? "Not calculated yet: Excel works this out when it opens the file" : undefined}
                        aria-label={value === undefined ? `${address}, blank` : `${address}: ${shown}`}
                        onClick={() => props.select(props.selected === anchor() ? null : anchor())}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            props.select(anchor());
                          } else if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
                            event.preventDefault();
                            const nextColumn = Math.max(1, column + (event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0));
                            const nextRow = Math.max(1, row.number + (event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0));
                            const nextAddress = `${columnName(nextColumn)}${nextRow}`;
                            props.select(`${current()?.anchor ?? ""}${nextAddress}`);
                            canvasRoot?.querySelector<HTMLElement>(`[data-cell="${nextAddress}"]`)?.focus();
                          }
                        }}
                      >{props.displayValues[`${current()?.anchor ?? ""}${address}`] ?? shown}</td>
                    );
                  }}</For>
                </tr>
              )}</For>
            </tbody>
          </table>
          <For each={chartBlocks()}>{(chart) => {
            const chartLabels = () => chart.rows.flatMap((row) => row.labels);
            const location = () => chartLabels().find((label) => label.startsWith("chart position: "))?.slice("chart position: ".length).split("!").pop() ?? "A1";
            const size = () => objectPositions()[location().toUpperCase()];
            const objectAnchor = () => `${current()?.anchor ?? ""}${location()}`;
            return <div class="office-sheet-object" style={{ left: size()?.left ?? "0px", top: size()?.top ?? "0px", width: size()?.width ?? "640px", height: size()?.height ?? "384px" }}>
              <OfficeChart anchor={chart.anchor} rows={chart.rows} placement={objectAnchor()} unitProps={(anchor) => ({ tabIndex: 0, "aria-current": props.selected === objectAnchor() ? "true" : undefined, "aria-label": `Chart at ${objectAnchor()}`, onClick: () => props.select(props.selected === objectAnchor() ? null : objectAnchor()) })} />
            </div>;
          }}</For>
          <For each={imageUnits()}>{(unit) => {
            const media = () => props.media.find((entry) => entry.alt_text === unit.text);
            const location = () => unit.labels.find((label) => label.startsWith("image position: "))?.split("!").pop() ?? media()?.cell ?? "A1";
            const size = () => objectPositions()[location().toUpperCase()];
            const objectAnchor = () => `${current()?.anchor ?? ""}${location()}`;
            return <div class="office-sheet-object" style={{ left: size()?.left ?? "0px", top: size()?.top ?? "0px", width: size()?.width ?? "190px", height: size()?.height ?? "140px" }}>
              <OfficeImage unit={unit} media={props.media} compact unitProps={() => ({ tabIndex: 0, "aria-current": props.selected === objectAnchor() ? "true" : undefined, "aria-label": `Image at ${objectAnchor()}: ${unit.text}`, onClick: () => props.select(props.selected === objectAnchor() ? null : objectAnchor()) })} />
            </div>;
          }}</For>
          </div>
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
