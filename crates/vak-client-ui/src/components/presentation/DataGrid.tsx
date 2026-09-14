import { For, Show, createMemo, createSignal } from "solid-js";
import { downloadCsv } from "./data";

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

  const columns = () => props.data?.columns || [];
  const rows = () => props.data?.rows || [];

  const filteredAndSortedRows = createMemo(() => {
    let list = [...rows()];
    const q = search().toLowerCase().trim();
    if (q) {
      list = list.filter((row) =>
        Object.values(row || {}).some((val) => String(val).toLowerCase().includes(q))
      );
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
    <div class="canvas-card data-grid-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Data Grid</span>
          <span class="card-subtitle">{props.data?.title ?? "Dataset Records"}</span>
          <span class="card-badge" style={{ "font-size": "11px", opacity: "0.8" }}>
            {filteredAndSortedRows().length} of {rows().length} rows
          </span>
        </div>
        <div class="card-actions">
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
        </div>
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
                    <button type="button" onClick={() => handleSort(col.key)}>
                    {col.label}{" "}
                    {sortCol() === col.key ? (sortAsc() ? "↑" : "↓") : "↕"}
                    </button>
                  </th>
                )}
              </For>
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
