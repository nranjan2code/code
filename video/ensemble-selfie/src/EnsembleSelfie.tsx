import React from 'react';
import {AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame} from 'remotion';

const cast = [
  {name:'Vakyartha', image:'vak', task:'A dinner plan that keeps things easy.', art:'meal'},
  {name:'Moss', image:'moss', task:'A Saturday with room to breathe.', art:'calendar'},
  {name:'Beni', image:'beni', task:'The essentials, packed step by step.', art:'bag'},
  {name:'Mira', image:'mira', task:'A reply that is clear and kind.', art:'letter'},
  {name:'Pip', image:'pip', task:'A small fix for a tricky bug.', art:'laptop'},
  {name:'Nori', image:'nori', task:'A pattern hiding in the numbers.', art:'chart'},
  {name:'Lumi', image:'lumi', task:'A birthday with a little imagination.', art:'cake'},
  {name:'Tavi', image:'tavi', task:'How a tiny seed starts to grow.', art:'plant'},
];
const C = {ink:'#1b1e36', muted:'#565970', indigo:'#2f3c94', saffron:'#f5a400', line:'#dfe3ee', soft:'#f0f3fa', white:'#ffffff'};
const ease=(v:number)=>Math.max(0,Math.min(1,v));

const Fade:React.FC<{start:number;end:number;children:React.ReactNode}> = ({start,end,children}) => {
  const f=useCurrentFrame();
  const opacity=Math.min(ease((f-start)/12),ease((end-f)/12));
  return <div style={{position:'absolute',inset:0,opacity}}>{children}</div>;
};
const Portrait:React.FC<{id:string;size:number;style?:React.CSSProperties}> = ({id,size,style}) => <Img src={staticFile(`ensemble/${id}.png`)} style={{width:size,height:size,objectFit:'contain',...style}}/>;

const Illustration:React.FC<{kind:string}> = ({kind}) => {
  const common={fill:'none',stroke:C.indigo,strokeWidth:5,strokeLinecap:'round' as const,strokeLinejoin:'round' as const};
  const drawings:Record<string,React.ReactNode>={
    meal:<><path d="M44 77h112l-10 49H54z"/><path d="M58 77c3-26 82-26 85 0M66 61c-9-12 8-16 1-27m33 27c-9-12 8-16 1-27m31 27c-9-12 8-16 1-27"/><path d="M45 142h112"/></>,
    calendar:<><rect x="42" y="47" width="116" height="104" rx="7"/><path d="M42 74h116M68 39v18m64-18v18M67 98h18m22 0h18M67 123h18"/><circle cx="130" cy="123" r="4" fill={C.saffron} stroke="none"/></>,
    bag:<><path d="M54 72h92l9 79H45z"/><path d="M76 72V58c0-24 48-24 48 0v14m-78 24h108"/><path d="M90 111h20"/></>,
    letter:<><path d="M39 55h122v93H39z"/><path d="m41 61 59 47 59-47M45 143l40-37m70 37-40-37"/><path d="m140 48 10-10" stroke={C.saffron}/></>,
    laptop:<><path d="M58 48h84v67H58zM43 132h114l-12 12H55z"/><path d="M79 79h42m-42 16h28"/><circle cx="132" cy="94" r="5" fill={C.saffron} stroke="none"/></>,
    chart:<><path d="M47 142h111M57 132V92h22v40m18 0V66h22v66m18 0V82h22v50"/><path d="m54 76 34-22 25 6 34-25"/><circle cx="147" cy="35" r="5" fill={C.saffron} stroke="none"/></>,
    cake:<><path d="M47 96h106v47H47zM39 143h122v12H39z"/><path d="M47 96c9-23 22 12 34-9 12-21 25 13 39-5 13-17 21 3 33 14M100 57V39m-8 2 8-12 8 12"/><path d="M100 70v8" stroke={C.saffron}/></>,
    plant:<><path d="M100 139V74m0 34c0-25-35-28-42-50 27-4 47 13 42 50Zm0-16c0-25 33-37 54-36-4 28-27 43-54 36ZM68 144h64l-9 17H77z"/><circle cx="101" cy="45" r="5" fill={C.saffron} stroke="none"/></>,
  };
  return <svg viewBox="0 0 200 190" width="205" height="190" aria-hidden="true"><g {...common}>{drawings[kind]}</g></svg>;
};

