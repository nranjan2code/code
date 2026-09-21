// Fixture payloads for exercising every registered card (`semantic_type`)
// renderer against the REAL code branches its builder takes — not one
// generic "kitchen sink" object reused everywhere.
//
// Every `semantic_type` in STRUCTURED_RENDERERS (PresentationRenderer.tsx)
// routes through exactly one of 13 `build*Spec` functions in
// GenericSpecRenderer.tsx. Each of those functions has several genuinely
// different internal branches driven by which fields are present (e.g.
// buildTableSpec has 5 distinct paths: explicit columns/rows, pros/cons,
// left/right comparison, bare array-of-objects, and flat key/value
// fallback). CARD_CATEGORY_FIXTURES below enumerates a payload PER BRANCH
// per builder, so "permutation coverage" means what it should: every code
// path in the renderer gets exercised, for every semantic_type that routes
// through it — not the same one shape copy-pasted 106 times.
//
// TYPE_CATEGORY maps every registered semantic_type to the category (builder)
// it actually uses, derived directly from PresentationRenderer.tsx's
// STRUCTURED_RENDERERS registry (kept in sync by the "categories must cover
// every registered type" self-check in CardHarness.tsx).

export type Category =
  | "universal_card"
  | "research"
  | "diff"
  | "test_matrix"
  | "terminal"
  | "table"
  | "timeline"
  | "recipe"
  | "ui_preview"
  | "chart"
  | "media"
  | "metric";

export interface Fixture {
  label: string;
  payload: unknown;
}

// --- Category -> distinct payload variants, one per real branch ----------

const universalCardFixtures: Fixture[] = [
  { label: "title + summary + rich entries", payload: { title: "Q3 Roadmap", summary: "Cross-team plan for the quarter.", owner: "Platform", status: "on_track", due: "2026-12-31" } },
  { label: "title only, no other entries", payload: { title: "Untitled Card" } },
  { label: "entries only, no title/summary", payload: { region: "us-east-1", tier: "gold", capacity: 82 } },
  { label: "empty {}", payload: {} },
];

const researchFixtures: Fixture[] = [
  {
    label: "sources[] + takeaways[] with citation_indices (grounded — correct shape)",
    payload: {
      title: "Grounded Research",
      sources: [
        { title: "Primary Source", url: "https://example.com/a", snippet: "Direct quote.", source_name: "Example Daily", published_at: "2026-09-18" },
        { title: "Secondary Source", url: "https://example.com/b" },
      ],
      takeaways: [
        { text: "First finding, tied to source 1.", citation_indices: [0] },
        { text: "Second finding, tied to source 2.", citation_indices: [1] },
      ],
    },
  },
  {
    label: "alt field names: references/citations + findings/points",
    payload: {
      references: [{ title: "Alt-named source", url: "https://example.com/c" }],
      findings: ["Plain string finding one.", "Plain string finding two."],
    },
  },
  {
    label: "string-only sources (bare URLs)",
    payload: { citations: ["https://example.com/d", "https://example.com/e"], items: ["Takeaway from bare-url sources."] },
  },
  {
    label: "UNGROUNDED: takeaways present, zero sources (the real observed bug shape)",
    payload: { title: "Ungrounded Synthesis", takeaways: ["Vague generic claim with nothing backing it."], sources: [] },
  },
  { label: "empty {}", payload: {} },
];

const diffFixtures: Fixture[] = [
  {
    label: "files[] with hunks",
    payload: { files: [{ filename: "src/example.ts", additions: 4, deletions: 1, hunks: "@@ -1,3 +1,6 @@\n-old\n+new\n+another" }] },
  },
  { label: "raw patch string, no files[]", payload: { diff: "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\n", filename: "f.txt" } },
  { label: "malformed files (not an array)", payload: { files: "not-an-array" } },
  { label: "empty {}", payload: {} },
];

