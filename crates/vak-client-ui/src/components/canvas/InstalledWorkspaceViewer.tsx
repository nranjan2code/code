import { batch, createEffect, on, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { acceptWorkspaceSave, workspaceArguments, workspaceInputs, workspacePointer as pointer } from "../../canvasWorkspaceInputs";
import { computeWorkspace } from "../../canvasComputeClient";
import type { ViewerProps } from "./types";
import { useCanvasWorkspace } from "./WorkspaceContext";
import { createLoader } from "./createLoader";

/** Package text is always text; the owned renderer interprets a closed data contract. */
export default function InstalledWorkspaceViewer(props: ViewerProps) {
  const host=useCanvasWorkspace()!;
  const session=()=>props.subject.kind==="workspace"?props.subject.sessionId:undefined;
  const resourceId=()=>props.subject.kind==="workspace"?props.subject.resourceId:"";
  const kind=()=>props.subject.kind==="workspace"?props.subject.workspaceType:"";
  const loader=createLoader(()=>[session(),resourceId(),kind()],async(_source,_current,signal)=>{
    const id=session();if(!id)throw new Error("This workspace has no conversation.");
    return api.canvasWorkspaces(id,signal);
  });
  const contribution=createMemo(()=>loader.data()?.contributions.find(item=>item.spec.id===kind()));
  const [resource,setResource]=createSignal<api.WorkspaceResource>();
  const [values,setValues]=createSignal<Record<string,string>>({});
  const [busy,setBusy]=createSignal(false),[error,setError]=createSignal(""),[prompt,setPrompt]=createSignal("");
  const [shared,setShared]=createSignal<string[]>([]);
  const [conflict,setConflict]=createSignal(false);
  let job:AbortController|undefined;
  const activation=createMemo(()=>{const current=contribution();return current?`${current.digest}:${current.generation}`:null;});
  createEffect(on(activation,()=>job?.abort(),{defer:true}));
  onCleanup(()=>job?.abort());
  let edits=0;
  createEffect(()=>{
    const loaded=loader.data();if(!loaded)return;
    const current=loaded.resources.find(item=>item.id===resourceId());
    if(current && current.workspace_type===kind())setResource(previous=>previous?.id===current.id && previous.revision>current.revision?previous:current);
  });
  createEffect(()=>{
    const current=resource(), definition=contribution();if(!current||!definition)return;
    const checkpoint=host.read("workspace/fields");
    if(checkpoint?.schema===1 && checkpoint.data && typeof checkpoint.data==="object" && !Array.isArray(checkpoint.data)){
      const saved=checkpoint.data as Record<string,unknown>;
      if(saved.revision!==current.revision){setConflict(true);setError("This workspace changed while these edits were pending. Copy your edits or reload before continuing.");return;}
      if(saved.values && typeof saved.values==="object" && !Array.isArray(saved.values) && Object.values(saved.values).every(value=>typeof value==="string")){
        setConflict(false);
        setValues(saved.values as Record<string,string>);return;
      }
    }
    setConflict(false);
    setValues(workspaceInputs(definition.spec.fields,current.data));
  });
  const refresh=()=>loader.reload();
  const returned=()=>{if(document.visibilityState==="visible")refresh();};
  window.addEventListener("focus",refresh);document.addEventListener("visibilitychange",returned);
  const timer=setInterval(refresh,5000);
  onCleanup(()=>{clearInterval(timer);window.removeEventListener("focus",refresh);document.removeEventListener("visibilitychange",returned);});
  const update=(path:string,value:string)=>{
    edits++;setValues(current=>({...current,[path]:value}));
    host.checkpoint("workspace/fields",{schema:1,summary:Object.entries(values()).map(([path,value])=>`${contribution()?.spec.fields.find(field=>field.path===path)?.label??"Field"}: ${value}`).join("\n"),data:{revision:resource()?.revision??0,values:values()}});
  };
  const act=async(action:"command"|"undo"|"redo",command?:api.WorkspaceSpec["commands"][number])=>{
    const current=resource(),definition=contribution(),id=session();if(!current||!definition||!id||busy()||conflict())return;
    setBusy(true);setError("");const generation=edits;
    job=new AbortController();const running=job;
    const cancelled=()=>running.abort();host.signal.addEventListener("abort",cancelled,{once:true});
    try{
      let argumentsValue=command?workspaceArguments(definition.spec,command,values()):{};
      if(command?.compute){
        const module=await api.workspaceModule(id,{workspace_type:definition.spec.id,digest:definition.digest,generation:definition.generation,command:command.id},running.signal);
        const output=await computeWorkspace(module,{state:current.data,arguments:argumentsValue,command:command.id},running.signal);
        if(!output||typeof output!=="object"||Array.isArray(output))throw new Error("The computation must return command arguments.");
        argumentsValue=output as Record<string,unknown>;
      }
      const saved=await api.changeWorkspace(id,{action,resource:current.id,revision:current.revision,digest:definition.digest,generation:definition.generation,...(command?{command:command.id,arguments:argumentsValue}:{})},running.signal);
      if(running.signal.aborted)return;
      batch(()=>{
        const accepted=acceptWorkspaceSave(definition.spec,values(),saved.data,command?.edits.map(edit=>edit.target),generation===edits);
        setValues(accepted.values);
        if(accepted.pending)host.checkpoint("workspace/fields",{schema:1,summary:"Pending workspace edits",data:{revision:saved.revision,values:accepted.values}});
        else host.clear("workspace/fields");
        setResource(saved);
      });
    }catch(cause){if(!host.signal.aborted)setError(cause instanceof Error?cause.message:"This workspace command failed.");}finally{host.signal.removeEventListener("abort",cancelled);running.abort();if(job===running)job=undefined;setBusy(false);}
  };
  const ask=async()=>{
    const current=resource(),definition=contribution(),id=session();if(!current||!definition||!id||!prompt().trim()||busy())return;
    setBusy(true);setError("");try{await api.askWorkspace(id,{resource:current.id,revision:current.revision,digest:definition.digest,generation:definition.generation,prompt:prompt(),context:shared()});setPrompt("");}catch(cause){setError(cause instanceof Error?cause.message:"The request could not be sent.");}finally{setBusy(false);}
  };
  return <section class="canvas-installed-workspace">
    <Show when={loader.error()}><p role="alert">{loader.error()}</p><button onClick={refresh}>Retry</button></Show>
    <Show when={!loader.loading() && !contribution()}><p role="status">This workspace plugin is unavailable. Its saved work and pending edits are kept.</p><button onClick={refresh}>Check again</button></Show>
    <Show when={contribution() && resource()} fallback={loader.loading()?<p>Loading workspace…</p>:undefined}>
      <header><h2>{contribution()?.spec.title}</h2><p>{contribution()?.spec.description}</p></header>
      <div class="canvas-workspace-actions"><button disabled={busy()||!resource()?.undo.length||!!host.read("workspace/fields")} onClick={()=>void act("undo")}>Undo</button><button disabled={busy()||!resource()?.redo.length||!!host.read("workspace/fields")} onClick={()=>void act("redo")}>Redo</button><button disabled={busy()} onClick={refresh}>Refresh</button></div>
      <Show when={error()}><p role="alert">{error()}</p></Show><Show when={conflict()}><button onClick={()=>{host.clear("workspace/fields");setError("");}}>Discard pending edits and use saved values</button></Show>
      <div class="canvas-workspace-fields"><For each={contribution()?.spec.fields}>{field=><label>{field.label}{field.unit?` (${field.unit})`:""}<Show when={field.kind!=="json"} fallback={<textarea value={values()[field.path]??""} onInput={event=>update(field.path,event.currentTarget.value)}/>}><Show when={field.kind!=="boolean"} fallback={<input type="checkbox" checked={values()[field.path]==="true"} onChange={event=>update(field.path,String(event.currentTarget.checked))}/>}><input type={field.kind==="number"?"number":"text"} step="any" value={values()[field.path]??""} onInput={event=>update(field.path,event.currentTarget.value)}/></Show></Show></label>}</For></div>
      <div class="canvas-workspace-actions"><For each={contribution()?.spec.commands}>{command=><button disabled={busy()||conflict()} onClick={()=>void act("command",command)}>{command.title}</button>}</For></div>
      <section><h3>Ask Agent</h3><p>Choose the saved fields to share. Pending edits stay here until applied.</p><For each={contribution()?.spec.context}>{path=><label><input type="checkbox" checked={shared().includes(path)} onChange={event=>setShared(current=>event.currentTarget.checked?[...current,path]:current.filter(item=>item!==path))}/>{contribution()?.spec.fields.find(field=>field.path===path)?.label??"Workspace value"}<pre>{JSON.stringify(pointer(resource()?.data,path),null,2)}</pre></label>}</For><textarea aria-label="Ask Agent about this workspace" value={prompt()} onInput={event=>setPrompt(event.currentTarget.value)}/><button disabled={busy()||!prompt().trim()} onClick={()=>void ask()}>Send request</button></section>
    </Show>
  </section>;
}
