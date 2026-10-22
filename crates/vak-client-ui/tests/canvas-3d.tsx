import { createEffect, createSignal, onMount, onCleanup } from "solid-js";
import { render } from "solid-js/web";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import { useCanvasWorkspace } from "../src/components/canvas/WorkspaceContext";
import { createCanvasDrawingSurface } from "../src/canvasDrawing";
import { registerCanvasWorkspace } from "../src/canvasWorkspaces";
import * as store from "../src/store";
import "../src/styles.css";
const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") === "dark" ? "dark" : "light";
const multiply = (a: number[], b: number[]) => Array.from({ length: 16 }, (_, i) => {
  const row = i % 4, col = Math.floor(i / 4);
  return a[row] * b[col * 4] + a[row + 4] * b[col * 4 + 1] + a[row + 8] * b[col * 4 + 2] + a[row + 12] * b[col * 4 + 3];
});
const rotation = (x: number, y: number) => multiply(
  [1,0,0,0,0,Math.cos(x),Math.sin(x),0,0,-Math.sin(x),Math.cos(x),0,0,0,0,1],
  [Math.cos(y),0,-Math.sin(y),0,0,1,0,0,Math.sin(y),0,Math.cos(y),0,0,0,0,1]);
const transposeVector = (m: number[], v: number[]) => [0,4,8].map(at => m[at]*v[0]+m[at+1]*v[1]+m[at+2]*v[2]);
const [report, setReport] = createSignal("Real WebGL2 mesh, lighting, depth, ray selection and animation. Synthetic renderer; no scientific simulation is claimed.");
const gaps: number[] = [], costs: number[] = [];
let measure = false;
function Scene() {
  const host = useCanvasWorkspace()!;
  const saved = host.read("scene/material");
  const [material, setMaterial] = createSignal(typeof saved?.data === "number" ? saved.data : 0.6);
  const [animated, setAnimated] = createSignal(!matchMedia("(prefers-reduced-motion: reduce)").matches);
  const [wireframe, setWireframe] = createSignal(false), [selected, setSelected] = createSignal(false);
  let canvas!: HTMLCanvasElement, invalidate = () => {};
  let pitch = 0.4, yaw = 0.6, distance = 6, drag: {x:number;y:number;px:number;py:number} | null = null;
  let lastRotation = rotation(pitch,yaw);
  createEffect(() => {
    const current = host.read("scene/material");
    if (current?.schema === 1 && typeof current.data === "number") {
      setMaterial(current.data); invalidate();
    }
  });
  onMount(() => {
    const gl = canvas.getContext("webgl2", { antialias:true, alpha:false });
    if (!gl) { setReport("WebGL2 is unavailable on this runtime; the host remains usable."); return; }
    const shader = (type:number, source:string) => {
      const shader = gl.createShader(type); if (!shader) throw new Error("Shader allocation failed");
      gl.shaderSource(shader,source); gl.compileShader(shader);
      if (!gl.getShaderParameter(shader,gl.COMPILE_STATUS)) { gl.deleteShader(shader); throw new Error("Shader compilation failed"); }
      return shader;
    };
    let disposeGpu = () => {}, paint: ((width:number,height:number)=>void) | null = null;
    const setup = () => {
      disposeGpu();
      const vert = shader(gl.VERTEX_SHADER,`#version 300 es
        in vec3 position; in vec3 normal; uniform mat4 mvp; uniform mat4 model; out vec3 n;
        void main(){gl_Position=mvp*vec4(position,1.);n=mat3(model)*normal;}`);
      const frag = shader(gl.FRAGMENT_SHADER,`#version 300 es
        precision highp float; in vec3 n; uniform float material; uniform bool selected; out vec4 color;
        void main(){float light=.22+.78*max(dot(normalize(n),normalize(vec3(.5,.8,1.))),0.);
        vec3 base=mix(vec3(.18,.28,.68),vec3(.86,.58,.18),material); if(selected)base=mix(base,vec3(1.),.2);color=vec4(base*light,1.);}`);
      const program=gl.createProgram(); if(!program)throw new Error("Program allocation failed");
      gl.attachShader(program,vert);gl.attachShader(program,frag);gl.linkProgram(program);
      if(!gl.getProgramParameter(program,gl.LINK_STATUS))throw new Error("Program link failed");
      const faces=[
        [-1,-1,1,1,-1,1,1,1,1,-1,1,1,0,0,1], [1,-1,-1,-1,-1,-1,-1,1,-1,1,1,-1,0,0,-1],
        [-1,1,1,1,1,1,1,1,-1,-1,1,-1,0,1,0], [-1,-1,-1,1,-1,-1,1,-1,1,-1,-1,1,0,-1,0],
        [1,-1,1,1,-1,-1,1,1,-1,1,1,1,1,0,0], [-1,-1,-1,-1,-1,1,-1,1,1,-1,1,-1,-1,0,0]];
      const vertices=faces.flatMap(face=>Array.from({length:4},(_,i)=>[...face.slice(i*3,i*3+3),...face.slice(12)]).flat());
      const solid=faces.flatMap((_,i)=>[0,1,2,0,2,3].map(n=>i*4+n));
      const edges=faces.flatMap((_,i)=>[0,1,1,2,2,3,3,0].map(n=>i*4+n));
      const vao=gl.createVertexArray(), vb=gl.createBuffer(), ib=gl.createBuffer();
      gl.bindVertexArray(vao);gl.bindBuffer(gl.ARRAY_BUFFER,vb);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(vertices),gl.STATIC_DRAW);
      for(const [name,offset]of [["position",0],["normal",12]] as const){const at=gl.getAttribLocation(program,name);gl.enableVertexAttribArray(at);gl.vertexAttribPointer(at,3,gl.FLOAT,false,24,offset);}
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,ib);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint16Array([...solid,...edges]),gl.STATIC_DRAW);
      const mvp=gl.getUniformLocation(program,"mvp"),model=gl.getUniformLocation(program,"model"),color=gl.getUniformLocation(program,"material"),select=gl.getUniformLocation(program,"selected");
      disposeGpu=()=>{gl.deleteBuffer(vb);gl.deleteBuffer(ib);gl.deleteVertexArray(vao);gl.deleteProgram(program);gl.deleteShader(vert);gl.deleteShader(frag);};
      paint=(width,height)=>{
        const f=1/Math.tan(Math.PI/8),near=.1,far=100;
        const projection=[f/(width/height),0,0,0,0,f,0,0,0,0,(far+near)/(near-far),-1,0,0,2*far*near/(near-far),0];
        const view=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,-distance,1];
        lastRotation=rotation(pitch,yaw);
        gl.viewport(0,0,width,height);gl.enable(gl.DEPTH_TEST);gl.clearColor(.085,.09,.105,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);
        gl.useProgram(program);gl.bindVertexArray(vao);gl.uniformMatrix4fv(mvp,false,new Float32Array(multiply(projection,multiply(view,lastRotation))));gl.uniformMatrix4fv(model,false,new Float32Array(lastRotation));
        gl.uniform1f(color,material());gl.uniform1i(select,selected()?1:0);
        gl.drawElements(wireframe()?gl.LINES:gl.TRIANGLES,wireframe()?edges.length:solid.length,gl.UNSIGNED_SHORT,wireframe()?solid.length*2:0);
      };
    };
    setup();
    const restored=()=>{try{setup();}catch(error){setReport(`FAILED: ${String(error)}`);}};
    canvas.addEventListener("webglcontextrestored",restored);
    const surface=createCanvasDrawingSurface(canvas,frame=>{if(animated()&&!drag)yaw+=frame.delta*.0004;paint?.(frame.pixelWidth,frame.pixelHeight);},
      {continuous:animated,onError:error=>setReport(`FAILED: ${String(error)}`),onFrame:timing=>{if(measure&&timing.interval>0){gaps.push(timing.interval);costs.push(timing.paint);}}});
    invalidate=surface.invalidate;
    onCleanup(()=>{surface.dispose();disposeGpu();canvas.removeEventListener("webglcontextrestored",restored);});
  });
  const pick=(event:PointerEvent)=>{
    const rect=canvas.getBoundingClientRect(),f=Math.tan(Math.PI/8);
    const origin=transposeVector(lastRotation,[0,0,distance]);
    const direction=transposeVector(lastRotation,[(2*(event.clientX-rect.left)/rect.width-1)*rect.width/rect.height*f,(1-2*(event.clientY-rect.top)/rect.height)*f,-1]);
    let low=0,high=Infinity;
    for(let i=0;i<3;i++){if(Math.abs(direction[i])<1e-9){if(Math.abs(origin[i])>1){high=-1;break;}continue;}const a=(-1-origin[i])/direction[i],b=(1-origin[i])/direction[i];low=Math.max(low,Math.min(a,b));high=Math.min(high,Math.max(a,b));}
    setSelected(high>=low);invalidate();
  };
  return <section style="height:100%;min-height:0;display:flex;flex-direction:column">
    <div style="display:flex;gap:12px;flex-wrap:wrap;padding:12px;align-items:center">
      <label><input type="checkbox" checked={animated()} onChange={event=>{setAnimated(event.currentTarget.checked);invalidate();}}/> Animate</label>
      <label><input type="checkbox" checked={wireframe()} onChange={event=>{setWireframe(event.currentTarget.checked);invalidate();}}/> Wireframe</label>
      <label>Material <input aria-label="Material" type="range" min="0" max="1" step="0.01" value={material()} onInput={event=>{setMaterial(Number(event.currentTarget.value));host.checkpoint("scene/material",{schema:1,summary:`Material mix ${material()}`,data:material()});invalidate();}}/></label>
      <button class="btn" onClick={()=>void sample()}>Measure 3D</button>
      <button class="btn" onClick={()=>host.clear("scene/material")}>Mark saved</button>
      <span role="status">{selected()?"Cube selected":"Click the cube to select"}</span>
    </div>
    <canvas ref={canvas} aria-label="Lit interactive 3D cube" style="width:100%;height:100%;flex:1;min-height:0;display:block;touch-action:none"
      onPointerDown={event=>{if(event.button!==0)return;canvas.setPointerCapture(event.pointerId);drag={x:event.clientX,y:event.clientY,px:pitch,py:yaw};}}
      onPointerMove={event=>{if(drag){pitch=drag.px+(event.clientY-drag.y)*.008;yaw=drag.py+(event.clientX-drag.x)*.008;invalidate();}}}
      onPointerUp={event=>{if(drag&&Math.hypot(event.clientX-drag.x,event.clientY-drag.y)<3)pick(event);drag=null;if(canvas.hasPointerCapture(event.pointerId))canvas.releasePointerCapture(event.pointerId);invalidate();}}
      onPointerCancel={()=>{drag=null;invalidate();}}
      onWheel={event=>{event.preventDefault();distance=Math.max(3,Math.min(20,distance*Math.exp(event.deltaY*.001)));invalidate();}}/>
    <output style="padding:8px;font-size:12px;white-space:pre-wrap;max-height:120px;overflow:auto">{report()}</output>
  </section>;
}
const unregister = registerCanvasWorkspace({id:"vak/scene-probe",spec:{badge:"3D workspace",views:[],defaultView:()=>null,layout:"focused",reloadable:false,devices:false,popout:false,feedback:"none",selects:()=>null,safetyFooter:false},component:Scene});
store.setActiveId("3d-workspace-probe");
window.fetch=async()=>new Response(JSON.stringify({records:[]}),{headers:{"Content-Type":"application/json"}});
render(()=> <ArtifactCanvas/>,document.getElementById("root")!);
store.openArtifactCanvas({kind:"workspace",title:"Interactive 3D · synthetic extension probe",workspaceType:"vak/scene-probe",resourceId:"synthetic-mesh",revision:"fixture",sessionId:"3d-workspace-probe"});
const percentile=(values:number[],p:number)=>[...values].sort((a,b)=>a-b)[Math.floor((values.length-1)*p)]??0;
async function sample(){
  gaps.length=0;costs.length=0;measure=true;
  for(let i=0;i<120;i++)await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
  measure=false;const canvas=document.querySelector("canvas")!;
  const result={viewport:[innerWidth,innerHeight],dpr:devicePixelRatio,samples:gaps.length,medianIntervalMs:percentile(gaps,.5),p95IntervalMs:percentile(gaps,.95),observedRafHz:Math.round(1000/percentile(gaps,.5)),p95CpuSubmissionMs:percentile(costs,.95),backing:[canvas.width,canvas.height],css:[canvas.clientWidth,canvas.clientHeight],fidelity:canvas.width===Math.round(canvas.getBoundingClientRect().width*devicePixelRatio)&&canvas.height===Math.round(canvas.getBoundingClientRect().height*devicePixelRatio),noPageOverflow:document.documentElement.scrollWidth<=innerWidth};
  setReport(JSON.stringify(result));document.documentElement.dataset.checks=result.samples&&result.fidelity&&result.noPageOverflow?"PASS":"FAILED";
}
if(params.has("run"))void sample();

if (import.meta.hot) import.meta.hot.dispose(unregister);
