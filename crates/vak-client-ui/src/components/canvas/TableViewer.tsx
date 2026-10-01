import { For, Show } from "solid-js";
import { subjectPath } from "../../canvasSubject";
import { subjectReader } from "../../artifactPreview";
import { parseDelimitedPreview, type DelimitedPreview } from "../../delimitedPreview";
import { createLoader, readText } from "./createLoader";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";

/** A CSV or TSV file as a bounded table, with its source one switch away. */
export default function TableViewer(props: ViewerProps) {
  const loader = createLoader(
    () => [props.subject, props.reloadKey] as const,
    async ([subject]) => {
      const file = subjectPath(subject);
      const text = await readText(subjectReader(subject), file);
      let table: DelimitedPreview | null = null;
      let warning: string | null = null;
      try {
        table = parseDelimitedPreview(text, file.toLowerCase().endsWith(".tsv") ? "\t" : ",");
      } catch (cause) {
        warning = cause instanceof Error ? cause.message : String(cause);
      }
      return { text, table, warning };
    },
  );
  return (
    <LoadState loader={loader}>{(loaded) =>
      <Show when={props.view === "source"} fallback={
        <>
          <Show when={loaded.warning}>{(warning) => <div class="artifact-canvas-preview-warning" role="status">{warning()}</div>}</Show>
          <Show when={loaded.table}>{(table) =>
            <div class="artifact-canvas-table-preview">
              <p>{table().totalRows} {table().totalRows === 1 ? "row" : "rows"} · {table().headers.length} {table().headers.length === 1 ? "column" : "columns"}{table().truncated ? ` · showing first ${table().rows.length}` : ""}</p>
              <div class="artifact-canvas-table-scroll">
                <table>
                  <thead><tr><For each={table().headers}>{(header, index) => <th scope="col">{header || `Column ${index() + 1}`}</th>}</For></tr></thead>
                  <tbody><For each={table().rows}>{(row) => <tr><For each={row}>{(cell) => <td>{cell}</td>}</For></tr>}</For></tbody>
                </table>
              </div>
            </div>
          }</Show>
        </>
      }>
        <SourcePane text={loaded.text} label={subjectPath(props.subject)} selection={props.selection} onSelect={props.onSelect} />
      </Show>
    }</LoadState>
  );
}
