import { render } from 'solid-js/web';
import { createSignal, Show } from 'solid-js';
import Sidebar from '../src/components/Sidebar';
import ChatPane from '../src/components/ChatPane';
import TopicsPanel from '../src/components/TopicsPanel';
import * as store from '../src/store';
import { turnDisplayLabel } from '../src/turnPairing';
import type { SessionTopics } from '../src/api';
import type { OutputItem } from '../src/types';
import '../src/styles.css';
const sid='organisation-fixture';
const params=new URLSearchParams(location.search);
document.documentElement.dataset.theme=params.get('theme')??'light';
const agent={id:'vak',name:'Vakyartha',revision:1,lifecycle:'active',character:'vak',animation:'subtle',voice:'default',personality:'',behaviour:'',responsibilities:'',instructions:''};
let snapshot:SessionTopics={topics:[],placements:[],automatic:true,revision:1,organisation_pending:true,turn_names:[{turn_id:'turn-1',title:null,source:'model',status:'pending'}]};
let releaseTopics: (()=>void)|undefined;
let holdNextTopics=false;
const json=(body:unknown)=>new Response(JSON.stringify(body),{headers:{'Content-Type':'application/json'}});
window.fetch=async(input,init)=>{
 const path=new URL(String(input),location.href).pathname;
 if(path==='/agents')return json({agents:[]});
 if(path==='/agents/vak/topics'){if(holdNextTopics){holdNextTopics=false;await new Promise<void>(resolve=>{releaseTopics=resolve;});}return json({topics:snapshot.topics.map(topic=>({...topic,session_id:sid,placements:snapshot.placements})),next:null});}
 if(path===`/sessions/${sid}/topics`&&init?.method==='POST'){
  const action=JSON.parse(String(init.body));
  if(action.action==='rename_turn') snapshot={...snapshot,revision:snapshot.revision+1,turn_names:snapshot.turn_names?.map(name=>name.turn_id===action.turn_id?{...name,title:action.title,source:'human',status:'complete'}:name)};
  return json(snapshot);
 }
 if(path===`/sessions/${sid}/topics`)return json(snapshot);
 return json({});
};
store.setBackend({ready:true,cwd:'/fixture',base_url:'',token:'',port:0,pid:1});
store.setSessions([{session_id:sid,title:'Sample conversation',agent,updated_at:'2026-10-23T00:00:00Z'}]);
store.setActiveId(sid);
store.setActiveAgent(agent);
store.setSidebarOpen(true);
const messages=[{role:'user' as const,content:[{type:'text' as const,text:'Make a field notes brief.'}]},{role:'assistant' as const,content:[{type:'text' as const,text:'Created the brief. The observations and open questions are ready to review.'}]}];
messages.unshift({role:'user',content:[{type:'text',text:'Review the field survey notes.'}]},{role:'assistant',content:[{type:'text',text:'Reviewed the observations and identified the open questions.'}]});
const items=messages.map((message,i)=>({id:`item-${i}`,turn_id:`turn-${Math.floor(i/2)+1}`,role:message.role,kind:i%2?'outcome':'message',provenance:{entry_id:`entry-${i}`},fallback_text:message.content[0].text,
 content:{type:'document',document:{schema_version:2,source_markdown:message.content[0].text,blocks:[{type:'paragraph',id:`paragraph-${i}`,content:[{type:'text',text:message.content[0].text}]}],coverage:[],metadata:{},diagnostics:[]}}} as OutputItem));
store.hydrateFromTranscript(sid,messages,messages.map((_,i)=>({entry_id:`entry-${i}`})),false);
store.hydrateFromPresentation(sid,{schema_version:2,session_id:sid,items,diagnostics:[]});
store.acceptOrganisation(sid,snapshot);
const [panel,setPanel]=createSignal(params.has('panel'));
render(()=><div class="app" style="--sidebar-width:278px;--dock-width:0px"><Show when={window.innerWidth>600}><Sidebar/></Show><div class="main" style={{'min-width':'0'}}><div class="workspace-header"><strong>Vakyartha</strong><button type="button" onClick={complete}>Finish organisation</button><button type="button" onClick={()=>setPanel(!panel())}>Topics</button><button type="button" onClick={()=>{holdNextTopics=true;store.setBackend({...store.backend(),cwd:'/fixture-pending'});}}>Hold topic refresh</button><button type="button" onClick={()=>{store.setBackend({...store.backend(),cwd:'/fixture-recovered'});releaseTopics?.();}}>Switch workspace</button></div><Show when={panel()} fallback={<ChatPane sessionId={sid}/>}><TopicsPanel sessionId={sid}/></Show></div></div>,document.getElementById('root')!);
function complete(){snapshot={...snapshot,revision:snapshot.revision+1,organisation_pending:false,turn_names:[{turn_id:'turn-1',title:'Field survey review',source:'model',status:'complete'},{turn_id:'turn-2',title:'Field notes brief',source:'model',status:'complete'}],topics:[{id:'topic-field',title:'Field notes',archived:false,pinned:false}],placements:[{turn_id:'turn-1',outcome:'The brief',topic_ids:['topic-field'],source:'model',actor:'fixture',trace:{}}]};store.acceptOrganisation(sid,snapshot);}
Object.assign(window,{completeOrganisation:complete,runChecks:async()=>{
 if(turnDisplayLabel('entry-0',store.presentationOf(sid))!=='')throw new Error('pending turn invented a name');
 complete();await new Promise(resolve=>requestAnimationFrame(resolve));
 if(turnDisplayLabel('entry-0',store.presentationOf(sid))!=='Field survey review')throw new Error('name did not arrive for the plain answer');
 if(document.documentElement.scrollWidth>window.innerWidth)throw new Error('horizontal overflow');
 return 'organisation: ok';
}});