const testMatrixFixtures: Fixture[] = [
  {
    label: "tests[] with explicit suite counts",
    payload: { suite_name: "unit", total: 3, passed: 2, failed: 1, skipped: 0, tests: [{ name: "a", status: "passed", duration_ms: 4 }, { name: "b", status: "passed" }, { name: "c", status: "failed", message: "boom" }] },
  },
  { label: "alt field names: cases[]/results[], counts absent (must derive)", payload: { cases: [{ test: "x", result: "passed" }, { test: "y", result: "failed" }] } },
  { label: "string-only test names", payload: { tests: ["renders ok", "handles edge case"] } },
  { label: "tests missing/null entirely", payload: { suite_name: "empty-suite", tests: null } },
];

const terminalFixtures: Fixture[] = [
  { label: "command + output + exit_code + duration_ms", payload: { command: "npm run build", output: "Build succeeded", exit_code: 0, duration_ms: 1200 } },
  { label: "alt field names: cmd/stdout", payload: { cmd: "ls -la", stdout: "total 0\ndrwxr-xr-x  2 user  staff  64 Jan 1 00:00 ." } },
  { label: "no exit_code (must show 'unavailable')", payload: { output: "still running…" } },
  { label: "empty {}", payload: {} },
];

const tableFixtures: Fixture[] = [
  { label: "explicit columns[] + rows[]", payload: { title: "Benchmark", columns: [{ key: "name", label: "Name" }, { key: "score", label: "Score", isNumeric: true }], rows: [{ name: "A", score: 92 }, { name: "B", score: 81 }] } },
  { label: "string columns[] + array rows[]", payload: { title: "Weekend options", columns: ["Option", "Cost", "Weather fit", "Effort"], rows: [["Indoor food trail", "₹₹", "Excellent", "Low"], ["Museum and garden", "₹", "Good", "Medium"]] } },
  { label: "pros[] / cons[]", payload: { title: "Tradeoffs", pros: ["Fast", "Cheap"], cons: ["Less accurate"] } },
  { label: "left/right comparison objects", payload: { title: "Plan A vs B", left_label: "Plan A", right_label: "Plan B", left: { latency_ms: 120, cost: "$$" }, right: { latency_ms: 340, cost: "$" } } },
  { label: "bare array of row objects (columns inferred from keys)", payload: { rows: [{ sku: "A1", qty: 3, price: 9.99 }, { sku: "B2", qty: 1, price: 14.5 }] } },
  { label: "flat key/value fallback (no rows/columns/pros/cons/left/right)", payload: { revenue: 120000, expenses: 98000, margin: "18%" } },
  { label: "empty {}", payload: {} },
];

const timelineFixtures: Fixture[] = [
  { label: "items[] rich objects with label/detail/status", payload: { title: "Rollout Plan", items: [{ label: "Kickoff", detail: "Initial step", status: "done" }, { label: "In progress", detail: "Current step", status: "active" }, { label: "Wrap up", status: "pending" }] } },
  { label: "alt array field: steps[] of plain strings", payload: { steps: ["Draft proposal", "Get sign-off", "Ship"] } },
  { label: "alt array field: milestones[] with question/task/activity keys", payload: { milestones: [{ task: "Design review" }, { activity: "Launch" }] } },
  { label: "noncoding options[] stay readable as choices", payload: { title: "Ways to spend the afternoon", options: [{ name: "Walk in the park", description: "Low cost and flexible" }, { name: "Visit a museum", description: "Indoor option" }] } },
  { label: "no matching array field — falls back to flat key/value as items", payload: { owner: "Platform team", due: "2026-12-31", status: "on track" } },
  { label: "empty {}", payload: {} },
];

