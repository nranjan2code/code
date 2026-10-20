import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, existsSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import http from 'node:http';
const leaks=[]; const referrers=[];
const collector=http.createServer((req,res)=>{if(req.url.includes('marker=')){leaks.push(req.url);referrers.push(req.headers.referer ?? null);}res.writeHead(200,{'Content-Type':'image/png'});res.end();});
await new Promise(resolve=>collector.listen(39493,'127.0.0.1',resolve));
const dir=mkdtempSync(tmpdir()+'/vak-security-browser-');
const child=spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--remote-debugging-port=0','--user-data-dir='+dir,'about:blank'],{stdio:'ignore'});
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const evidence='/tmp/vak-security-hardening-browser';mkdirSync(evidence,{recursive:true});
try {
  for(let n=0;n<100&&!existsSync(dir+'/DevToolsActivePort');n++) await delay(100);
  const port=readFileSync(dir+'/DevToolsActivePort','utf8').split('\n')[0];
  const pages=await (await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  const ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);
  await new Promise(resolve=>ws.addEventListener('open',resolve,{once:true}));
  let seq=0;const pending=new Map();
  ws.addEventListener('message',event=>{const msg=JSON.parse(event.data);if(msg.id){const p=pending.get(msg.id);pending.delete(msg.id);msg.error?p.reject(msg.error):p.resolve(msg.result)}});
  const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}))});
  await call('Page.enable');
  const results=[];
  for(const [width,height] of [[1440,900],[390,844]]) for(const theme of ['light','dark']) for(const media of ['on','off']){
    await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
    await call('Page.navigate',{url:`http://127.0.0.1:39492/app/tests/security-boundaries.html?theme=${theme}&media=${media}`});
    let result;
    for(let n=0;n<100;n++){
      await delay(100);
      result=await call('Runtime.evaluate',{expression:'typeof window.runChecks === "function" ? window.runChecks() : null',returnByValue:true});
      if(result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      if(result.result?.value) break;
    }
    if(!result.result?.value) throw new Error('fixture did not start');
    await delay(300);
    if(leaks.length) throw new Error('DATA LEAK: '+JSON.stringify(leaks));
    const screenshot=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
    writeFileSync(`${evidence}/${width}-${theme}-${media}.png`,Buffer.from(screenshot.data,'base64'));
    results.push(result.result.value);console.log(JSON.stringify(result.result.value));
  }
  await call('Page.navigate',{url:'http://127.0.0.1:39492/app/tests/security-boundaries.html?native=1'});
  await delay(1000);
  await call('Runtime.evaluate',{expression:'document.querySelector("a.lnk").click()'});
  let opened=await call('Runtime.evaluate',{expression:'window.openedExternalLinks.length',returnByValue:true});
  if(opened.result.value!==0) throw new Error('synthetic click reached native opener');
  const position=await call('Runtime.evaluate',{expression:'JSON.stringify((()=>{const r=document.querySelector("a.lnk").getBoundingClientRect();return {x:r.x+5,y:r.y+5}})())',returnByValue:true});
  const point=JSON.parse(position.result.value);
  await call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...point});
  await call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...point});
  opened=await call('Runtime.evaluate',{expression:'window.openedExternalLinks',returnByValue:true});
  if(opened.result.value?.length!==1) throw new Error('trusted click did not reach native opener');
  if(leaks.length) throw new Error('native click fetched in webview');
  results.push({nativeTrustedClick:true,syntheticClickRefused:true});
  for(const href of ['javascript:window.UNSAFE_EXECUTED=true','file:///tmp/synthetic','data:text/html,synthetic','https://owner:secret@example.com/']) {
    const blockedPoint=await call('Runtime.evaluate',{expression:`JSON.stringify((()=>{const a=document.createElement('a');a.href=${JSON.stringify(href)};a.textContent='Rejected URL';a.addEventListener('click',e=>window.rejectedPrevented=e.defaultPrevented);document.body.prepend(a);a.scrollIntoView();const r=a.getBoundingClientRect();return {x:r.x+5,y:r.y+5}})())`,returnByValue:true});
    const point=JSON.parse(blockedPoint.result.value);
    await call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...point});
    await call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...point});
    const blocked=await call('Runtime.evaluate',{expression:'window.rejectedPrevented === true && !window.UNSAFE_EXECUTED && window.openedExternalLinks.length === 1',returnByValue:true});
    if(blocked.result.value!==true) throw new Error('rejected URL fell through: '+href);
  }
  results.push({rejectedUrlsInert:true});

  await call('Page.navigate',{url:'http://127.0.0.1:39492/app/tests/security-boundaries.html'});
  await delay(1000);
  const webPosition=await call('Runtime.evaluate',{expression:'JSON.stringify((()=>{const r=document.querySelector("a.lnk").getBoundingClientRect();return {x:r.x+5,y:r.y+5}})())',returnByValue:true});
  const webPoint=JSON.parse(webPosition.result.value);
  await call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...webPoint});
  await call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...webPoint});
  await delay(500);
  const newPages=await (await fetch('http://127.0.0.1:'+port+'/json/list')).json();
  if(!newPages.some(p=>p.url.includes('/collect?marker='))) throw new Error('web click did not open a separate tab');
  if(leaks.length!==1 || referrers[0]!==null) throw new Error('web navigation missing or sent a referrer: '+JSON.stringify({leaks,referrers}));
  results.push({webTrustedClick:true,separateTab:true,noReferrer:true});
  writeFileSync(evidence+'/results.json',JSON.stringify({results,automaticRequests:leaks.length-1,deliberateRequests:1,referrers},null,2));
  ws.close();
} finally {child.kill();collector.close();await delay(500);rmSync(dir,{recursive:true,force:true});}
