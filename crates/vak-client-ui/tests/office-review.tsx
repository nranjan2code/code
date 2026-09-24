import { render } from "solid-js/web";
import OfficeChangeList from "../src/components/OfficeChangeList";
import type { OfficeDiff } from "../src/api";
import "../src/styles.css";

// A diff shaped exactly like the server's office-diff response for the
// fixtures in crates/vak-ooxml/tests/diff.rs. Deterministic; no server.
const word: OfficeDiff = {
  path: "q3.docx",
  compared_with: "workspace",
  summary: ["Summary: 1 added", "Outlook: 1 changed"],
  flags: ["1 external link(s) (recorded, never followed)"],
  changes: [
    { section: "Summary", anchor: "p:1A000001", kind: "added", before: null, after: "[inserted by Mira: Details]" },
    { section: "Outlook", anchor: "p@11", kind: "changed", before: "Steady.", after: "[deleted by Mira: Steady.][inserted by Mira: Growing.]" },
  ],
};
const sheet: OfficeDiff = {
  path: "budget.xlsx",
  compared_with: "workspace",
  summary: ["Budget: 1 added, 1 changed"],
  flags: [],
  changes: [
    { section: "Budget", anchor: "Budget!B2", kind: "changed", before: "100", after: "150" },
    { section: "Budget", anchor: "Budget!F9", kind: "added", before: null, after: "note" },
  ],
};
const fresh: OfficeDiff = { path: "deck.pptx", compared_with: "nothing (new file)", summary: ["Deck: 1 added"], flags: [], changes: [
  { section: "Deck", anchor: "", kind: "added", before: null, after: "new file: 2 slides" },
] };

render(() => (
  <div style="display:grid;gap:24px;padding:16px;max-width:720px;margin:0 auto">
    <section id="word"><h3>q3.docx</h3><OfficeChangeList diff={word} /></section>
    <section id="sheet"><h3>budget.xlsx</h3><OfficeChangeList diff={sheet} /></section>
    <section id="fresh"><h3>deck.pptx</h3><OfficeChangeList diff={fresh} /></section>
  </div>
), document.getElementById("root")!);

(window as any).runChecks = () => {
  const word = document.querySelector("#word")!;
  const inserted = word.querySelectorAll("ins.office-redline-inserted");
  const deleted = word.querySelectorAll("del.office-redline-deleted");
  if (inserted.length !== 2 || deleted.length !== 1) throw new Error(`redline: ${inserted.length} ins, ${deleted.length} del`);
  if (deleted[0].textContent !== "Steady.") throw new Error("deleted text");
  if (word.querySelectorAll("h4")[1]?.textContent !== "Outlook") throw new Error("grouped by heading");
  if (!document.querySelector("#sheet")!.textContent!.includes("Budget!B2")) throw new Error("cell anchor");
  if (!document.querySelector("#fresh")!.textContent!.includes("A new file")) throw new Error("new file wording");
  if (document.querySelector("script:not([type=module])")) throw new Error("no script injected");
  return "office-review: ok";
};