const recipeFixtures: Fixture[] = [
  {
    label: "ingredients[]/steps[] as rich objects with timers",
    payload: { title: "Pancakes", servings: 4, prep_time_minutes: 10, cook_time_minutes: 20, ingredients: [{ name: "Flour", amount: 2, unit: "cups" }, { name: "Egg", amount: 1 }], steps: [{ text: "Mix dry ingredients", timer_minutes: 2 }, { text: "Cook on griddle" }] },
  },
  { label: "string-only ingredients/steps", payload: { title: "Toast", ingredients: ["Bread", "Butter"], steps: ["Toast the bread", "Spread butter"] } },
  { label: "alt field names: items[] for ingredients, instructions[] for steps", payload: { items: ["Pasta", "Sauce"], instructions: ["Boil pasta", "Add sauce"] } },
  { label: "null/no ingredients or steps at all", payload: { title: "Empty Recipe" } },
  { label: "malformed field types", payload: { title: 12345, ingredients: "not-an-array", steps: { not: "an array" } } },
];

const uiPreviewFixtures: Fixture[] = [
  { label: "artifact_path + status + title", payload: { status: "ready", title: "Preview", artifact_path: "/tmp/preview/index.html" } },
  { label: "inline html", payload: { status: "ready", html: "<p>inline preview</p>" } },
  { label: "empty {} — must show 'Preview file is unavailable'", payload: {} },
];

const chartFixtures: Fixture[] = [
  { label: "multi-series, object points {x,y}", payload: { title: "Weekly Trend", x_label: "Day", y_label: "Value", series: [{ name: "Series A", points: [{ x: "Mon", y: 10 }, { x: "Tue", y: 14 }, { x: "Wed", y: 9 }] }, { name: "Series B", points: [{ x: "Mon", y: 3 }, { x: "Tue", y: 5 }] }] } },
  { label: "single series, tuple points [x,y]", payload: { series: [{ name: "Series A", points: [[0, 3], [1, 5], [2, 4]] }] } },
  { label: "chart_type override present", payload: { chart_type: "area", series: [{ name: "A", points: [{ x: 1, y: 2 }] }] } },
  { label: "empty series[] — must show 'No data points supplied.'", payload: { series: [] } },
  { label: "series present but malformed points", payload: { series: [{ name: "A", points: "not-an-array" }] } },
];

const mediaFixtures: Fixture[] = [
  { label: "link: full url/image_url/title/description/site_name", payload: { url: "https://example.com", image_url: "https://example.com/preview.png", title: "Example Site", description: "A simulated link preview.", site_name: "example.com" } },
  { label: "image/video/audio: source + alt", payload: { source: "https://example.com/photo.jpg", alt: "Simulated photo" } },
  { label: "url is non-string (no src should render)", payload: { url: 12345, title: "Bad URL type" } },
  { label: "empty {}", payload: {} },
];

const metricFixtures: Fixture[] = [
  { label: "single label/value/unit", payload: { label: "Uptime", value: 99.9, unit: "%" } },
  { label: "metric_grid: multiple key/value entries, no single value", payload: { location: "Data Center A", cpu: 42, memory: "68%", disk: "12%" } },
  { label: "empty {} — must show '—' placeholder, not blank", payload: {} },
];

export const CATEGORY_FIXTURES: Record<Category, Fixture[]> = {
  universal_card: universalCardFixtures,
  research: researchFixtures,
  diff: diffFixtures,
  test_matrix: testMatrixFixtures,
  terminal: terminalFixtures,
  table: tableFixtures,
  timeline: timelineFixtures,
  recipe: recipeFixtures,
  ui_preview: uiPreviewFixtures,
  chart: chartFixtures,
  media: mediaFixtures,
  metric: metricFixtures,
};

