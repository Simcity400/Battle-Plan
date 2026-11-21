import { writeFile, readFile } from 'node:fs/promises';
import path from 'node:path';
if (process.argv.length < 4) throw new Error('Usage: node tools/desktop_cdp.mjs <session-directory> inspect|screenshot|click|click-selector|key|eval|file [arguments]');
const session = JSON.parse(await readFile(path.join(process.argv[2], 'session.json'), 'utf8'));
if (!Number.isInteger(session.port) || session.port < 1024 || session.port > 65535) throw new Error('Invalid debugging port');
const targets=await (await fetch(`http://127.0.0.1:${session.port}/json/list`, {signal: AbortSignal.timeout(10000)})).json();
const target=targets.find(t=>t.type==='page' && t.title==='Battle Plan');
if(!target) throw new Error('Desktop WebView2 target unavailable');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{
 const timer=setTimeout(()=>{socket.close();reject(new Error('Desktop connection timed out'));},10000);
 socket.onopen=()=>{clearTimeout(timer);resolve();};
 socket.onerror=error=>{clearTimeout(timer);reject(error);};
});
let sequence=0;
const pending=new Map();
socket.onmessage=({data})=>{const reply=JSON.parse(data);const item=pending.get(reply.id);if(item){pending.delete(reply.id);reply.error?item.reject(reply.error):item.resolve(reply.result);}};
socket.onclose=()=>{for(const item of pending.values())item.reject(new Error('Desktop connection closed'));pending.clear();};
const send=(method,params={})=>new Promise((resolve,reject)=>{
 const id=++sequence;
 const timer=setTimeout(()=>{pending.delete(id);reject(new Error(`Timed out: ${method}`));},30000);
 pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});
 socket.send(JSON.stringify({id,method,params}));
});
try {
 const native=await send('Runtime.evaluate',{expression:'!!window.__TAURI_INTERNALS__',returnByValue:true});
 if(native.result.value!==true) throw new Error('Refusing to control a page outside the desktop host');
 const action=process.argv[3];
 if(action==='screenshot'){
   const result=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
   await writeFile(process.argv[4],Buffer.from(result.data,'base64'));
   console.log(JSON.stringify({screenshot:process.argv[4],url:target.url}));
 } else if(action==='inspect') {
   const result=await send('Runtime.evaluate',{expression:`({native:!!window.__TAURI_INTERNALS__,text:document.body.innerText,elements:[...document.querySelectorAll('button,input,select,summary,[role=tab]')].filter(el=>el.checkVisibility()).map(el=>({tag:el.tagName,name:el.getAttribute('aria-label')||el.innerText||el.closest('label')?.innerText,disabled:el.disabled,checked:el.checked,value:el.value,rect:el.getBoundingClientRect().toJSON()}))})`,returnByValue:true});
   console.log(JSON.stringify(result.result.value));
 } else if(action==='key'){
   const key=process.argv[4];const code={Enter:13,Escape:27,ArrowDown:40,ArrowUp:38,Home:36,End:35,Tab:9}[key]||0;
   await send('Input.dispatchKeyEvent',{type:'keyDown',key,windowsVirtualKeyCode:code,nativeVirtualKeyCode:code});
   await send('Input.dispatchKeyEvent',{type:'keyUp',key,windowsVirtualKeyCode:code,nativeVirtualKeyCode:code});
 } else if(action==='click'||action==='click-selector'){
   let x=Number(process.argv[4]),y=Number(process.argv[5]);
   if(action==='click-selector'){
     const r=await send('Runtime.evaluate',{expression:`(()=>{const el=document.querySelector(${JSON.stringify(process.argv[4])});el.scrollIntoView({block:'nearest'});const r=el.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`,returnByValue:true});
     if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));
     ({x,y}=r.result.value);
   }
   await send('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});
   await send('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
   console.log('clicked');
 } else if(action==='eval'||action==='file') {
   const expression=action==='file'?await readFile(process.argv[4],'utf8'):process.argv[4];
   const result=await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
   if(result.exceptionDetails)throw new Error(JSON.stringify(result.exceptionDetails));
   console.log(JSON.stringify(result.result.value));
 } else throw new Error(`Unknown action: ${action}`);
} finally {socket.close();}
