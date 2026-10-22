import { evaluateWorkspaceModule } from "./canvasCompute";
/** Owned bootstrap only. Package WebAssembly gets no imports or Web APIs. */
self.onmessage = async (event: MessageEvent<{module:ArrayBuffer;input:unknown}>) => {
  try {self.postMessage({ok:true,value:await evaluateWorkspaceModule(new Uint8Array(event.data.module),event.data.input)});}
  catch {self.postMessage({ok:false,error:"The workspace computation could not produce a valid result."});}
};