// Every semantic_type currently registered in STRUCTURED_RENDERERS
// (PresentationRenderer.tsx), mapped to the category/builder it actually
// uses. `coding.search`, `travel_options`, and `meal_plan` are
// data-shape-conditional in the real renderer (they pick between two
// builders depending on what fields are present) — listed under BOTH
// categories they can resolve to, since a harness that only tests one path
// of a conditional isn't testing the conditional at all.
export const TYPE_CATEGORY: Record<string, Category[]> = {
  // universal.* card kinds
  map: ["universal_card"], route_map: ["universal_card"], calendar: ["universal_card"], availability: ["universal_card"],
  board: ["universal_card"], entity: ["universal_card"], search_results: ["universal_card"], evidence: ["universal_card"],
  decision_analysis: ["universal_card"], document: ["universal_card"], graph: ["universal_card"], form: ["universal_card"],
  action: ["universal_card"], transaction: ["universal_card"], alert: ["universal_card"], conversation: ["universal_card"],
  progress_dashboard: ["universal_card"], simulation: ["universal_card"],
  // research
  "research.synthesis": ["research"], research_brief: ["research"], research: ["research"], news: ["research"],
  // diff
  "coding.diff": ["diff"], diff: ["diff"],
  // test matrix
  "test.report": ["test_matrix"], test: ["test_matrix"], "ci.test_matrix": ["test_matrix"], test_matrix: ["test_matrix"],
  // terminal
  "terminal.view": ["terminal"], terminal: ["terminal"], "terminal.session": ["terminal"],
  // table
  "coding.benchmark": ["table"], "coding.dependencies": ["table"], "coding.search": ["table", "research"],
  "data.grid": ["table"], table: ["table"], dataframe: ["table"], dataset: ["table"], comparison: ["table"],
  comparison_table: ["table"], pros_cons: ["table"], inventory: ["table"], scorecard: ["table"], decision_matrix: ["table"],
  criteria_matrix: ["table"], tradeoff_analysis: ["table"], budget: ["table"], finance_summary: ["table"], invoice_summary: ["table"],
  travel_options: ["table", "timeline"],
  // recipe
  "recipe.card": ["recipe"], recipe: ["recipe"], "lifestyle.recipe": ["recipe"], "lifestyle.culinary_recipe": ["recipe"],
  recipe_summary: ["recipe"], meal_plan: ["recipe", "timeline"],
  // ui preview
  "ui.preview": ["ui_preview"], preview: ["ui_preview"],
  // timeline (large group)
  "coding.deployment": ["timeline"], "coding.incident": ["timeline"], "coding.architecture": ["timeline"], "coding.release": ["timeline"],
  "plan.timeline": ["timeline"], timeline: ["timeline"], itinerary: ["timeline"], checklist: ["timeline"], schedule: ["timeline"],
  agenda: ["timeline"], milestones: ["timeline"], progress: ["timeline"], status: ["timeline"], steps: ["timeline"],
  overview: ["timeline"], summary: ["timeline"], detail: ["timeline"], notes: ["timeline"], follow_up: ["timeline"],
  reminder: ["timeline"], shopping_list: ["timeline"], lesson: ["timeline"], reading_list: ["timeline"], habit_plan: ["timeline"],
  project_plan: ["timeline"], meeting_notes: ["timeline"], contact_log: ["timeline"], home_project: ["timeline"],
  care_plan: ["timeline"], event_plan: ["timeline"], media_list: ["timeline"], collection: ["timeline"], faq: ["timeline"],
  decision: ["timeline"],
  // chart
  chart: ["chart"], "telemetry.chart": ["chart"], trend: ["chart"], timeseries: ["chart"], metric_chart: ["chart"],
  bar_chart: ["chart"], comparison_chart: ["chart"],
  // media
  "link.preview": ["media"], "media.image": ["media"], "media.video": ["media"], "media.audio": ["media"],
  // metric
  weather: ["metric"], "telemetry.metric": ["metric"], metric: ["metric"],
};

export function fenceFor(semanticType: string, payload: unknown): string {
  return "```vak\n" + JSON.stringify({
    semantic_type: semanticType,
    schema_version: 2,
    skill_id: "harness",
    skill_version: "1.0",
    payload,
  }) + "\n```";
}

