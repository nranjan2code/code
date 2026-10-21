import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import Icon from "../src/components/Icon";
import "../src/styles.css";
const params=new URLSearchParams(location.search);
document.documentElement.dataset.theme=params.get("theme")??"light";
document.documentElement.style.setProperty("--text-scale",params.get("scale")??"1");
const [on,setOn]=createSignal(false);
const [chosen,setChosen]=createSignal("Preview");
render(()=><main style="height:100dvh;overflow:auto;padding:24px;max-width:1100px;margin:auto">
  <h1 style="font-size:22px;margin:0 0 8px">Vakyartha controls</h1><p style="margin:0 0 20px;color:var(--muted)">Compact desktop controls, a clear primary action, quiet secondary actions and readable fields.</p>
  <section class="controls-sample" style="display:grid;grid-template-columns:minmax(0,1fr);gap:16px;font-size:16px;line-height:1.6">
    <div class="controls-row" style="display:flex;align-items:center;flex-wrap:wrap;gap:8px"><button id="primary" class="btn primary" onClick={()=>setOn(!on())}>Save changes</button><button id="secondary" class="btn">Cancel</button><button id="compact" class="btn sm">Refresh</button><button class="btn danger">Delete</button><button class="btn" disabled>Unavailable</button><button id="icon" class="icon-button" aria-label="More options" title="More options"><Icon name="more" size={16}/></button></div>
    <div class="controls-row" style="display:flex;gap:8px;flex-wrap:wrap"><button class="workspace-details-button"><Icon name="grid" size={16}/>Today</button><button class="workspace-details-button"><Icon name="tag" size={16}/>Topics</button><button class="settings-button">Connect account</button><button class="artifact-canvas-btn">Open in Canvas</button></div>
    <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><div class="artifact-canvas-segmented">{["Preview","Code"].map(label=><button class="artifact-canvas-seg-btn" classList={{active:chosen()===label}} onClick={()=>setChosen(label)}>{label}</button>)}</div><button class="chip on">All files</button><button class="chip">Documents</button><button class="filter-button">Recent</button></div>
    <div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><label style="display:grid;gap:5px;font-size:14px">Name<input id="field" value="Quarterly report" /></label><label style="display:grid;gap:5px;font-size:14px">Choice<select id="select"><option>Automatic</option><option>Manual</option></select></label><label style="display:flex;align-items:center;gap:8px;font-size:14px"><input id="check" type="checkbox"/>Keep a copy</label><button class="switch" role="switch" aria-checked={on()} aria-label="Notifications" classList={{on:on()}} onClick={()=>setOn(!on())}><span/></button></div>
    <div><div class="result-card-actions"><button class="btn primary">Review</button><button class="btn">Open</button><button class="file-action-icon" aria-label="File history"><Icon name="history" size={16}/></button><button class="card-button">Download CSV</button></div></div>
    <div style="width:300px;max-width:100%;container-type:inline-size"><div class="workbench-panel" style="height:auto"><button id="narrow-control" class="btn">Narrow companion pane</button></div></div>
    <nav style="display:grid;width:220px;gap:4px;border-top:1px solid var(--border-soft);padding-top:12px"><button class="sb-nav-item"><Icon name="search" size={16}/><span>Search</span></button><button class="sb-nav-item active"><Icon name="folder" size={16}/><span>Library</span></button></nav>
    <label style="display:grid;gap:6px;font-size:14px">Notes<textarea rows={3} placeholder="Add a comment…"/></label>
  </section>
</main>,document.getElementById("root")!);
Object.assign(window,{runChecks:()=>{
 const passed:string[]=[];const check=(ok:unknown,label:string)=>{if(!ok)throw new Error(label);passed.push(label)};
 const coarse=matchMedia("(pointer: coarse)").matches;const primary=document.querySelector<HTMLButtonElement>("#primary")!;const secondary=document.querySelector<HTMLButtonElement>("#secondary")!;
 const base=getComputedStyle(secondary);check(parseFloat(base.fontSize)===Math.max(12,14*Number(params.get("scale")??1)),"Control labels use control typography inside reading-sized content");
 check(secondary.getBoundingClientRect().height>=(coarse?40:28),"Button hit area follows pointer density");check(coarse||secondary.getBoundingClientRect().height<=40,"Desktop buttons stay compact even with enlarged reading text");
 const narrowHeight=document.querySelector("#narrow-control")!.getBoundingClientRect().height;check(coarse?narrowHeight===44:narrowHeight>=32&&narrowHeight<=40,"A narrow companion pane does not imply touch-sized desktop controls");
 check(base.borderTopColor==="rgba(0, 0, 0, 0)","Secondary actions do not introduce another outlined box");check(getComputedStyle(primary).backgroundColor!==base.backgroundColor,"Primary action has a distinct emphasis");
 for(const field of document.querySelectorAll<HTMLElement>("input:not([type=checkbox]),select")){check(field.getBoundingClientRect().height>=(coarse?40:28),`${field.tagName}: field density follows the shared control system`)}
 for(const el of document.querySelectorAll<HTMLElement>("button,input,select,textarea")){check(parseFloat(getComputedStyle(el).fontSize)>=12,"Controls preserve the readable type floor");}
 document.querySelector<HTMLButtonElement>(".switch")!.click();check(document.querySelector('.switch')?.getAttribute('aria-checked')==="true","Switch remains keyboard/assistive-technology stateful");
 document.querySelectorAll<HTMLButtonElement>(".artifact-canvas-seg-btn")[1].click();check(document.querySelectorAll(".artifact-canvas-seg-btn")[1].classList.contains("active"),"Segmented selection remains explicit");
 secondary.focus();check(document.activeElement===secondary,"Quiet secondary controls retain keyboard focus");
 check(document.querySelector("main")!.scrollWidth<=document.querySelector("main")!.clientWidth+1,"Controls do not widen their scrolling surface");return passed;
}});
