import { For, Show, createMemo, createSignal } from "solid-js";

export interface DataGridColumn {
  key: string;
  label: string;
  isNumeric?: boolean;
}

export interface DataGridData {
  title?: string;
  columns: DataGridColumn[];
  rows: Record<string, any>[];
}

export default function DataGrid(props: { data: DataGridData }) {
  const [search, setSearch] = createSignal("");
  const [sortCol, setSortCol] = createSignal<string | null>(null);
  const [sortAsc, setSortAsc] = createSignal<boolean>(true);
  const [copied, setCopied] = createSignal(false);

  const handleSort = (colKey: string) => {
    if (sortCol() === colKey) {
      setSortAsc(!sortAsc());
    } else {
      setSortCol(colKey);
      setSortAsc(true);
    }
  };

  const filteredAndSortedRows = createMemo(() => {
    let list = [...props.data.rows];
    const q = search().toLowerCase().trim();
    if (q) {
      list = list.filter((row) =>
        Object.values(row).some((val) => String(val).toLowerCase().includes(q))
      );
    }
    const col = sortCol();
    if (col) {
      const isNum = props.data.columns.find((c) => c.key === col)?.isNumeric;
      list.sort((a, b) => {
        const valA = a[col];
        const valB = b[col];
        if (isNum) {
          const numA = parseFloat(String(valA).replace(/[^0-9.-]/g, "")) || 0;
          const numB = parseFloat(String(valB).replace(/[^0-9.-]/g, "")) || 0;
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
    const cols = props.data.columns;
    let csv = cols.map((c) => `"${c.label}"`).join(",") + "\n";
    for (const row of filteredAndSortedRows()) {
      csv += cols.map((c) => `"${String(row[c.key] ?? "").replace(/"/g, '""')}"`).join(",") + "\n";
    }
    void navigator.clipboard.writeText(csv);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div class="canvas-card data-grid-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Data Grid</span>
          <span class="card-subtitle">{props.data.title ?? "Dataset Records"}</span>
        </div>
        <div class="card-actions">
          <input
            type="text"
            class="grid-search-input"
            placeholder="Search records..."
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
          <button class="pill-action-btn" onClick={handleExportCsv}>
            {copied() ? "✓ Copied CSV" : "Export CSV"}
          </button>
        </div>
      </div>

      <div class="grid-table-container">
        <table class="sleek-grid">
          <thead>
            <tr>
              <For each={props.data.columns}>
                {(col) => (
                  <th
                    class={col.isNumeric ? "cell-numeric" : ""}
                    onClick={() => handleSort(col.key)}
                  >
                    {col.label}{" "}
                    {sortCol() === col.key ? (sortAsc() ? "↑" : "↓") : "↕"}
                  </th>
                )}
              </For>
            </tr>
          </thead>
          <tbody>
            <For each={filteredAndSortedRows()}>
              {(row) => (
                <tr>
                  <For each={props.data.columns}>
                    {(col) => {
                      const val = String(row[col.key] ?? "");
                      const isStatus = col.key.toLowerCase().includes("status");
                      const isPositive = val.startsWith("+") || val.toLowerCase().includes("healthy");
                      const isNegative = val.startsWith("-") || val.toLowerCase().includes("fail");
                      return (
                        <td
                          class={col.isNumeric ? "cell-numeric" : ""}
                          style={{
                            color: isPositive
                              ? "var(--emerald-bright)"
                              : isNegative
                              ? "var(--rose-bright)"
                              : undefined,
                            "font-weight": isStatus ? "600" : undefined,
                          }}
                        >
                          <Show when={isStatus}>● </Show>
                          {val}
                        </td>
                      );
                    }}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
