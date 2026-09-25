// Dev-only visual harness: exercises every registered card (semantic_type)
// renderer, across every real code branch its builder takes, against fixture
// output — no live vak-server, LLM provider, tool, or web search is involved.
// Runs the production client rendering path
// (assistantParts -> parseVakFence -> StructuredView -> STRUCTURED_RENDERERS)
// so a regression here is a regression a real chat turn would also hit.
import { createMemo, ErrorBoundary, For, Show, createSignal } from "solid-js";
import { AdaptiveTreeView, StructuredView, structuredRendererTypes } from "../components/PresentationRenderer";
import { assistantParts, groupAssistantParts } from "../structured";
import {
  CATEGORY_FIXTURES,
  TYPE_CATEGORY,
  REGRESSION_TEXT_FIXTURES,
  SCENARIO_FIXTURES,
  MULTI_CARD_FIXTURES,
  scenarioLooksUngrounded,
} from "./fixtures";
import type { Category, Fixture, ScenarioFixture, MultiCardFixture } from "./fixtures";
import type { StructuredOutput } from "../types";
import AgentMark from "../components/AgentMark";
import GenericSpecRenderer from "../components/presentation/GenericSpecRenderer";
import { AGENT_CHARACTERS, AGENT_CHARACTER_IDS } from "../agentGlyph";

// `?stress=1` pads every string in every fixture with long prose plus a long
// unbroken token, the shape of real model/tool output (titles, snippets,
// URLs) that short fixtures never exercise. Layout bugs where text escapes its
// container only show up with this kind of data.
const STRESS = new URLSearchParams(globalThis.location?.search ?? "").has("stress");
const LONG = " — a long descriptive continuation that a real model or tool result would plausibly produce, well past any single line";
const UNBROKEN = "https://example.com/a/very/long/unbroken/path/segment/that/has/no/spaces/at/all/0123456789";
function stress(value: unknown): unknown {
  if (typeof value === "string") {
    return /^https?:\/\//.test(value) ? value + UNBROKEN.slice(20) : value + LONG;
  }
  if (Array.isArray(value)) return value.map(stress);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, k === "semantic_type" || k === "chart_type" || k === "status" || k === "unit" ? v : stress(v)]));
  }
  return value;
}

function CardCell(props: { semanticType: string; payload: unknown; variantLabel: string }) {
  const [failed, setFailed] = createSignal(false);
  const output = createMemo<StructuredOutput>(() => ({
    semantic_type: props.semanticType,
    schema_version: 2,
    skill_id: "harness",
    skill_version: "1.0",
    payload: (STRESS ? stress(props.payload) : props.payload) as Record<string, unknown>,
  }));
  return (
    <div class="harness-cell" classList={{ "harness-cell-failed": failed() }}>
      <div class="harness-cell-head">
        <code>{props.semanticType}</code>
        <span class="harness-variant">{props.variantLabel}</span>
      </div>
      <ErrorBoundary
        fallback={(err) => {
          setFailed(true);
          return <div class="harness-error">CRASHED: {String(err)}</div>;
        }}
      >
        <StructuredView output={output()} fallback={JSON.stringify(props.payload)} />
      </ErrorBoundary>
    </div>
  );
}

function TextFixtureCell(props: { name: string; text: string; note: string }) {
  const parts = createMemo(() => assistantParts(props.text, false));
  const isBlank = createMemo(() => parts().length === 0);
  return (
    <div class="harness-cell" classList={{ "harness-cell-failed": isBlank() && props.text.trim() !== "" }}>
      <div class="harness-cell-head">
        <code>{props.name}</code>
      </div>
      <p class="harness-note">{props.note}</p>
      <Show when={isBlank() && props.text.trim() !== ""}>
        <div class="harness-error">Produced ZERO parts from non-empty input — this is the "total silence" regression.</div>
      </Show>
      <For each={parts()}>
        {(part) =>
          part.type === "text" ? (
            <pre class="harness-text-part">{part.text}</pre>
          ) : (
            <ErrorBoundary fallback={(err) => <div class="harness-error">CRASHED: {String(err)}</div>}>
              <StructuredView output={part.output} fallback={part.source} />
            </ErrorBoundary>
          )
        }
      </For>
    </div>
  );
}

