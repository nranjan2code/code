import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, existsSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import http from 'node:http';
const leaks=[];
const collector=http.createServer((req,res)=>{leaks.push(req.url);res.writeHead(200,{'Content-Type':'image/png'});res.end();});
await new Promise(resolve=>collector.listen(39493,'127.0.0.1',resolve));
const dir=mkdtempSync(tmpdir()+'/vak-security-browser-');
const child=spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--remote-debugging-port=0','--user-data-dir='+dir,'about:blank'],{stdio:'ignore'});
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const evidence='/tmp/vak-security-hardening-admin-browser';mkdirSync(evidence,{recursive:true});
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
  for(const [width,height] of [[1440,900],[390,844]]) for(const theme of ['light','dark']) for(const media of ['complete','incomplete']){
    await call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
    await call('Page.navigate',{url:`http://127.0.0.1:39494/admin/tests/data-integrity.html?theme=${theme}&state=${media}`});
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
  writeFileSync(evidence+'/results.json',JSON.stringify({results,leaks},null,2));
  ws.close();
} finally {child.kill();collector.close();await delay(500);rmSync(dir,{recursive:true,force:true});}
