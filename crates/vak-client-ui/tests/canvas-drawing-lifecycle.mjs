import assert from "node:assert/strict";
import { createCanvasDrawingSurface } from "../src/canvasDrawing.ts";

// Drive the production scheduler with explicit clock/visibility/observer events.
// This verifies resource lifetime, not browser layout or presented frame rate.
const originals = new Map(["window","document","requestAnimationFrame","cancelAnimationFrame","ResizeObserver","IntersectionObserver","matchMedia"].map(key=>[key,Object.getOwnPropertyDescriptor(globalThis,key)]));
const frames = new Map(); let nextFrame = 0;
let resize; let intersection; let density;
let resizeDisconnected = false, intersectionDisconnected = false;
const doc = new EventTarget(); doc.visibilityState = "visible";
const canvas = new EventTarget(); canvas.width = 0; canvas.height = 0;
canvas.getBoundingClientRect = () => ({width:200.5,height:100.25});
const replace = (key,value) => Object.defineProperty(globalThis,key,{value,writable:true,configurable:true});
replace("window",{devicePixelRatio:1.25}); replace("document",doc);
replace("requestAnimationFrame",fn=>{const id=++nextFrame;frames.set(id,fn);return id;});
replace("cancelAnimationFrame",id=>frames.delete(id));
replace("ResizeObserver",class {constructor(fn){resize=fn;}observe(){}disconnect(){resizeDisconnected=true;}});
replace("IntersectionObserver",class {constructor(fn){intersection=fn;}observe(){}disconnect(){intersectionDisconnected=true;}});
replace("matchMedia",()=>({addEventListener(_type,fn){density=fn;},removeEventListener(){}}));
const flush = time => { const current=[...frames.values()]; frames.clear(); for(const fn of current) fn(time); };
try {
  let continuous = true;
  const drawn=[]; const errors=[];
  const surface=createCanvasDrawingSurface(canvas,frame=>drawn.push(frame),{continuous:()=>continuous,onError:error=>errors.push(error)});
  flush(100); flush(108);
  assert.equal(drawn.length,1);
  assert.equal(canvas.width,251); assert.equal(canvas.height,125);
  // Many invalidations and resize signals coalesce into one drawing callback.
  for(let i=0;i<30;i++) surface.invalidate();
  flush(116); assert.equal(drawn.length,2); assert.equal(drawn[1].delta,8);
  doc.visibilityState="hidden";doc.dispatchEvent(new Event("visibilitychange"));
  flush(1000); assert.equal(drawn.length,2); assert.equal(frames.size,0);
  doc.visibilityState="visible";doc.dispatchEvent(new Event("visibilitychange"));
  flush(1016); assert.equal(drawn.at(-1).delta,0,"resume must not advance by the whole hidden interval");
  intersection([{isIntersecting:false}]);flush(1024);assert.equal(frames.size,0);
  intersection([{isIntersecting:true}]);flush(1032);assert.equal(drawn.at(-1).delta,0);
  const lost=new Event("webglcontextlost",{cancelable:true});canvas.dispatchEvent(lost);
  assert.equal(lost.defaultPrevented,true);flush(1040);const beforeRestore=drawn.length;assert.equal(frames.size,0);
  canvas.dispatchEvent(new Event("webglcontextrestored"));flush(1048);assert.equal(drawn.length,beforeRestore+1);
  continuous=false;flush(1056);const idle=drawn.length;assert.equal(frames.size,0,"static work does not keep an animation loop alive");
  surface.invalidate();flush(1064);assert.equal(drawn.length,idle+1);assert.equal(frames.size,0);
  window.devicePixelRatio=2;density();resize([{contentRect:{width:300.25,height:120.5}}]);
  flush(1072);flush(1080);assert.equal(canvas.width,601);assert.equal(canvas.height,241);
  surface.dispose();surface.invalidate();resize([{contentRect:{width:10,height:10}}]);doc.dispatchEvent(new Event("visibilitychange"));
  flush(2000);assert.equal(frames.size,0);assert.equal(resizeDisconnected,true);assert.equal(intersectionDisconnected,true);assert.deepEqual(errors,[]);
  let attempts=0;
  const broken=createCanvasDrawingSurface(canvas,()=>{attempts++;throw new Error("GPU unavailable");},{continuous:true,onError:error=>errors.push(error)});
  flush(2010);flush(2020);assert.equal(attempts,1);assert.equal(errors.length,1);assert.equal(frames.size,0,"failed draw does not spin or throw outside the renderer boundary");
  broken.dispose();
  console.log("Canvas scheduling: idle, coalescing, visibility, context loss, DPI, disposal and failure containment passed");
} finally {
  for(const [key,descriptor]of originals) {if(descriptor)Object.defineProperty(globalThis,key,descriptor);else delete globalThis[key];}
}