// Regression fixtures for the two previously-fixed bugs, run through
// `assistantParts()` directly rather than `StructuredView`.
export const REGRESSION_TEXT_FIXTURES: { name: string; text: string; note: string }[] = [
  {
    name: "truncated/malformed vak fence (non-streaming)",
    text: "Here is what I found:\n\n```vak\n{\"semantic_type\": \"research.synthesis\", \"payload\": {\"title\": \"Oops\", \n```\n",
    note: "Must NOT render as total silence — should show the 'could not be rendered' fallback text (fixed in 3.4.4).",
  },
  {
    name: "vak-tagged fence with invalid JSON body",
    text: "```vak\nnot even json { \n```",
    note: "Same fallback-text contract as above, for a body that never parses at all.",
  },
  {
    name: "well-formed json fence without vak tag but with semantic_type",
    text: "```json\n{\"semantic_type\": \"metric\", \"payload\": {\"label\": \"Uptime\", \"value\": 99.9, \"unit\": \"%\"}}\n```",
    note: "Implicit json-fence detection path — should render as a card, not text.",
  },
  {
    name: "mismatched row closer plus one stray closing brace",
    text: "```vak\n{\"semantic_type\":\"travel_options\",\"payload\":{\"title\":\"Saturday options\",\"columns\":[\"Option\",\"Cost\"],\"rows\":[[\"Park\",\"Low\"},{\"Crafts\",\"Low\"]]}}\n}\n```",
    note: "Observed from a real local-model response. A delimiter may be repaired only when the nesting stack proves the intended closer; model-supplied keys and values remain unchanged.",
  },
  {
    name: "empty document (no blocks, no source text)",
    text: "",
    note: "Should surface a 'no result' notice rather than a blank turn (fixed in 3.4.3). Verified separately against PresentationDocumentView, not assistantParts.",
  },
];

// --- Real-world scenario simulations (groundedness, not render-safety) ---
// Verified against the live session transcript
// (~/Library/Application Support/vak/agents/vak/sessions/68fcfec7b0da0cd4/
// 01a0b41f-....jsonl), not guessed from source: a `tavily_search` MCP tool
// call succeeded and returned real, dated, specific results; the model's
// final answer ignored that tool_result and wrote vague, unsourced prose,
// with no vak-fence, no citations, and a false claim of "already" having
// answered.
//
// This class of bug is NOT specific to Tavily or to search. Any MCP/tool
// call whose result is meant to ground a structured card — a database
// query, a file read, a different search provider (Exa/Firecrawl/whatever
// gets added next), a code-search tool — can suffer the same failure: tool
// succeeds, model ignores the result, answer renders fine but is
// unsupported. So scenarios below are phrased generically ("a retrieval
// tool call succeeded") rather than hardcoding "tavily".
export interface ScenarioFixture {
  name: string;
  query: string;
  text: string;
  expectation: "grounded_card" | "explicit_no_data";
  note: string;
}

