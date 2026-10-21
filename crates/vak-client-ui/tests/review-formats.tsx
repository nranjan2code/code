import { render } from "solid-js/web";
import WorkbenchPanel from "../src/components/WorkbenchPanel";
import * as store from "../src/store";
import type { SandboxCandidateRecord, OfficeReview } from "../src/api";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") ?? "light";
const session = "review-format-fixture";
const entries = ["report.html", "notes.md", "plain.txt", "source.js", "data.csv", "config.json", "report.docx", "report.xlsx", "report.pptx", "report.pdf", "image.png", "audio.wav", "video.mp4", "archive.zip", "deleted.txt", "unavailable.txt", `${"long-name-".repeat(18)}.js`];
let selected: SandboxCandidateRecord | null = null;
const mutations: string[] = [];
const requests: string[] = [];
const text = (path: string) => path.endsWith("html") ? '<!doctype html><h1>HTML draft</h1><button onclick="parent.postMessage(\'review-escaped\',\'*\')">Scenario</button><script>parent.postMessage("review-escaped","*")</script>' : `${path}\n${"a_very_long_line_".repeat(100)}\nDraft content`;
const office = (path: string): OfficeReview => ({path, compared_with:"workspace", summary:["1 change"], flags:[], changes:[{section:"Updated content", anchor:`location-${"long".repeat(30)}`, kind:"changed", before:"Previous content", after:"Revised content"}], choices:[{id:"choice-1", label:`Edit ${path}`, requires:[], changes:[{section:"Updated content",anchor:`location-${"long".repeat(30)}`,kind:"changed",before:"Previous content",after:"Revised content"}]},{id:"choice-2",label:`Edit another part of ${path}`,requires:[],changes:[]}]});
window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  requests.push(url.pathname);
  if (init?.method && init.method !== "GET") { mutations.push(url.pathname); throw new Error("This fixture must not mutate saved files"); }
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), {status,headers:{"Content-Type":"application/json"}});
  if (url.pathname.endsWith("/sandbox/records")) return json({records:selected ? [{kind:"Candidate",record:selected}] : [], artifacts:selected?.candidate.files.map(file=>({artifact:`artifact-${file.path}`,version:selected!.candidate.candidate_id,path:file.path,candidate:selected!.candidate.candidate_id,promoted:false}))??[]});
  if (url.pathname.includes("/comments")) return json({comments:[]});
  const artifact = /^\/library\/([^/]+)(.*)$/.exec(url.pathname);
  if (artifact) {
    const path=decodeURIComponent(artifact[1]).replace(/^artifact-/,"");
    if (!artifact[2]) return json({starred:false});
    if (artifact[2].endsWith("/review")) return json(office(path));
    if (artifact[2].endsWith("/text")) {
      if (path === "unavailable.txt") return json({error:"Unavailable saved bytes"},404);
      return json({path,kind:/\.(png|wav|mp4|zip)$/.test(path)?"binary":"text",bytes:100,editable:false,...(/\.(png|wav|mp4|zip)$/.test(path)?{}:{content:text(path)})});
    }
  }
  if (url.pathname === "/fs/file" || url.pathname.endsWith("/artifact")) return json({path:url.searchParams.get("path"),kind:"text",bytes:8,editable:false,content:"Before content"});
  return json({records:[],comments:[],artifacts:[]});
};
Object.defineProperty(window, "EventSource", {value:class extends EventTarget { close(){} }});
store.setActiveId(session);
render(()=><div style="height:100vh;display:flex;justify-content:flex-end"><div style="width:360px;height:100%;transform:translateX(0);overflow:hidden"><WorkbenchPanel /></div></div>,document.getElementById("root")!);
const wait=()=>new Promise(r=>setTimeout(r,120));
const within=(element:Element)=>{const r=element.getBoundingClientRect();return r.left>=-1&&r.top>=-1&&r.right<=innerWidth+1&&r.bottom<=innerHeight+1};
const buttons=(root:Element)=>[...root.querySelectorAll<HTMLButtonElement>("button")];
async function open(paths:string[], tag:string) {
  const id=`exec-${tag}`;
  selected={record_id:`record-${tag}`,session_id:session,turn_id:"turn",result_id:"result",execution_id:id,environment_id:"fixture",candidate_digest:"fixture",verified:true,draft_checks:tag==="checks-failed"?[{verifier:"fixture",path:paths[0],status:"failed",evidence:"Synthetic failure"}]:[],updated_at:new Date().toISOString(),candidate:{candidate_id:`candidate-${tag}`,source_root:"/tmp/fixture-draft",destination_root:"/tmp/fixture-workspace",files:paths.map(path=>({path,candidate_hash:"new",base_hash:"old",bytes:100,operation:path==="deleted.txt"?"Delete":"Upsert"}))}};
  store.setWorkbenchExecutions([{id,ownerSessionId:session,tool:"bash",command:"synthetic",language:"",scratchDir:"/tmp/fixture-draft",stdout:"",stderr:"",packages:[],artifacts:paths.map(path=>({path:`/tmp/fixture-draft/${path}`,mimeType:"application/octet-stream",sizeBytes:100})),status:"completed",timestamp:new Date().toISOString()}]);
  store.setActiveExecutionId(id);await wait();store.openCandidateReview(id,session,selected.candidate.candidate_id);
  for(let i=0;i<50;i++){await wait();if(document.querySelector(".candidate-review")&&!document.querySelector(".candidate-review")!.textContent!.includes("Loading the saved version"))break;}
  await new Promise(r=>setTimeout(r,300));
}
Object.assign(window,{setReviewTechnicalDetails:store.setTechnicalDetailsSignal,openReviewFormat:async(path:string)=>open([path],path),runChecks:async()=>{
  const passed:string[]=[];const check=(ok:unknown,label:string)=>{if(!ok)throw new Error(label);passed.push(label)};
  let escaped=false;window.addEventListener("message",e=>{if(e.data==="review-escaped")escaped=true});
  await wait();
  for(const path of entries){
    await open([path],path);
    const dialog=document.querySelector<HTMLElement>(".candidate-review")!;
    check(!!dialog&&within(dialog)&&!dialog.closest(".workbench-panel"),`${path}: Review fills the viewport outside Details`);
    const close=dialog.querySelector<HTMLButtonElement>(".sheet-close")!;
    const accept=buttons(dialog).find(x=>/^Accept \d/.test(x.textContent??""))!;
    check(within(close)&&within(accept)&&within(buttons(dialog).find(x=>x.textContent==="Keep as draft")!),`${path}: close and decisions fit`);
    const body=dialog.querySelector<HTMLElement>(".sheet-body")!;
    check(body.scrollWidth<=body.clientWidth+1,`${path}: content does not overflow horizontally`);
    const comparison=dialog.querySelector<HTMLDetailsElement>(".candidate-review-compare");
    if(comparison){comparison.open=true;await wait();check(body.scrollWidth<=body.clientWidth+1,`${path}: before/after comparison stays within the body`);comparison.open=false;}
    if(path==="report.html")check(!!dialog.querySelector("iframe[sandbox='']")&&!escaped,`${path}: HTML preview stays script-blocked`);
    if(/\.(docx|xlsx|pptx|pdf)$/.test(path)){
      store.setTechnicalDetailsSignal(true); await wait();
      check(body.scrollWidth<=body.clientWidth+1,`${path}: long technical anchors also fit`);
      store.setTechnicalDetailsSignal(false); await wait();
      check(dialog.textContent?.includes("Revised content"),`${path}: semantic changes are readable`);
      const choice=dialog.querySelector<HTMLInputElement>(".office-choice input")!;
      choice.click();check(accept.disabled,`${path}: omitted edits prevent acceptance until a new version exists`);choice.click();
      check(!accept.disabled,`${path}: restoring every edit restores acceptance`);
    }
    if(/\.(png|wav|mp4|zip)$/.test(path))check(dialog.textContent?.includes("This file has no text preview"),`${path}: binary files have an explicit inspect/download path`);
    if(path==="deleted.txt")check(dialog.textContent?.includes("This file will be deleted."),`${path}: deletion is explicit`);
    if(path==="unavailable.txt")check(accept.disabled&&!!dialog.querySelector('[role="alert"]'),`${path}: failed reads keep acceptance disabled`);
    if(path!=="unavailable.txt")check(!accept.disabled,`${path}: successfully read file can be accepted`);
    const inclusion=dialog.querySelector<HTMLInputElement>(".candidate-review-include input")!;
    inclusion.click();check(accept.disabled&&dialog.textContent?.includes("Choose at least one file to accept."),`${path}: the include control explains an empty selection`);inclusion.click();
    check(dialog.querySelector(".candidate-review-file-actions")!.children.length<=2,`${path}: only preview and More compete in the file toolbar`);
    const more=dialog.querySelector<HTMLDetailsElement>(".candidate-review-more")!;
    more.open=true;await wait();
    window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));
    check(!more.open&&dialog.isConnected,`${path}: Escape closes More before Review`);
    more.open=true;
    const focus=buttons(dialog).find(x=>x.textContent==="Focus on this file")!;focus.click();await wait();
    const focusedContent=dialog.querySelector<HTMLElement>(/\.(docx|xlsx|pptx|pdf)$/.test(path)?".candidate-review-changes":".candidate-review-preview");
    check(focusedContent&&focusedContent.getClientRects().length>0&&(focusedContent.textContent?.trim()||focusedContent.querySelector("iframe")),`${path}: Focus keeps the file content visible`);
    check(within(accept)&&!!buttons(dialog).find(x=>x.textContent==="Show full review"),`${path}: focused review preserves decisions and return control`);
    if(/\.(docx|xlsx|pptx|pdf)$/.test(path)) {
      dialog.querySelector<HTMLButtonElement>(".office-change-comment")!.click(); await wait();
      check(!dialog.classList.contains("candidate-review-focused")&&dialog.querySelector<HTMLDetailsElement>(".candidate-review-discussion")!.open&&document.activeElement?.id==="candidate-review-comment",`${path}: commenting from Focus opens the addressed discussion`);
    } else { more.open=true;buttons(dialog).find(x=>x.textContent==="Show full review")!.click();await wait(); }
    dialog.querySelector<HTMLDetailsElement>(".candidate-review-discussion")!.open=true;await wait();
    const comment=dialog.querySelector<HTMLTextAreaElement>("#candidate-review-comment")!;
    comment.scrollIntoView();check(within(comment),`${path}: draft comments remain reachable`);
    close.click();await wait();check(!document.querySelector(".candidate-review"),`${path}: closes cleanly`);
  }
  await open(["report.docx"],"checks-failed");
  const warningDialog=document.querySelector(".candidate-review")!;
  warningDialog.querySelector<HTMLDetailsElement>(".candidate-review-more")!.open=true;
  buttons(warningDialog).find(x=>x.textContent==="Focus on this file")!.click();await wait();
  check(within(warningDialog.querySelector('[role="alert"]')!),"A failed check stays visible in Focus view");
  warningDialog.querySelector<HTMLButtonElement>(".sheet-close")!.click();await wait();
  const mixed=["report.html","source.js","report.docx","report.xlsx","report.pptx","report.pdf","image.png"];
  await open(mixed,"mixed");const dialog=document.querySelector(".candidate-review")!;
  const accept=buttons(dialog).find(x=>/^Accept 7/.test(x.textContent??""))!;
  check(dialog.textContent?.includes("Review 6 more selected files before accepting."),"Mixed draft explains why acceptance is unavailable");
  check(accept.disabled,"Mixed draft waits until every selected file has been read");
  for(const path of mixed){buttons(dialog).find(x=>x.textContent===path)!.click();await wait();await wait();check(dialog.textContent?.includes(path)&&within(accept),`Mixed draft: ${path} can be selected without losing decisions`);}
  check(!accept.disabled,"Mixed draft becomes ready only after every selected file is read");
  const first=dialog.querySelector<HTMLInputElement>(".candidate-review-files input")!;first.click();await wait();
  check(accept.textContent==="Accept 6 selected files","Mixed draft file selection updates acceptance scope");
  dialog.querySelector<HTMLDetailsElement>(".candidate-review-more")!.open=true;
  buttons(dialog).find(x=>x.textContent==="Open in Library")!.click();await wait();
  check(!document.querySelector(".candidate-review")&&store.libraryOpen()&&store.libraryFocus()==="artifact-image.png","Open in Library closes Review and carries the selected artifact");
  check(mutations.length===0,"No test accepts, exports, rewrites or comments on a real file");
  return passed;
}});
