/** One pure calculation per worker. A timeout/abort destroys the realm and memory. */
export function computeWorkspace(module:ArrayBuffer,input:unknown,signal:AbortSignal):Promise<unknown>{
  if(signal.aborted)return Promise.reject(new Error("Workspace computation was cancelled."));
  return new Promise((resolve,reject)=>{
    const worker=new Worker(new URL("./canvasComputeWorker.ts",import.meta.url),{type:"module"});
    let finished=false;
    const finish=(value:unknown,error?:string)=>{
      if(finished)return;finished=true;
      clearTimeout(timer);signal.removeEventListener("abort",aborted);worker.terminate();
      if(error)reject(new Error(error));else resolve(value);
    };
    const aborted=()=>finish(undefined,"Workspace computation was cancelled.");
    const timer=setTimeout(()=>finish(undefined,"This calculation exceeded the interactive time budget. Run longer work through the Agent’s execution tools."),30_000);
    signal.addEventListener("abort",aborted,{once:true});
    worker.onmessage=event=>event.data?.ok===true?finish(event.data.value):finish(undefined,"The workspace computation failed.");
    worker.onerror=()=>finish(undefined,"The workspace computation failed.");
    worker.onmessageerror=()=>finish(undefined,"The workspace computation returned an invalid message.");
    try{worker.postMessage({module,input},[module]);}catch{finish(undefined,"The workspace computation could not start.");}
  });
}