function ScenarioCell(props: ScenarioFixture) {
  const parts = createMemo(() => assistantParts(props.text, false));
  const ungrounded = createMemo(() => scenarioLooksUngrounded(props));
  return (
    <div class="harness-cell" classList={{ "harness-cell-failed": ungrounded() }}>
      <div class="harness-cell-head">
        <code>{props.name}</code>
        <span class="harness-variant">expects: {props.expectation}</span>
      </div>
      <p class="harness-note"><em>Query:</em> "{props.query}"</p>
      <p class="harness-note">{props.note}</p>
      <Show when={ungrounded()}>
        <div class="harness-error">
          UNGROUNDED: response text has no vak-fence, no URL/citation, and no honest admission of missing data — but the
          scenario expects a grounded_card. This is the failure mode from the real bug report.
        </div>
      </Show>
      <For each={parts()}>
        {(part) =>
          part.type === "text" ? (
            <pre class="harness-text-part">{part.text}</pre>
          ) : (
            <ErrorBoundary fallback={(err) => <div class="harness-error">CRASHED: {String(err)}</div>}>
              <StructuredView output={part.output} fallback={part.source} />
            </ErrorBoundary>
          )
        }
      </For>
    </div>
  );
}

function MultiCardCell(props: MultiCardFixture) {
  const parts = createMemo(() => assistantParts(props.text, false));
  const groups = createMemo(() => groupAssistantParts(parts()));
  const actualCardCount = createMemo(() => parts().filter((p) => p.type === "card").length);
  const countMismatch = createMemo(() => actualCardCount() !== props.expectedCardCount);
  return (
    <div class="harness-cell" classList={{ "harness-cell-failed": countMismatch() }}>
      <div class="harness-cell-head">
        <code>{props.name}</code>
        <span class="harness-variant">expects {props.expectedCardCount} card(s), got {actualCardCount()}</span>
      </div>
      <p class="harness-note"><em>Query:</em> "{props.query}"</p>
      <p class="harness-note">{props.note}</p>
      <Show when={countMismatch()}>
        <div class="harness-error">
          CARD COUNT MISMATCH: fixture text should parse into {props.expectedCardCount} card(s) but produced {actualCardCount()}.
        </div>
      </Show>
      <For each={groups()}>
        {(group) =>
          group.type === "text" ? (
            <pre class="harness-text-part">{group.text}</pre>
          ) : group.cards.length === 1 ? (
            <ErrorBoundary fallback={(err) => <div class="harness-error">CRASHED: {String(err)}</div>}>
              <StructuredView output={group.cards[0].output} fallback={group.cards[0].source} />
            </ErrorBoundary>
          ) : (
            <div class="card-group" style={{ "--card-group-count": group.cards.length }}>
              <For each={group.cards}>
                {(card) => (
                  <div class="card-group-item">
                    <ErrorBoundary fallback={(err) => <div class="harness-error">CRASHED: {String(err)}</div>}>
                      <StructuredView output={card.output} fallback={card.source} />
                    </ErrorBoundary>
                  </div>
                )}
              </For>
            </div>
          )
        }
      </For>
    </div>
  );
}

// Self-check: every semantic_type actually registered in
// STRUCTURED_RENDERERS must appear in TYPE_CATEGORY (fixtures.ts), or the
// harness would silently skip testing it. This is surfaced in the UI, not
// just console — a missing mapping is exactly the kind of coverage gap that
// makes "we tested all card types" a false claim.
function coverageGaps(): string[] {
  return structuredRendererTypes.filter((t) => !TYPE_CATEGORY[t]);
}