export const EnsembleSelfie:React.FC=()=>{
  const f=useCurrentFrame();
  const gather=ease((f-555)/78);
  const shutter=ease((f-632)/5)*(1-ease((f-642)/8));
  const wallpaper=ease((f-645)/10);
  return <AbsoluteFill style={{background:C.white,color:C.ink,fontFamily:'-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif',overflow:'hidden'}}>
    <Fade start={0} end={62}>
      <div style={{position:'absolute',left:92,top:197,fontSize:22,color:C.muted}}>An evening with the companions</div>
      <div style={{position:'absolute',left:88,top:248,fontSize:82,lineHeight:'.98',fontWeight:600,letterSpacing:-5}}>A little help.<br/>A lot of heart.</div>
      <div style={{position:'absolute',left:92,top:455,width:54,height:4,background:C.saffron}}/>
      <div style={{position:'absolute',left:92,top:484,fontSize:25,color:C.muted}}>Eight personalities, each in their element.</div>
      <Img src={staticFile('ensemble/dusk.jpg')} style={{position:'absolute',right:0,top:0,width:580,height:720,objectFit:'cover',clipPath:'ellipse(76% 88% at 90% 50%)'}}/>
    </Fade>
    {cast.map((c,i)=>{
      const start=60+i*60,end=start+60,p=ease((f-start)/60);
      const bob=Math.sin((f-start)*.14)*5;
      return <Fade key={c.image} start={start} end={end}>
        <div style={{position:'absolute',inset:0,background:i%2===0?C.white:C.soft}}/>
        <div style={{position:'absolute',left:96,top:104,width:5,height:46,background:C.saffron}}/>
        <div style={{position:'absolute',left:120,top:104,fontSize:21,color:C.muted}}>In their element</div>
        <div style={{position:'absolute',left:92,top:172,fontSize:68,lineHeight:1,fontWeight:600,letterSpacing:-3}}>{c.name}</div>
        <div style={{position:'absolute',left:96,top:264,width:460,fontSize:31,lineHeight:1.35,color:C.muted}}>{c.task}</div>
        <div style={{position:'absolute',left:100,top:390}}><Illustration kind={c.art}/></div>
        <div style={{position:'absolute',right:48,top:72,width:570,height:565,borderRadius:40,background:C.white,border:`1px solid ${C.line}`,boxShadow:'0 24px 60px rgba(27,30,54,.09)'}}/>
        <Portrait id={c.image} size={555} style={{position:'absolute',right:53,top:73+bob,transform:`translateX(${(1-p)*45}px)`,filter:'drop-shadow(0 18px 16px rgba(27,30,54,.13))'}}/>
      </Fade>;
    })}
    {f>=530&&f<655&&<>
      <div style={{position:'absolute',top:48,width:'100%',textAlign:'center',fontSize:24,color:C.muted,opacity:ease((f-535)/15)}}>Work can wait. Let’s get a picture.</div>
      {cast.map((c,i)=>{
        const target=38+i*150,from=i%2===0?-210:1300,x=interpolate(gather,[0,1],[from,target]);
        const jump=Math.sin((f-540-i*4)*.18)*Math.max(0,1-gather)*18;
        return <Portrait key={c.image} id={c.image} size={225} style={{position:'absolute',left:x,top:337+jump,filter:'drop-shadow(0 14px 10px rgba(27,30,54,.16))'}}/>;
      })}
      <div style={{position:'absolute',bottom:42,left:'50%',transform:'translateX(-50%)',fontSize:28,fontWeight:600,color:C.indigo,opacity:ease((f-605)/16),whiteSpace:'nowrap'}}>Together is a lovely place to be.</div>
    </>}
    {f>=640&&<Img src={staticFile('ensemble/dusk.jpg')} style={{position:'absolute',inset:0,width:'100%',height:'100%',objectFit:'cover',opacity:wallpaper,transform:`scale(${1.07-.07*ease((f-645)/60)})`}}/>}
    {f>=667&&<div style={{position:'absolute',bottom:34,width:'100%',textAlign:'center',fontSize:31,fontWeight:600,color:C.white,textShadow:'0 3px 14px rgba(0,0,0,.8)',opacity:ease((f-667)/15)}}>A moment to keep.</div>}
    <AbsoluteFill style={{background:C.white,opacity:shutter,pointerEvents:'none'}}/>
  </AbsoluteFill>;
};
