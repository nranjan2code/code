import { createSignal, Show } from "solid-js";
import { render } from "solid-js/web";
import WorkspaceLauncher from "../src/components/canvas/WorkspaceLauncher";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import { computeWorkspace } from "../src/canvasComputeClient";
import * as store from "../src/store";
import type { WorkspaceContribution, WorkspaceResource } from "../src/api";
import "../src/styles.css";

// Production host/renderer and real browser Worker; only the authenticated API is a stub.
const params=new URLSearchParams(location.search);
document.documentElement.dataset.theme=params.get("theme")==="dark"?"dark":"light";
const [launcher,setLauncher]=createSignal(false);
const [report,setReport]=createSignal("Installed workspace acceptance");
const contribution:WorkspaceContribution={plugin:"fixture",digest:"fixture-digest",generation:1,spec:{schema:1,id:"fixture/parameters",title:"Component parameters",description:"Installed contract probe",initial:{length:12},state_schema:{type:"object"},fields:[{path:"/length",label:"Length",kind:"number",unit:"mm"}],commands:[{id:"apply",title:"Apply parameters",arguments:{type:"object"},edits:[{target:"/length",argument:"/length"}]}],context:["/length"]}};
let resource:WorkspaceResource={id:"fixture-resource",workspace_type:contribution.spec.id,plugin_digest:contribution.digest,title:contribution.spec.title,revision:1,data:{length:12},undo:[],redo:[]};
let enabled=true;
let asked:Record<string,unknown>|undefined;
const json=(value:unknown,status=200)=>new Response(JSON.stringify(value),{status,headers:{"Content-Type":"application/json"}});
window.fetch=async(input,init)=>{
  const url=String(input);
  if(url.endsWith("/canvas/workspaces/ask")){asked=JSON.parse(String(init?.body));return json({ok:true});}
  if(url.endsWith("/canvas/workspaces")){
    if(init?.method==="POST"){
      const body=JSON.parse(String(init.body));
      if(body.action==="create") { resource={...resource,revision:1,data:{length:12},undo:[],redo:[]};return json(resource); }
      if(!enabled||body.revision!==resource.revision)return json({error:"Workspace activation or revision changed"},409);
      if(body.action==="command")resource={...resource,revision:resource.revision+1,data:body.arguments,undo:[resource.data],redo:[]};
      else if(body.action==="undo")resource={...resource,revision:resource.revision+1,data:resource.undo.at(-1),undo:[],redo:[resource.data]};
      else resource={...resource,revision:resource.revision+1,data:resource.redo.at(-1),redo:[],undo:[resource.data]};
      return json(resource);
    }
    return json({contributions:enabled?[contribution]:[],resources:[resource]});
  }
  return json({records:[]});
};
store.setActiveId("installed-workspace-fixture");
const subject={kind:"workspace" as const,workspaceType:contribution.spec.id,resourceId:resource.id,revision:"head",sessionId:"installed-workspace-fixture",title:resource.title};
render(()=><><button onClick={()=>setLauncher(true)}>Browse workspaces</button><Show when={launcher()}><WorkspaceLauncher session="installed-workspace-fixture" onClose={()=>setLauncher(false)}/></Show><button onClick={()=>store.openArtifactCanvas(subject)}>Open installed workspace</button><output style="display:block;white-space:pre-wrap">{report()}</output><ArtifactCanvas/></>,document.getElementById("root")!);
const settle=async()=>{for(let i=0;i<8;i++)await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));};
const button=(text:string)=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(item=>item.textContent===text)!;
const input=()=>document.querySelector<HTMLInputElement>('.canvas-workspace-fields input')!;
const type=(value:string)=>{input().value=value;input().dispatchEvent(new Event("input",{bubbles:true}));};
const passed:string[]=[];
const check=(value:unknown,label:string)=>{if(!value)throw new Error(label);passed.push(label);};
const unsigned=(value:number)=>{const out:number[]=[];do{const next=value&127;value>>>=7;out.push(next|(value?128:0));}while(value);return out;};
const section=(id:number,bytes:number[])=>[id,...unsigned(bytes.length),...bytes];
const name=(value:string)=>[...unsigned(value.length),...new TextEncoder().encode(value)];
function wasm(loop=false){return new Uint8Array([0,97,115,109,1,0,0,0,...section(1,[2,0x60,1,0x7f,1,0x7f,0x60,2,0x7f,0x7f,1,0x7e]),...section(3,[2,0,1]),...section(5,[1,1,1,1]),...section(7,[3,...name("memory"),2,0,...name("alloc"),0,0,...name("evaluate"),0,1]),...section(10,[2,5,0,0x41,0x80,0x08,0x0b,...(loop?[9,0,3,0x40,0x0c,0,0x0b,0x42,11,0x0b]:[4,0,0x42,11,0x0b])]),...section(11,[1,0,0x41,0,0x0b,11,...new TextEncoder().encode('{"value":7}')])]);}
async function run(){
  button("Browse workspaces").click();await settle();
  check(document.querySelector('.sheet')?.textContent?.includes("Start a workspace"),"Production launcher lists enabled workspace types");
  button("Start Component parameters").click();await settle();
  check(!!document.querySelector('.artifact-canvas')&&!document.querySelector('.sheet'),"Launcher starts a resource and closes its sheet");
  document.querySelector<HTMLButtonElement>('[aria-label="Close Canvas"]')!.click();await new Promise(resolve=>setTimeout(resolve,300));await settle();
  button("Browse workspaces").click();await settle();button("Component parameters").click();await settle();
  check(!!document.querySelector('.artifact-canvas')&&!document.querySelector('.sheet'),"Launcher reopens saved work in its conversation");
  check(input()?.value==="12","Installed definition mounts in the production Canvas host");
  type("24");await settle();check(!!store.canvasEntry()?.checkpoints?.["workspace/fields"],"Pending fields checkpoint before commit");
  button("Apply parameters").click();await settle();
  check((resource.data as {length:number}).length===24&&resource.revision===2,"Declared command saves typed arguments");
  check(!store.canvasEntry()?.checkpoints?.["workspace/fields"],"Successful full commit clears pending fields");
  button("Undo").click();await settle();check(input().value==="12","Undo restores saved state");
  button("Redo").click();await settle();check(input().value==="24","Redo restores the command");
  type("30");await settle();enabled=false;window.dispatchEvent(new Event("focus"));await settle();
  check(document.body.textContent?.includes("plugin is unavailable"),"Disable shows an explicit unavailable view");
  check(!!store.canvasEntry()?.checkpoints?.["workspace/fields"],"Disable retains pending fields");
  enabled=true;window.dispatchEvent(new Event("focus"));await settle();check(input().value==="30","Enable restores pending fields");
  resource={...resource,revision:resource.revision+1,data:{length:40}};window.dispatchEvent(new Event("focus"));await settle();
  check(button("Apply parameters").disabled,"Remote revision requires explicit pending-edit resolution");
  button("Discard pending edits and use saved values").click();await settle();check(input().value==="40","Explicit discard uses the saved revision");
  const checkbox=document.querySelector<HTMLInputElement>('.canvas-installed-workspace input[type="checkbox"]')!;checkbox.click();
  const prompt=document.querySelector<HTMLTextAreaElement>('[aria-label="Ask Agent about this workspace"]')!;prompt.value="Explain length";prompt.dispatchEvent(new Event("input",{bubbles:true}));button("Send request").click();await settle();
  check(JSON.stringify(asked?.context)==='["/length"]'&&asked?.revision===resource.revision,"Ask Agent sends selected saved fields and revision");
  const result=await computeWorkspace(wasm().buffer,{},new AbortController().signal);check((result as {value:number}).value===7,"Real browser Worker evaluates the bounded WASM ABI");
  const cancel=new AbortController();const pending=computeWorkspace(wasm(true).buffer,{},cancel.signal);setTimeout(()=>cancel.abort(),50);
  let cancelled=false;try{await pending;}catch(error){cancelled=error instanceof Error&&error.message==="Workspace computation was cancelled.";}check(cancelled,"Aborting an infinite WASM calculation terminates its worker");
  check(document.documentElement.scrollWidth<=innerWidth,"Installed workspace fits the viewport");
  document.documentElement.dataset.checks=`PASS: ${passed.length} installed workspace checks`;setReport(passed.join("\n"));
}
if(params.has("run"))void run().catch(error=>{document.documentElement.dataset.checks=`FAILED: ${String(error)}`;setReport(String(error));});
