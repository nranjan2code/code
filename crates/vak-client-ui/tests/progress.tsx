import { render } from "solid-js/web";
import AgentPresence from "../src/components/AgentPresence";
import DetailsPanel from "../src/components/DetailsPanel";
import ChatPane from "../src/components/ChatPane";
import * as store from "../src/store";
import type { OutputItem, WorkProgressItem } from "../src/types";
import "../src/styles.css";
const params=new URLSearchParams(location.search);
document.documentElement.dataset.theme=params.get("theme")??"light";
window.fetch=async(input)=>{const url=String(input);const body=url.includes("/commitments")?{commitments:[],priorities:[]}:url.includes("/work")?null:{topics:[],placements:[],automatic:true};return new Response(JSON.stringify(body),{headers:{"Content-Type":"application/json"}});};
const sid="progress-fixture";
const items:WorkProgressItem[]=[
 {label:"Read the supplied material",state:"done",reason:null},
 {label:"Compare the alternatives and check the calculations",state:"in_progress",reason:null},
 {label:"Write the recommendation",state:"pending",reason:null},
 {label:"Check an unavailable external source",state:"dropped",reason:"The source is unavailable; the recommendation will state that evidence gap."},
];
const user={timestamp:"2026-10-22T00:00:00Z",actions:[],fallback_text:"Compare the supplied options.",id:"user",turn_id:"request",role:"user",kind:"message",content:{type:"document",document:{schema_version:2,blocks:[{type:"paragraph",id:"u",content:[{type:"text",text:"Compare the supplied options."}]}],source_markdown:"Compare the supplied options."}},provenance:{entry_id:"request"}} as OutputItem;
const progress=(items:WorkProgressItem[])=>({timestamp:"2026-10-22T00:00:00Z",actions:[],fallback_text:"",id:"progress-request",turn_id:"request",role:"tool",kind:"progress",status:"succeeded",content:{type:"work_progress",progress:{items}}}) as OutputItem;
store.setSessions([{session_id:sid,title:"Progress acceptance",updated_at:"2026-10-22T00:00:00Z"}]);store.setActiveId(sid);
store.hydrateFromTranscript(sid,[{role:"user",content:[{type:"text",text:"Compare the supplied options."}]}],[{entry_id:"request"}],false);
store.hydrateFromPresentation(sid,{schema_version:2,session_id:sid,items:[user,progress(items)],diagnostics:[]});
store.markRunning(sid,true);
const finish=()=>store.applyPresentationEvent(sid,{sequence:1,snapshot:null,delta:{type:"item_replaced",item:progress(items.map(item=>item.state==="in_progress"?{...item,state:"done"}:item))}});
render(()=><main style={{height:"100vh",display:"flex","flex-direction":"column"}}>
<style>{`.progress-fixture-body {display:flex;flex:1;min-height:0}.progress-fixture-chat {flex:1;min-width:0}.progress-fixture-details {width:390px;max-width:100%;height:100%}@media(max-width:700px){.progress-fixture-chat{display:none}.progress-fixture-details{width:100%}}`}</style>
<header style={{height:"70px",display:"flex","justify-content":"center","align-items":"center",position:"relative","z-index":10}}><AgentPresence sessionId={sid} working={true}/></header>
<div class="progress-fixture-body"><div class="progress-fixture-chat"><ChatPane sessionId={sid}/></div><aside class="progress-fixture-details"><DetailsPanel/></aside></div><button class="btn" onClick={finish}>Finish current step</button>
</main>,document.getElementById("root")!);
