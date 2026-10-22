/** Pure computation ABI. No filesystem, network, clock, credentials or host imports. */
const MAX_PAGES = 1024; // 64 MiB per disposable UI computation.
const MAX_JSON = 2 * 1024 * 1024;

/** Reject unbounded/shared/imported memories before instantiation can allocate them. */
export function validateWorkspaceModule(bytes: Uint8Array): void {
  if (bytes.length > 16 * 1024 * 1024 || bytes.length < 8 || bytes.slice(0,8).some((byte,index)=>byte !== [0,97,115,109,1,0,0,0][index])) throw new Error("Invalid workspace computation module.");
  let offset=8;
  const unsigned=()=>{
    let value=0,shift=0;
    for(let count=0;count<5;count++){
      const byte=bytes[offset++];if(byte===undefined)throw new Error("Truncated workspace module.");
      value+=(byte&127)*2**shift;
      if(!(byte&128)){if(value>0xffffffff)throw new Error("Invalid module length.");return value;}
      shift+=7;
    }
    throw new Error("Invalid module length.");
  };
  let memories=0;
  while(offset<bytes.length){
    const section=bytes[offset++],length=unsigned(),end=offset+length;
    if(end>bytes.length)throw new Error("Truncated workspace module.");
    if(section===1){
      const types=unsigned();if(types>2048)throw new Error("Workspace computation has too many signatures.");
      for(let index=0;index<types;index++){
        if(bytes[offset++]!==0x60)throw new Error("Workspace computation uses unsupported heap types.");
        for(const maximum of [128,16]){const count=unsigned();if(count>maximum)throw new Error("Workspace signature is too large.");for(let at=0;at<count;at++)if(![0x7f,0x7e,0x7d,0x7c,0x7b].includes(bytes[offset++]))throw new Error("Workspace signature needs numeric values.");}
      }
      if(offset!==end)throw new Error("Invalid workspace function signatures.");
    }
    if(section===3 && unsigned()>8192)throw new Error("Workspace computation has too many functions.");
    if(section===2 && unsigned()!==0)throw new Error("Workspace computations cannot import host capabilities.");
    if(section===4){
      const tables=unsigned();if(tables>4)throw new Error("Too many workspace tables.");
      for(let index=0;index<tables;index++){
        const type=bytes[offset++],flags=unsigned(),initial=unsigned(),maximum=unsigned();
        if(![0x70,0x6f].includes(type)||flags!==1||initial>maximum||maximum>16_384)throw new Error("Workspace tables need bounded limits.");
      }
      if(offset!==end)throw new Error("Invalid workspace table declaration.");
    }
    if(section===5){
      memories=unsigned();if(memories!==1)throw new Error("Workspace computation needs one bounded memory.");
      const flags=unsigned(),initial=unsigned(),maximum=unsigned();
      if(flags!==1 || initial>maximum || maximum>MAX_PAGES)throw new Error("Workspace computation memory must have a declared maximum of 64 MiB or less.");
      if(offset!==end)throw new Error("Invalid workspace memory declaration.");
    }
    offset=end;
  }
  if(memories!==1)throw new Error("Workspace computation needs one bounded memory.");
}

/** Run only inside a disposable worker in the app. Exported for ABI regression tests. */
export async function evaluateWorkspaceModule(bytes: Uint8Array, input: unknown): Promise<unknown> {
  validateWorkspaceModule(bytes);
  const compiled=await WebAssembly.compile(bytes as Uint8Array<ArrayBuffer>);
  if(WebAssembly.Module.imports(compiled).length)throw new Error("Workspace computations cannot import host capabilities.");
  const instance=await WebAssembly.instantiate(compiled,{});
  const {memory,alloc,evaluate}=instance.exports;
  if(!(memory instanceof WebAssembly.Memory) || typeof alloc!=="function" || typeof evaluate!=="function")throw new Error("Workspace module must export memory, alloc and evaluate.");
  const encoded=new TextEncoder().encode(JSON.stringify(input));
  if(encoded.length>MAX_JSON)throw new Error("Workspace computation input is too large.");
  const pointer=Number(alloc(encoded.length));
  const validRange=(pointer:number,length:number)=>Number.isSafeInteger(pointer)&&pointer>=0&&Number.isSafeInteger(length)&&length>=0&&pointer+length<=memory.buffer.byteLength;
  if(!validRange(pointer,encoded.length))throw new Error("Workspace allocator returned an invalid range.");
  new Uint8Array(memory.buffer,pointer,encoded.length).set(encoded);
  const result=evaluate(pointer,encoded.length);
  if(typeof result!=="bigint" || result<0n)throw new Error("Workspace computation returned an invalid result handle.");
  const start=Number(result>>32n),length=Number(result&0xffffffffn);
  if(length>MAX_JSON || !validRange(start,length))throw new Error("Workspace computation output is too large or invalid.");
  return JSON.parse(new TextDecoder("utf-8",{fatal:true}).decode(new Uint8Array(memory.buffer,start,length)));
}