export default function CardHarness() {
  const types = structuredRendererTypes;
  const [filter, setFilter] = createSignal("");
  const filtered = createMemo(() => types.filter((t) => t.includes(filter().toLowerCase())));
  const gaps = createMemo(coverageGaps);

  const totalCards = createMemo(() =>
    types.reduce((sum, t) => {
      const categories = TYPE_CATEGORY[t] ?? [];
      return sum + categories.reduce((s, c) => s + (CATEGORY_FIXTURES[c]?.length ?? 0), 0);
    }, 0),
  );

  return (
    <div class="harness-root">
      <header class="harness-header">
        <h1>Card Render Harness</h1>
        <p>
          Exercises fixture output as raw <code>vak</code> fences through the production client rendering path
          (<code>assistantParts</code> → <code>parseVakFence</code> → <code>StructuredView</code>). No live backend, LLM
          provider, tool, or web search call is made; this is visual renderer coverage, not an end-to-end pipeline test.
          Every fixture below targets a genuinely distinct branch inside the
          renderer's own <code>build*Spec</code> functions (columns/rows vs. pros/cons vs. left/right for tables;
          object vs. tuple chart points; string vs. object recipe steps; etc.) — not one payload reused everywhere.
          {" "}{types.length} registered semantic types → {totalCards()} total permutation cards below.
        </p>
        <Show when={gaps().length > 0}>
          <div class="harness-error">
            COVERAGE GAP: {gaps().length} registered type(s) have no fixture mapping and are NOT being tested:{" "}
            {gaps().join(", ")}
          </div>
        </Show>
        <input
          type="text"
          placeholder="Filter semantic_type…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
      </header>

      <section>
        <h2>Selected pack projections</h2>
        <p class="harness-section-note">Compiled pack trees can differ from the direct structured-card adapter. This fixture uses the root shape produced by the built-in metric pack for a multi-reading result.</p>
        <div class="harness-grid">
          <div class="harness-cell">
            <div class="harness-cell-head"><code>seed.metric</code><span class="harness-variant">emitted multi-reading payload</span></div>
            <GenericSpecRenderer node={{ primitive: "metric", props: { label: "Noida now", condition: "Sunny", temperature: "35.2°C", humidity: "31%" }, children: [] }} />
          </div>
          <div class="harness-cell">
            <div class="harness-cell-head"><code>seed.travel-options</code><span class="harness-variant">selected table with owner choice</span></div>
            <AdaptiveTreeView
              tree={{ schema_version: 1, spec_id: "seed.travel-options", revision: 6, digest: "harness", root: { primitive: "table", props: { title: "Saturday choices", columns: [{ key: "option", label: "Option" }, { key: "fit", label: "Fit" }] }, children: [{ primitive: "row", props: { option: "Museum", fit: "Indoor" }, children: [] }, { primitive: "row", props: { option: "Garden", fit: "Outdoors" }, children: [] }] }, accessibility_summary: "Saturday choices", coverage: { rendered_paths: [], omitted_paths: [] } }}
              fallback="Saturday choices"
              sessionId="harness-session"
              resultId="harness-result"
            />
          </div>
        </div>
      </section>

      <section>
        <h2>Agent character package</h2>
        <p class="harness-section-note">
          Canonical Vakyartha mascot and all seven built-in companions at compact, conversation, and promotional sizes.
          The final portrait in each row exercises the working-state motion.
        </p>
        <div class="harness-character-grid">
          <For each={AGENT_CHARACTER_IDS}>{(id) => (
            <article class="harness-character-card">
              <div class="harness-character-sizes">
                <AgentMark character={id} size={24} />
                <AgentMark character={id} size={40} />
                <AgentMark character={id} size={88} state="working" />
              </div>
              <strong>{AGENT_CHARACTERS[id].name}</strong>
              <span>{AGENT_CHARACTERS[id].kind}</span>
              <small>{AGENT_CHARACTERS[id].personality}</small>
            </article>
          )}</For>
        </div>
      </section>

      <section>
        <h2>Real-world scenario simulations (groundedness, not just render-safety)</h2>
        <p class="harness-section-note">
          These replay actual observed model behavior — including the verified real bug where a tool call succeeded but
          the model answered with ungrounded prose anyway. Phrased generically (any retrieval/search-type tool, not
          just Tavily) since the failure mode isn't provider-specific. A card can render perfectly and still be a bad
          answer; this section flags that case specifically.
        </p>
        <div class="harness-grid harness-grid-wide">
          <For each={SCENARIO_FIXTURES}>{(s) => <ScenarioCell {...s} />}</For>
        </div>
      </section>

      <section>
        <h2>Multi-card composition (analysis, dashboards, reporting, writing)</h2>
        <p class="harness-section-note">
          Most answers need exactly one card, but some genuinely need several — a dashboard-style report, an analysis
          plus its supporting data, a postmortem combining a timeline with real command output and test results. These
          exercise the grid layout (<code>groupAssistantParts</code> → <code>.card-group</code>) that multiple adjacent
          `vak` fences in one answer now render into, instead of unrelated stacked blocks.
        </p>
        <div class="harness-grid harness-grid-wide">
          <For each={MULTI_CARD_FIXTURES}>{(f) => <MultiCardCell {...f} />}</For>
        </div>
      </section>

      <section>
        <h2>Regression fixtures (malformed / empty fences)</h2>
        <div class="harness-grid harness-grid-wide">
          <For each={REGRESSION_TEXT_FIXTURES}>{(f) => <TextFixtureCell {...f} />}</For>
        </div>
      </section>

      <section>
        <h2>All registered card types × every real branch of their renderer</h2>
        <div class="harness-grid">
          <For each={filtered()}>
            {(semanticType) => {
              const categories = TYPE_CATEGORY[semanticType] ?? [];
              const fixtures: { category: Category; fixture: Fixture }[] = categories.flatMap((c) =>
                (CATEGORY_FIXTURES[c] ?? []).map((fixture) => ({ category: c, fixture })),
              );
              return (
                <For each={fixtures}>
                  {({ category, fixture }) => (
                    <CardCell
                      semanticType={semanticType}
                      payload={fixture.payload}
                      variantLabel={`[${category}] ${fixture.label}`}
                    />
                  )}
                </For>
              );
            }}
          </For>
        </div>
      </section>
    </div>
  );
}