export const SCENARIO_FIXTURES: ScenarioFixture[] = [
  {
    name: "retrieval tool succeeded, model ignored the tool result (verified real bug)",
    query: "what are top news in india right now",
    text:
      "I have already provided a summary of the top news in India based on the latest search results. Here is a synthesis of those key topics for you:\n\n" +
      "**Key Themes in Indian News Right Now:**\n\n" +
      "1. **Political & Regulatory Updates:** Opposition parties are raising concerns over proposed financial policies (like UPI payment fees), and the Supreme Court is addressing important regulatory matters.\n" +
      "2. **Geopolitics & Industry:** India is balancing its foreign relations while pushing for domestic strengths.\n\n" +
      "For the most detailed, up-to-the-minute information, I recommend checking live news aggregators. However, this summary reflects the major trending topics.",
    expectation: "grounded_card",
    note:
      "Reproduces the exact response from the live session transcript (model: gemma4:e2b-mlx, tool: tavily_search — but the failure mode generalizes to ANY retrieval-type tool). The tool call actually ran and returned real, dated headlines; the model had everything it needed and still answered with vague, unsourced prose, no vak-fence, and a false claim of having 'already' answered.",
  },
  {
    name: "retrieval tool result correctly synthesized (what correct looks like)",
    query: "what are top news in india right now",
    text:
      "Here is what I found:\n\n" +
      fenceFor("research.synthesis", {
        title: "Top News in India — Sept 18, 2026",
        as_of: "2026-09-18T18:00:00Z",
        sources: [
          { title: "Parliament debates UPI fee proposal", url: "https://example-news.in/upi-fee-debate", source_name: "Example News India", published_at: "2026-09-18" },
          { title: "RBI holds rates steady amid inflation watch", url: "https://example-news.in/rbi-rates", source_name: "Example Business Daily", published_at: "2026-09-18" },
        ],
        takeaways: [
          { text: "Parliament is debating a proposed UPI transaction fee, opposed by several state governments.", citation_indices: [0] },
          { text: "RBI held its policy rate steady, citing inflation risk from global oil prices.", citation_indices: [1] },
        ],
      }),
    expectation: "grounded_card",
    note: "Real citations, a real 'as of' timestamp, and a rendered card the user can click through — this is the bar the bad case above fails to meet.",
  },
  {
    name: "retrieval tool available, model skipped the fence anyway (partial-regression shape)",
    query: "analyze our top 5 competitors' pricing pages and summarize the differences",
    text: "Looking at the competitor pages, pricing generally clusters around tiered plans with usage-based add-ons, and most offer an enterprise tier with custom pricing.",
    expectation: "grounded_card",
    note: "Simulates the same failure class outside news: a competitive-analysis query where a fetch/crawl tool ran, but the answer is a vague paraphrase with no per-competitor sourcing — no card, no citations, for a query that demands per-source grounding.",
  },
  {
    name: "explicit, honest 'no data available' answer (acceptable alternative to a card)",
    query: "what's our current AWS spend by service this month",
    text:
      "I don't have a working retrieval tool connected for billing data right now, so I can't pull this month's actual AWS spend — anything I say would be a guess, not real numbers. " +
      "Connect a billing/cost integration, or paste the export and I'll analyze it.",
    expectation: "explicit_no_data",
    note: "This is what the bad case above SHOULD have said instead of hedging with vague generic 'themes'. Honest and actionable beats confidently vague — and this should NOT be flagged, since no tool result is being silently dropped.",
  },
];

// --- Multi-card composition scenarios -------------------------------------
// A single answer often genuinely needs more than one card — an analysis
// alongside its supporting data, a dashboard-style report combining a
// metric overview with a chart and a table, a plan with both a timeline and
// a budget. `assistantParts()` already loops over every `vak` fence in one
// message; `groupAssistantParts()` (structured.ts) then groups adjacent
// cards into one `card_group` render unit, laid out as a responsive grid
// (see `.card-group` in styles.css) instead of unrelated stacked blocks.
// These fixtures exercise that path directly — deliberately spanning
// analysis, dashboards, reporting, and writing, not just news/weather.
export interface MultiCardFixture {
  name: string;
  query: string;
  text: string;
  expectedCardCount: number;
  note: string;
}

