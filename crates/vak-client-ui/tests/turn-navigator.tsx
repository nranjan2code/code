import { render } from 'solid-js/web';
import ChatPane from '../src/components/ChatPane';
import Sidebar from '../src/components/Sidebar';
import * as store from '../src/store';
import type { Message, OutputItem } from '../src/types';
import '../src/styles.css';
const sid='navigator-fixture';
const params=new URLSearchParams(location.search);
document.documentElement.dataset.theme=params.get('theme')??'light';
window.fetch=async()=>new Response(JSON.stringify({topics:[],placements:[],automatic:true}),{headers:{'Content-Type':'application/json'}});
const messages:Message[]=[];const entries:{entry_id:string}[]=[];const items:OutputItem[]=[];
for(let i=1;i<=100;i++){
 messages.push({role:'user',content:[{type:'text',text:`Request ${i}`}]},{role:'assistant',content:[{type:'text',text:`Answer ${i}`} ]});
 entries.push({entry_id:`request-${i}`},{entry_id:`answer-${i}`});
 items.push({id:`item-${i}`,turn_id:`turn-${i}`,role:'user',kind:'message',provenance:{entry_id:`request-${i}`},content:{type:'document',document:{schema_version:2,source_markdown:`Request ${i}`,blocks:[{type:'paragraph',id:`p-${i}`,content:[{type:'text',text:`Request ${i}`}]}],coverage:[],metadata:{},diagnostics:[]}}} as OutputItem);
}
store.setSessions([{session_id:sid,agent:{id:"vak",name:"Vakyartha",revision:1},title:"hi",updated_at:"2026-10-22T00:00:00Z"}]);
store.setActiveId(sid);store.hydrateFromTranscript(sid,messages,entries,false);
store.hydrateFromPresentation(sid,{schema_version:2,session_id:sid,items,diagnostics:[],answers:items.map((item,i)=>({turn_id:item.turn_id,sections:[{name:`Reading club result ${i+1}`,form:'text',items:[]}]}))});
render(()=><div style={{height:'100vh',display:'flex'}}>{params.has('sidebar')&&<Sidebar/>}<div style={{flex:1,display:'flex',"flex-direction":'column',"min-width":0}}><ChatPane sessionId={sid}/></div></div>,document.getElementById('root')!);
