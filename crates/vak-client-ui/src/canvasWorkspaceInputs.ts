import type { WorkspaceSpec } from "./api";
export function workspacePointer(value:unknown,path:string):unknown {
  return path.split("/").slice(1).reduce<unknown>((current,part)=>{
    const key=part.replace(/~1/g,"/").replace(/~0/g,"~");
    return current && typeof current==="object" && Object.hasOwn(current,key)?(current as Record<string,unknown>)[key]:undefined;
  },value);
}
export function workspaceInputs(fields:WorkspaceSpec["fields"],data:unknown):Record<string,string>{
  return Object.fromEntries(fields.map(field=>{const value=workspacePointer(data,field.path);return [field.path,typeof value==="string"?value:JSON.stringify(value)??""];}));
}
export function workspaceArguments(spec:WorkspaceSpec,command:WorkspaceSpec["commands"][number],values:Record<string,string>):Record<string,unknown>{
  const output:Record<string,unknown>={};
  for(const binding of command.edits){
    const field=spec.fields.find(field=>field.path===binding.target);if(!field)throw new Error("This command needs an unsupported input field.");
    let value:unknown=values[field.path];
    if(field.kind!=="text"){try{value=JSON.parse(String(value));}catch{throw new Error(`Enter a valid ${field.kind==="number"?"number":"value"} for ${field.label}.`);}}
    const parts=binding.argument.split("/").slice(1).map(part=>part.replace(/~1/g,"/").replace(/~0/g,"~"));
    if(!parts.length||parts.some(part=>!part||["__proto__","constructor","prototype"].includes(part)))throw new Error("This command has an invalid argument address.");
    let target=output;
    for(const part of parts.slice(0,-1)){
      if(!Object.hasOwn(target,part))target[part]={};
      const next=target[part];if(!next||typeof next!=="object"||Array.isArray(next))throw new Error("Command argument addresses overlap.");
      target=next as Record<string,unknown>;
    }
    if(Object.hasOwn(target,parts.at(-1)!))throw new Error("Command argument addresses overlap.");
    target[parts.at(-1)!]=value;
  }
  return output;
}
/** A partial command saves only its targets; other pending fields and input
 * typed during the request remain edits based on the new saved revision. */
export function acceptWorkspaceSave(spec:WorkspaceSpec,values:Record<string,string>,data:unknown,targets:readonly string[]|undefined,sameGeneration:boolean){
  const canonical=workspaceInputs(spec.fields,data);
  const next={...values};
  if(sameGeneration)for(const field of spec.fields)if(!targets||targets.includes(field.path))next[field.path]=canonical[field.path];
  return {values:next,pending:spec.fields.some(field=>next[field.path]!==canonical[field.path])};
}
