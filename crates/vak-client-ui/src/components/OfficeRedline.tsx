import { For } from "solid-js";
import { parseRedline } from "../officeRedline";

// Office text with the reader's markers drawn as elements: tracked
// insertions and deletions as <ins>/<del> with their author, hidden and
// white text marked (O6). Always text nodes, never mounted markup.
export default function Redline(props: { text: string }) {
  return (
    <For each={parseRedline(props.text)}>{(segment) => {
      switch (segment.kind) {
        case "inserted":
          return <ins class="office-redline-inserted" title={`Inserted by ${segment.author}`}>{segment.text}</ins>;
        case "deleted":
          return <del class="office-redline-deleted" title={`Deleted by ${segment.author}`}>{segment.text}</del>;
        case "hidden":
          return <span class="office-redline-hidden" title="Hidden text in the document">{segment.text}</span>;
        case "white":
          return <span class="office-redline-hidden" title="White text in the document">{segment.text}</span>;
        default:
          return <>{segment.text}</>;
      }
    }}</For>
  );
}
