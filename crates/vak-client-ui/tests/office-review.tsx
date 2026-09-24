import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import OfficeChangeList from "../src/components/OfficeChangeList";
import type { OfficeReview } from "../src/api";
import { keep, leaveOut } from "../src/officeChoices";
import "../src/styles.css";

// Reviews shaped exactly like the server's office-review response for the
// fixtures in crates/vak-ooxml/tests/diff.rs. Deterministic; no server.
const word: OfficeReview = {
  path: "q3.docx",
  compared_with: "workspace",
  summary: ["Summary: 1 added", "Outlook: 1 changed"],
  flags: ["1 external link(s) (recorded, never followed)"],
  changes: [
    { section: "Summary", anchor: "p:1A000001", kind: "added", before: null, after: "[inserted by Mira: Details]" },
    { section: "Outlook", anchor: "p@11", kind: "changed", before: "Steady.", after: "[deleted by Mira: Steady.][inserted by Mira: Growing.]" },
  ],
};
const sheet: OfficeReview = {
  path: "budget.xlsx",
  compared_with: "workspace",
  summary: ["Budget: 1 added, 1 changed"],
  flags: [],
  changes: [
    { section: "Budget", anchor: "Budget!B2", kind: "changed", before: "100", after: "150" },
    { section: "Budget", anchor: "Budget!F9", kind: "added", before: null, after: "note" },
  ],
};
// Four choices; the last builds on the second (a paragraph written after a
// new paragraph), so leaving the second out takes the last with it.
const choosing: OfficeReview = {
  path: "q3.docx",
  compared_with: "workspace",
  summary: ["Summary: 3 added", "Outlook: 1 changed"],
  flags: [],
  changes: [],
  choices: [
    { id: "0", label: "Rewrite paragraph p@11", requires: [], changes: [{ section: "Outlook", anchor: "p@11", kind: "changed", before: "Steady.", after: "[deleted by Mira: Steady.][inserted by Mira: Growing.]" }] },
    { id: "1", label: "New paragraph after p@1", requires: [], changes: [{ section: "Summary", anchor: "p:1A000001", kind: "added", before: null, after: "[inserted by Mira: Details]" }] },
    { id: "2", label: "New paragraph after p@3", requires: [], changes: [{ section: "Summary", anchor: "p:1A000002", kind: "added", before: null, after: "[inserted by Mira: Risks]" }] },
    { id: "3", label: "New paragraph after p:1A000001", requires: ["1"], changes: [{ section: "Summary", anchor: "p:1A000003", kind: "added", before: null, after: "[inserted by Mira: More details]" }] },
  ],
};
choosing.changes = choosing.choices!.flatMap((choice) => choice.changes);
const whole: OfficeReview = { ...sheet, path: "whole.xlsx", choices_unavailable: "the draft is not what its recorded edits produce" };
const narrowed: OfficeReview = { ...sheet, path: "narrowed.xlsx", changes: sheet.changes.slice(0, 1), narrowed_from: { candidate_id: "c-full", keep: ["0:B2"] } };
const [excluded, setExcluded] = createSignal<ReadonlySet<string>>(new Set());
let madeWith: string[] | null = null;
let opened: string | null = null;

const fresh: OfficeReview = { path: "deck.pptx", compared_with: "nothing (new file)", summary: ["Deck: 1 added"], flags: [], changes: [
  { section: "Deck", anchor: "", kind: "added", before: null, after: "new file: 2 slides" },
] };

render(() => (
  <div style="display:grid;gap:24px;padding:16px;max-width:720px;margin:0 auto">
    <section id="word"><h3>q3.docx</h3><OfficeChangeList review={word} /></section>
    <section id="sheet"><h3>budget.xlsx</h3><OfficeChangeList review={sheet} /></section>
    <section id="fresh"><h3>deck.pptx</h3><OfficeChangeList review={fresh} /></section>
    <section id="choosing"><h3>q3.docx, choosing</h3><OfficeChangeList
      review={choosing}
      excluded={excluded()}
      onToggle={(id) => setExcluded((current) => current.has(id) ? keep(choosing.choices!, current, id) : leaveOut(choosing.choices!, current, id))}
      onMakeVersion={() => { madeWith = choosing.choices!.filter((choice) => !excluded().has(choice.id)).map((choice) => choice.id); }}
    /></section>
    <section id="whole"><h3>whole.xlsx</h3><OfficeChangeList review={whole} onToggle={() => {}} onMakeVersion={() => {}} /></section>
    <section id="narrowed"><h3>narrowed.xlsx</h3><OfficeChangeList review={narrowed} onOpenVersion={(id) => { opened = id; }} /></section>
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

  const choosing = document.querySelector("#choosing")!;
  const boxes = () => [...choosing.querySelectorAll<HTMLInputElement>(".office-choice input[type=checkbox]")];
  if (boxes().length !== 4 || boxes().some((box) => !box.checked)) throw new Error("every choice starts kept");
  if (choosing.querySelector(".office-choice-footer")) throw new Error("no footer until something is left out");
  if (!choosing.textContent!.includes("Builds on: New paragraph after p@1")) throw new Error("requirement named");
  boxes()[1].click();
  if (boxes()[1].checked || boxes()[3].checked) throw new Error("leaving 1 out takes 3 with it");
  if (!boxes()[0].checked || !boxes()[2].checked) throw new Error("others stay kept");
  const footer = choosing.querySelector(".office-choice-footer")!;
  if (!footer.textContent!.includes("Keeping 2 of 4 changes.")) throw new Error(`footer: ${footer.textContent}`);
  boxes()[3].click();
  if (!boxes()[1].checked || !boxes()[3].checked) throw new Error("keeping 3 brings back 1");
  boxes()[2].click();
  choosing.querySelector<HTMLButtonElement>(".office-choice-footer button")!.click();
  if (JSON.stringify(madeWith) !== JSON.stringify(["0", "1", "3"])) throw new Error(`made with ${JSON.stringify(madeWith)}`);
  boxes()[2].click();
  if (choosing.querySelector(".office-choice-footer")) throw new Error("footer leaves once everything is kept again");

  const whole = document.querySelector("#whole")!;
  if (whole.querySelector("input[type=checkbox]")) throw new Error("no choices when unavailable");
  if (!whole.textContent!.includes("accepted or rejected whole")) throw new Error("says why it is whole");
  const narrowed = document.querySelector("#narrowed")!;
  if (!narrowed.textContent!.includes("keeps 1 of the full draft's changes")) throw new Error("narrowed note");
  narrowed.querySelector<HTMLButtonElement>(".office-choice-note button")!.click();
  if (opened !== "c-full") throw new Error("choose again opens the full draft");
  return "office-review: ok";
};
