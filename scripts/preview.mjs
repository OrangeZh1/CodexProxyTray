import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { watch } from 'node:fs';
import { dirname, resolve, sep, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { liveStatus, checkUpdates } from './live-status.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
const port = Number(process.env.PORT || 4173);
const clients = new Set();
const session = Date.now().toString(36);
const bridge = `
const nativeBackend=!!window.__TAURI__?.core;if(!nativeBackend)window.__TAURI__={core:{async invoke(command,args={}){const response=await fetch('/__dev/api/'+command,{cache:'no-store'});const data=await response.json();if(!response.ok)throw data.error||'读取失败';return data}}};
const events=new EventSource('/__dev/events');events.onmessage=()=>location.reload();
events.addEventListener('session',event=>{if(event.data!==${JSON.stringify(session)})location.reload()});
`;
const types = { '.html':'text/html; charset=utf-8', '.css':'text/css; charset=utf-8', '.mjs':'text/javascript; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.svg':'image/svg+xml' };
const server=createServer(async(req,res)=>{
  const url=new URL(req.url,'http://127.0.0.1');
  if(req.headers.host!==`127.0.0.1:${port}` && req.headers.host!==`localhost:${port}`){res.writeHead(403).end();return;}
  if(req.method!=='GET'){res.writeHead(405).end();return;}
  if(url.pathname.startsWith('/__dev/api/')){try{const command=url.pathname.split('/').pop();let data;if(command==='get_status'||command==='refresh_status')data=await liveStatus(command==='refresh_status');else if(command==='check_updates')data=await checkUpdates();else{res.writeHead(409,{'Content-Type':'application/json; charset=utf-8'}).end(JSON.stringify({error:'此预览读取真实状态，修改设置请使用 Tauri 桌面窗口。'}));return;}res.writeHead(200,{'Content-Type':'application/json; charset=utf-8','Cache-Control':'no-store'}).end(JSON.stringify(data));}catch(error){res.writeHead(503,{'Content-Type':'application/json; charset=utf-8'}).end(JSON.stringify({error:String(error.message||error)}));}return;}
  if(url.pathname==='/__dev/events'){res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-cache','Connection':'keep-alive'});res.write(`event: session\ndata: ${session}\n\n`);clients.add(res);req.on('close',()=>clients.delete(res));return;}
  if(url.pathname==='/__dev/bridge.js'){res.writeHead(200,{'Content-Type':'text/javascript; charset=utf-8','Cache-Control':'no-store'}).end(bridge);return;}
  let path;
  try{path=resolve(root,'.'+decodeURIComponent(url.pathname==='/'?'/index.html':url.pathname));if(!path.startsWith(root+sep))throw Error();const info=await stat(path);if(!info.isFile())throw Error();let data=await readFile(path);if(extname(path)==='.html')data=data.toString().replace('<script type="module"','<script src="/__dev/bridge.js"></script><script type="module"');res.writeHead(200,{'Content-Type':types[extname(path)]||'application/octet-stream','Cache-Control':'no-store','X-Content-Type-Options':'nosniff'}).end(data);}catch{res.writeHead(404).end('Not found');}
});
let timer;
const watcher=watch(root,{recursive:true},()=>{clearTimeout(timer);timer=setTimeout(()=>{for(const client of clients)client.write('data: reload\n\n')},150)});
const heartbeat=setInterval(()=>{for(const client of clients)client.write(': heartbeat\n\n')},20000);
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{clearInterval(heartbeat);watcher.close();for(const client of clients)client.end();server.close(()=>process.exit(0));});
server.listen(port,'127.0.0.1',()=>console.log(`CodexProxyTray UI preview: http://127.0.0.1:${port} (PID ${process.pid}, live read-only status)`));
