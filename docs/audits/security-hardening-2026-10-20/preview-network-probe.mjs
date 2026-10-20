import http from 'node:http';import dgram from 'node:dgram';import {spawn} from 'node:child_process';import {mkdtempSync,readFileSync,existsSync,rmSync} from 'node:fs';import {tmpdir} from 'node:os';
const delay=ms=>new Promise(r=>setTimeout(r,ms));let packets=0;const udp=dgram.createSocket('udp4');udp.on('message',()=>packets++);await new Promise(r=>udp.bind(39496,'127.0.0.1',r));
const mode=process.argv[2]??'baseline';
const child=`<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; base-uri 'none'; form-action 'none'; script-src 'unsafe-inline' blob:; style-src 'unsafe-inline' data:; img-src data: blob:; font-src data: blob:; media-src data: blob:; connect-src 'none'"><script>const c=new RTCPeerConnection({iceServers:[{urls:'stun:127.0.0.1:39496'}]});c.createDataChannel('synthetic');c.createOffer().then(x=>c.setLocalDescription(x)).catch(e=>parent.postMessage(String(e),'*'));c.onicecandidateerror=e=>parent.postMessage('ice:'+e.errorCode,'*');parent.postMessage('started','*');</script>`;
const {sandboxedSrcdoc,previewSandbox}=await import(process.cwd()+'/crates/vak-client-ui/src/safeUrl.ts');
const attack=child.slice(child.indexOf('<script>'));
const shown=mode==='static'?sandboxedSrcdoc(attack):child;
let policy;let slot='';
if(mode==='origin'){
 const {readFileSync}=await import('node:fs');const rust=readFileSync('crates/vak-server/src/preview.rs','utf8');
 policy=rust.match(/const POLICY: &str = "([\s\S]*?)";/)[1].replaceAll('\\\n','');
 slot=JSON.parse(rust.match(/const THEME_SLOT: &str = (".*");/)[1]);
}
const attr=shown.replaceAll('&','&amp;').replaceAll('"','&quot;');
const server=http.createServer((q,r)=>{r.setHeader('Content-Type','text/html');if(q.url==='/origin'){r.setHeader('Content-Security-Policy',policy);r.end(slot+attack);return;}
 const frame=mode==='origin'?'<iframe sandbox="'+previewSandbox('origin')+'" src="/origin"></iframe>':'<iframe sandbox="'+(mode==='static'?previewSandbox('static'):'allow-scripts allow-forms')+'" srcdoc="'+attr+'"></iframe>';
 r.end('<script>window.probeMessages=[];onmessage=e=>probeMessages.push(e.data);setTimeout(()=>document.querySelector("iframe").contentWindow.postMessage({type:"vak-theme",css:"body{color:rgb(1, 2, 3)}"},"*"),1000)</script>'+frame);
});await new Promise(r=>server.listen(39495,'127.0.0.1',r));
const dir=mkdtempSync(tmpdir()+'/vak-preview-probe-');const chrome=spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',['--headless=new','--disable-gpu','--no-first-run','--disable-background-networking','--remote-debugging-port=0','--user-data-dir='+dir,'http://127.0.0.1:39495'],{stdio:'ignore'});
try{for(let i=0;i<100&&!existsSync(dir+'/DevToolsActivePort');i++)await delay(100);const port=readFileSync(dir+'/DevToolsActivePort','utf8').split('\n')[0];const pages=await(await fetch('http://127.0.0.1:'+port+'/json/list')).json();const ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));await delay(5000);ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id===1){console.log(JSON.stringify({mode,packets,...m.result}));if(mode!=='baseline'&&(packets||(mode==='origin'?m.result.result?.value!==JSON.stringify({messages:[],theme:'body{color:rgb(1, 2, 3)}'}):m.result.result?.value!=='[]')))process.exitCode=1;ws.close();}});ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:mode==='origin'?'JSON.stringify({messages:probeMessages,theme:document.querySelector("iframe").contentDocument.querySelector("style[data-vak-theme]").textContent})':'JSON.stringify(probeMessages)',returnByValue:true}}));await delay(500);}finally{chrome.kill();udp.close();server.close();await delay(500);rmSync(dir,{recursive:true,force:true});}
