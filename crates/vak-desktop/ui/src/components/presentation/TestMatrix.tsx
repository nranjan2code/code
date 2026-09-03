import { For, Show, createMemo, createSignal } from "solid-js";

export interface TestCase {
  name: string;
  status: "passed" | "failed" | "skipped";
  duration_ms?: number;
  message?: string;
  traceback?: string;
}

export interface TestSuiteData {
  suite_name?: string;
  total?: number;
  passed?: number;
  failed?: number;
  skipped?: number;
  duration_ms?: number;
  tests: TestCase[];
}

export default function TestMatrix(props: { data: TestSuiteData }) {
  const [filter, setFilter] = createSignal<"all" | "failed">("all");

  const passedCount = createMemo(() => {
    return props.data.passed ?? props.data.tests.filter((t) => t.status === "passed").length;
  });

  const failedCount = createMemo(() => {
    return props.data.failed ?? props.data.tests.filter((t) => t.status === "failed").length;
  });

  const totalCount = createMemo(() => {
    return props.data.total ?? props.data.tests.length;
  });

  const successPercent = createMemo(() => {
    const tot = totalCount();
    if (tot === 0) return 100;
    return Math.round((passedCount() / tot) * 100);
  });

  const filteredTests = createMemo(() => {
    if (filter() === "failed") {
      return props.data.tests.filter((t) => t.status === "failed");
    }
    return props.data.tests;
  });

  return (
    <div class="canvas-card test-matrix-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class={`card-badge ${failedCount() > 0 ? "badge-rose" : "badge-emerald"}`}>
            Test Suite
          </span>
          <span class="card-subtitle">
            {props.data.suite_name ?? "Test Execution"} ({props.data.duration_ms ?? 0}ms)
          </span>
        </div>
        <div class="test-tab-filter">
          <button
            class="filter-chip"
            classList={{ active: filter() === "all" }}
            onClick={() => setFilter("all")}
          >
            All ({totalCount()})
          </button>
          <button
            class="filter-chip"
            classList={{ active: filter() === "failed" }}
            onClick={() => setFilter("failed")}
          >
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
                  stroke="rgba(244, 63, 94, 0.25)"
                  stroke-width="3.5"
                />
                <path
                  d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                  fill="none"
                  stroke="var(--emerald-bright)"
                  stroke-width="3.5"
                  stroke-dasharray={`${successPercent()}, 100`}
                  stroke-linecap="round"
                />
              </svg>
            </div>
            <div>
              <div style={{ "font-size": "14px", "font-weight": "700", color: "var(--text-main)" }}>
                {successPercent()}% Success Rate
              </div>
              <div style={{ "font-size": "11.5px", color: "var(--text-muted)" }}>
                {passedCount()} of {totalCount()} tests verified
              </div>
            </div>
          </div>
          <div style={{ display: "flex", gap: "14px", "font-size": "12px", "font-weight": "600" }}>
            <span style={{ color: "var(--emerald-bright)" }}>● {passedCount()} Passed</span>
            <Show when={failedCount() > 0}>
              <span style={{ color: "var(--rose-bright)" }}>✕ {failedCount()} Failed</span>
            </Show>
          </div>
        </div>

        <div class="test-grid-list">
          <For each={filteredTests()}>
            {(t) => (
              <div class="test-item-card" classList={{ "fail-card": t.status === "failed" }}>
                <div style={{ width: "100%" }}>
                  <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
                    <div>
                      <span
                        style={{
                          color: t.status === "failed" ? "var(--rose-bright)" : "var(--emerald-bright)",
                          "font-weight": "bold",
                          "margin-right": "6px",
                        }}
                      >
                        {t.status === "failed" ? "✕" : "✓"}
                      </span>
                      <strong style={{ color: "var(--text-main)" }}>{t.name}</strong>
                    </div>
                    <Show when={t.duration_ms !== undefined}>
                      <span style={{ color: "var(--text-muted)", "font-size": "11px" }}>
                        {t.duration_ms}ms
                      </span>
                    </Show>
                  </div>
                  <Show when={t.traceback || t.message}>
                    <div class="traceback-drawer">{t.traceback ?? t.message}</div>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
