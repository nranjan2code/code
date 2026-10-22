import { createSignal, onCleanup } from "solid-js";
import { render } from "solid-js/web";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import { useCanvasWorkspace, type CanvasWorkspace } from "../src/components/canvas/WorkspaceContext";
import { registerCanvasWorkspace, type CanvasWorkspaceDefinition } from "../src/canvasWorkspaces";
import * as store from "../src/store";
import "../src/styles.css";

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") === "dark" ? "dark" : "light";
window.fetch = async () => new Response(JSON.stringify({records:[],contributions:[],resources:[]}), {headers:{"Content-Type":"application/json"}});
const [report, setReport] = createSignal("Running host lifecycle checks…");
let handle: CanvasWorkspace | undefined;
let mounts = 0;
let disposals = 0;
function Editor() {
  const host = useCanvasWorkspace()!;
  handle = host;
  mounts++;
  onCleanup(() => disposals++);
  return <section style="padding:20px"><h2>Owned workspace lifecycle probe</h2>
    <label>Component name <input aria-label="Component name" value={String(host.read("probe/component")?.data ?? "")} onInput={event => host.checkpoint("probe/component", {schema:1, summary:event.currentTarget.value, data:event.currentTarget.value})} /></label>
    <p>Editing checkpoints remain in the shared host when this renderer is unavailable.</p>
    <button onClick={() => host.clear("probe/component")}>Save component</button>
  </section>;
}
const definition: CanvasWorkspaceDefinition = {id:"vak/host-probe", spec:{badge:"Workspace",views:[],defaultView:()=>null,layout:"focused",reloadable:false,devices:false,popout:false,feedback:"none",selects:()=>null,safetyFooter:false},component:Editor};
let unregister = registerCanvasWorkspace(definition);

store.setActiveId("workspace-host-probe");
const subject = {kind:"workspace" as const,title:"Workspace host lifecycle",workspaceType:definition.id,resourceId:"synthetic-component",revision:"fixture",sessionId:"workspace-host-probe"};
render(() => <><button onClick={() => store.openArtifactCanvas(subject)}>Open workspace</button><output style="display:block;padding:20px;white-space:pre-wrap">{report()}</output><ArtifactCanvas /></>,document.getElementById("root")!);
const settle = async () => { for(let i=0;i<6;i++) await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve())); };
const passed: string[] = [];
const check = (condition: unknown, text: string) => { if(!condition) throw new Error(text); passed.push(text); };
async function run() {
  const opener = document.querySelector<HTMLButtonElement>("button")!;
  opener.focus(); opener.click(); await settle();
  check(document.querySelector(".artifact-canvas")?.contains(document.activeElement), "Focused workspace receives keyboard focus");
  const input = document.querySelector<HTMLInputElement>('[aria-label="Component name"]')!;
  input.value = "Rotor"; input.dispatchEvent(new Event("input", {bubbles:true})); await settle();
  const before = store.canvasEntry();
  const oldHandle = handle!;
  unregister(); await settle();
  check(document.body.textContent?.includes("This workspace plugin is unavailable"), "Unavailable contribution has an explicit preserved state");
  check(store.canvasEntry()?.checkpoints?.["probe/component"].data === "Rotor", "Removing the renderer keeps its pending work");
  check(oldHandle.signal.aborted && disposals === 1, "Removing the renderer revokes its editing lease and disposes its view");
  oldHandle.clear("probe/component"); oldHandle.checkpoint("probe/component", {schema:1,summary:"Late callback",data:"wrong"});
  check(store.canvasEntry() === before, "Late callbacks cannot alter the tab after removal");
  unregister = registerCanvasWorkspace({...definition}); await settle();
  check(mounts === 2 && document.querySelector<HTMLInputElement>('[aria-label="Component name"]')?.value === "Rotor", "Returning contribution restores the same resource and pending work");
  check(handle !== oldHandle && !handle!.signal.aborted, "The replacement renderer receives a fresh lease");
  const firstHandle = handle!;
  store.setActiveId("another-workspace-conversation");
  store.openArtifactCanvas(subject); await settle();
  check(firstHandle.signal.aborted && handle !== firstHandle, "The same subject in another conversation has a separate editing lease");
  check(!handle!.read("probe/component"), "Another conversation does not inherit the first conversation’s pending edits");
  const secondHandle = handle!;
  secondHandle.checkpoint("probe/component", {schema:1,summary:"Second conversation",data:"Stator"});
  store.setActiveId("workspace-host-probe"); await settle();
  check(secondHandle.signal.aborted && handle!.read("probe/component")?.data === "Rotor", "Returning restores only that conversation’s pending work");
  document.querySelector<HTMLButtonElement>('[aria-label="Close Canvas"]')!.click(); await settle();
  check(document.body.textContent?.includes("Discard edits and close"), "Closing pending work asks for an explicit discard");
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="Keep working")!.click(); await settle();
  check(store.canvasEntry()?.checkpoints?.["probe/component"].data === "Rotor", "Keeping work leaves the checkpoint intact");
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="Save component")!.click(); await settle();
  const live = handle!;
  document.querySelector<HTMLButtonElement>('[aria-label="Close Canvas"]')!.click();
  await new Promise(resolve=>setTimeout(resolve,300)); await settle();
  check(!store.canvasOpen() && live.signal.aborted, "Closing a saved workspace revokes its lease");
  check(document.activeElement === opener, "Closing returns focus to the opener");
  check(document.documentElement.scrollWidth <= innerWidth, "The host fits the viewport");
  document.documentElement.dataset.checks = `PASS: ${passed.length} host lifecycle checks`;
  setReport(passed.join("\n"));
}
if(params.has("run")) void run().catch(error => { document.documentElement.dataset.checks = `FAILED: ${String(error)}`; setReport(String(error)); });
