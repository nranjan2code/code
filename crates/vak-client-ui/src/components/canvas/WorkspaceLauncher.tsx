import { createSignal, For, Show } from "solid-js";
import * as api from "../../api";
import { activeId, openArtifactCanvas } from "../../store";
import Sheet from "../Sheet";
import { createLoader } from "./createLoader";

export default function WorkspaceLauncher(props:{session:string;onClose:()=>void}) {
  const loader=createLoader(()=>props.session,async(session,_current,signal)=>api.canvasWorkspaces(session,signal));
  const [error,setError]=createSignal(""),[busy,setBusy]=createSignal(false);
  const open=(resource:api.WorkspaceResource)=>{
    if(activeId()!==props.session)return;
    openArtifactCanvas({kind:"workspace",title:resource.title,workspaceType:resource.workspace_type,resourceId:resource.id,revision:"head",sessionId:props.session});props.onClose();
  };
  const create=async(definition:api.WorkspaceContribution)=>{
    if(busy())return;setBusy(true);setError("");try{const resource=await api.changeWorkspace(props.session,{action:"create",workspace_type:definition.spec.id,digest:definition.digest,generation:definition.generation});open(resource);}catch(cause){setError(cause instanceof Error?cause.message:"Could not create this workspace.");}finally{setBusy(false);}
  };
  return <Sheet title="Canvas workspaces" subtitle="Open saved work or start with an enabled plugin." size="wide" onClose={props.onClose}>
    <div class="canvas-installed-workspace"><Show when={loader.error()||error()}><p role="alert">{loader.error()||error()}</p><button onClick={loader.reload}>Retry</button></Show>
      <h3>Saved work</h3><For each={loader.data()?.resources} fallback={<p>No saved workspaces in this conversation yet.</p>}>{resource=><div class="canvas-workspace-actions"><button disabled={busy()} onClick={()=>open(resource)}>{resource.title}</button><span>{loader.data()?.contributions.some(item=>item.spec.id===resource.workspace_type)?"":"Plugin unavailable · work kept"}</span></div>}</For>
      <h3>Start a workspace</h3><For each={loader.data()?.contributions} fallback={<p>Install and enable a workspace plugin in Settings to start one.</p>}>{definition=><div><h4>{definition.spec.title}</h4><p>{definition.spec.description}</p><button disabled={busy()} onClick={()=>void create(definition)}>Start {definition.spec.title}</button></div>}</For>
    </div>
  </Sheet>;
}