export const MULTI_CARD_FIXTURES: MultiCardFixture[] = [
  {
    name: "quarterly business review: metric overview + trend chart + budget table",
    query: "give me a quarterly business review for the platform team",
    text:
      "Here's the Q3 review:\n\n" +
      fenceFor("metric", { title: "Q3 Snapshot", location: "Platform", uptime: "99.95%", active_users: "48,200", mrr: "$412K" }) +
      "\n\n" +
      fenceFor("chart", {
        title: "MRR Trend",
        chart_type: "line",
        x_label: "Month",
        y_label: "MRR ($K)",
        series: [{ name: "MRR", points: [{ x: "Jul", y: 360 }, { x: "Aug", y: 388 }, { x: "Sep", y: 412 }] }],
      }) +
      "\n\n" +
      fenceFor("budget", {
        title: "Q3 Spend Breakdown",
        columns: [{ key: "category", label: "Category" }, { key: "amount", label: "Amount", isNumeric: true }],
        rows: [{ category: "Infrastructure", amount: 82000 }, { category: "Headcount", amount: 210000 }, { category: "Tooling", amount: 14000 }],
      }),
    expectedCardCount: 3,
    note: "Dashboard-style report: one card each for the current-state metrics, the trend over time, and the underlying budget — genuinely distinct data, not one card padded with everything.",
  },
  {
    name: "competitive analysis: research synthesis + comparison table",
    query: "compare our pricing against the top 3 competitors",
    text:
      "Here's the analysis:\n\n" +
      fenceFor("research.synthesis", {
        title: "Competitive Pricing Analysis",
        sources: [
          { title: "Competitor A pricing page", url: "https://competitor-a.example.com/pricing" },
          { title: "Competitor B pricing page", url: "https://competitor-b.example.com/pricing" },
        ],
        takeaways: [
          { text: "Competitor A undercuts us on the entry tier by 20%.", citation_indices: [0] },
          { text: "Competitor B has no free tier, unlike us and Competitor A.", citation_indices: [1] },
        ],
      }) +
      "\n\n" +
      fenceFor("comparison_table", {
        title: "Entry-Tier Pricing",
        columns: [{ key: "vendor", label: "Vendor" }, { key: "price", label: "Price/mo", isNumeric: true }, { key: "free_tier", label: "Free Tier" }],
        rows: [{ vendor: "Us", price: 29, free_tier: "Yes" }, { vendor: "Competitor A", price: 23, free_tier: "Yes" }, { vendor: "Competitor B", price: 35, free_tier: "No" }],
      }),
    expectedCardCount: 2,
    note: "Analysis scenario: cited findings (research.synthesis) plus the structured numbers those findings are based on (comparison_table) — connected, not merged into one overloaded card.",
  },
  {
    name: "incident postmortem writing: timeline + terminal output + test report",
    query: "write up the postmortem for last night's deploy incident",
    text:
      "## Incident Postmortem\n\nHere's what happened, in order:\n\n" +
      fenceFor("coding.incident", {
        title: "Deploy Incident — Sept 17",
        items: [
          { label: "22:14 UTC", detail: "Deploy triggered rollback loop", status: "detected" },
          { label: "22:19 UTC", detail: "On-call paged, rollback manually halted", status: "mitigated" },
          { label: "22:41 UTC", detail: "Root cause identified: missing migration guard", status: "resolved" },
        ],
      }) +
      "\n\nThe failing command during triage:\n\n" +
      fenceFor("terminal.view", { command: "kubectl rollout status deploy/api", output: "error: deployment \"api\" exceeded its progress deadline", exit_code: 1 }) +
      "\n\nRegression tests added as a follow-up:\n\n" +
      fenceFor("test.report", {
        suite_name: "migration-guard-regression",
        tests: [{ name: "rejects deploy without guard", status: "passed" }, { name: "allows deploy with guard", status: "passed" }],
      }),
    expectedCardCount: 3,
    note: "Writing/reporting scenario: a written narrative (prose + timeline) grounded by the actual failing command output and the regression tests that followed — three genuinely different artifact types in one coherent report.",
  },
  {
    name: "single-card control: a plain metric answer must NOT be wrapped as a group",
    query: "what's our current uptime",
    text: "Here you go:\n\n" + fenceFor("metric", { label: "Uptime", value: 99.95, unit: "%" }),
    expectedCardCount: 1,
    note: "Negative control — most answers need exactly one card. A lone card must render as a normal single card, not forced into a group layout with grid styling for no reason.",
  },
];

function containsCitations(text: string): boolean {
  return /https?:\/\//.test(text) || text.includes('"sources"') || text.includes('"citation_indices"');
}

// A scenario "fails" the groundedness check when it expects a grounded card
// but the raw text is neither a card nor an honest admission of missing data.
export function scenarioLooksUngrounded(s: ScenarioFixture): boolean {
  if (s.expectation !== "grounded_card") return false;
  const hasFence = s.text.includes('"semantic_type"');
  const admitsNoData = /don't have|do not have|no (?:live |real-?time |working )?(?:web[- ]search|search tool|retrieval tool|access to)/i.test(s.text);
  return !hasFence && !containsCitations(s.text) && !admitsNoData;
}
