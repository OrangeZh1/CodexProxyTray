// QA-only IPC transport. Serves the actual ui/ files and never contacts native services.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
const port = Number(process.env.QA_PORT || 4174);
if (!Number.isInteger(port) || port < 1 || port > 65535 || port === 4173) throw Error('QA_PORT must be a separate port; 4173 is reserved for the real preview.');
const commands = ['get_status', 'refresh_status', 'set_enabled', 'save_proxy', 'save_system', 'launch_codex', 'launch_cli', 'check_updates', 'open_release', 'open_source', 'open_logs'];
const defaults = new Map(), queued = new Map(), history = [];
let online = true, epoch = 0, revision = 0, requestId = 0;
const freshState = () => ({ revision: ++revision, enabled: true, guard_ready: true, proxy_url: 'http://127.0.0.1:7890', proxy_reachable: true, package_count: 1, package_version: 'QA', codex_running: false, restart_required: false, cli_detected: true, cli_ready: true, autostart: false, launch_target:'none',cli_workdir:'',network_enabled:true,auto_check_updates:false,update_interval_hours:6,update:{checking:false,checked_at:null,status:'unchecked',latest:null,notes:'',error:null},backend_ready: true, read_only: false, logs: [{ at: Date.now(), level: 'info',category:'QA',details:'仅测试，不修改真实应用或配置。',message: 'QA 模拟 IPC，无真实代理操作。' }] });
let state = freshState();
const view = () => ({ online, state, defaults: Object.fromEntries(defaults), queued: Object.fromEntries(queued), history });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const fault = (message, name = 'Error') => Object.assign(Error(message), { name });
const json = (res, status, data) => { if (!res.destroyed) res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' }).end(JSON.stringify(data)); };

async function body(req) {
  const chunks = []; let size = 0;
  for await (const chunk of req) { size += chunk.length; if (size > 65536) throw Error('QA request too large'); chunks.push(chunk); }
  return JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}');
}

function control(input) {
  if (input.reset) { epoch++; online = true; defaults.clear(); queued.clear(); history.length = 0; state = freshState(); }
  if (typeof input.online === 'boolean' && input.online !== online) { online = input.online; epoch++; }
  if (input.clearRules) { defaults.clear(); queued.clear(); }
  if (input.rule) {
    const { command, delayMs = 0, outcome = 'success', once = true } = input.rule;
    if (!commands.includes(command) || !Number.isInteger(delayMs) || delayMs < 0 || delayMs > 30000 || !['success', 'error', 'abort'].includes(outcome) || typeof once !== 'boolean') throw Error('Invalid QA rule');
    const rule = { delayMs, outcome };
    if (once) { const list = queued.get(command) || []; if (list.length >= 20) throw Error('QA rule queue is full'); list.push(rule); queued.set(command, list); }
    else defaults.set(command, rule);
  }
}

function resultFor(command, args) {
  if (command === 'check_updates') {state.update={checking:false,checked_at:Date.now(),status:'available',latest:'0.2.0',notes:'QA 模拟发布说明。',error:null};return { status: 'available', current: '0.1.0', latest: '0.2.0', url: 'https://github.com/OrangeZh1/CodexProxyTray/releases', notes: 'QA 模拟发布说明。' };}
  if (command === 'get_status' || command === 'refresh_status') return structuredClone(state);
  if (command === 'set_enabled') { state.enabled = !!args.enabled; state.guard_ready = state.enabled; }
  if (command === 'save_proxy') { state.proxy_url = String(args.proxyUrl); state.restart_required = state.codex_running; }
  if (command === 'save_system') {for(const [input,field] of Object.entries({autostart:'autostart',launchTarget:'launch_target',cliWorkdir:'cli_workdir',networkEnabled:'network_enabled',autoCheckUpdates:'auto_check_updates',updateIntervalHours:'update_interval_hours'})){if(args[input]!==undefined&&args[input]!==null)state[field]=args[input];}}
  if (command === 'launch_codex') state.codex_running = true;
  state.revision = ++revision;
  state.logs.push({ at: Date.now(), level: 'info', message: `QA 模拟：${command}` });
  if (state.logs.length > 100) state.logs.shift();
  return ['open_release','open_source','open_logs'].includes(command) ? null : structuredClone(state);
}

async function invoke(command, args) {
  if (!commands.includes(command)) throw Error('Unknown QA command');
  const list = queued.get(command);
  const rule = list?.shift() || defaults.get(command) || { delayMs: 0, outcome: 'success' };
  const generation = epoch;
  const row = { id: ++requestId, command, args, delayMs: rule.delayMs, outcome: rule.outcome, startedAt: Date.now(), pending: true };
  history.push(row); if (history.length > 100) history.shift();
  try {
    if (!online) throw fault('QA 模拟断线');
    // Capture replies at dispatch. Delayed reads can arrive after a newer save reply.
    const value = rule.outcome === 'success' ? resultFor(command, args) : null;
    await sleep(rule.delayMs);
    if (generation !== epoch || !online) throw fault('QA 连接已重置');
    if (rule.outcome === 'error') throw fault(`QA 模拟失败：${command}`);
    if (rule.outcome === 'abort') throw fault(`QA 模拟取消：${command}`, 'AbortError');
    row.responseRevision = value?.revision ?? null;
    return value;
  } catch (error) { row.error = { name: error.name, message: error.message }; throw error; }
  finally { row.pending = false; row.finishedAt = Date.now(); }
}

const bridge = `window.__TAURI__={core:{invoke:async(command,args={})=>{const response=await fetch('/__qa/invoke',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({command,args})});const reply=await response.json();if(!reply.ok){if(reply.error.name==='AbortError')throw new DOMException(reply.error.message,'AbortError');const error=new Error(reply.error.message);error.name=reply.error.name;throw error;}return reply.value;}}};`;
const controlHtml = `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>CodexProxyTray · QA IPC 控制</title><style>body{font:14px/1.7 system-ui;max-width:800px;margin:30px auto;padding:0 20px}h1{font-size:20px;color:#a32323}label{display:inline-block;margin:8px 12px 8px 0}input,select,button{font:inherit;padding:6px}pre{background:#f2f3f1;padding:15px;white-space:pre-wrap;max-height:500px;overflow:auto}</style><h1>QA 模拟 IPC 控制</h1><p>仅修改本 fixture 内存，不操作真实助手、代理、注册表或 GitHub。</p><form id="plan"><label>命令 <select name="command">${commands.map(command => `<option>${command}</option>`).join('')}</select></label><label>延迟毫秒 <input name="delayMs" type="number" min="0" max="30000" value="2500"></label><label>结果 <select name="outcome"><option value="success">成功</option><option value="error">失败</option><option value="abort">取消</option></select></label><label><input name="once" type="checkbox" checked>仅下一次</label><button>设置规则</button></form><p><button data-control='{"online":false}'>模拟断线</button> <button data-control='{"online":true}'>恢复连接</button> <button data-control='{"clearRules":true}'>清除规则</button> <button data-control='{"reset":true}'>重置 fixture</button> <button id="refresh">刷新记录</button> <a href="/" target="_blank">打开 QA 页面</a></p><div id="feedback" role="status"></div><pre id="state"></pre><script src="/__qa/control.js"></script></html>`;
const controls = `const feedback=document.getElementById('feedback');async function refresh(){document.getElementById('state').textContent=JSON.stringify(await(await fetch('/__qa/state')).json(),null,2)}async function set(input){const response=await fetch('/__qa/control',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(input)});const data=await response.json();feedback.textContent=response.ok?'规则已设置':data.error;await refresh()}document.getElementById('plan').onsubmit=async event=>{event.preventDefault();const form=new FormData(event.currentTarget);await set({rule:{command:form.get('command'),delayMs:Number(form.get('delayMs')),outcome:form.get('outcome'),once:form.has('once')}})};document.querySelectorAll('[data-control]').forEach(button=>button.onclick=()=>set(JSON.parse(button.dataset.control)));document.getElementById('refresh').onclick=refresh;refresh();`;
const types = { '.html': 'text/html; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' };
const server = createServer(async (req, res) => {
  if (![ `127.0.0.1:${port}`, `localhost:${port}` ].includes(req.headers.host)) return res.writeHead(403).end();
  if (req.headers.origin && ![`http://127.0.0.1:${port}`, `http://localhost:${port}`].includes(req.headers.origin)) return res.writeHead(403).end();
  const url = new URL(req.url, `http://127.0.0.1:${port}`);
  try {
    if (req.method === 'POST' && url.pathname === '/__qa/invoke') {
      try { const input = await body(req); json(res, 200, { ok: true, value: await invoke(input.command, input.args || {}) }); }
      catch (error) { json(res, 200, { ok: false, error: { name: error.name, message: error.message } }); }
      return;
    }
    if (req.method === 'POST' && url.pathname === '/__qa/control') { control(await body(req)); return json(res, 200, view()); }
    if (req.method !== 'GET') return res.writeHead(405).end();
    if (url.pathname === '/__qa/state') return json(res, 200, view());
    if (url.pathname === '/__qa/bridge.js' || url.pathname === '/__qa/control.js') return res.writeHead(200, { 'Content-Type': 'text/javascript; charset=utf-8', 'Cache-Control': 'no-store' }).end(url.pathname.endsWith('bridge.js') ? bridge : controls);
    if (url.pathname === '/__qa' || url.pathname === '/__qa/') return res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' }).end(controlHtml);
    const path = resolve(root, '.' + decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname));
    if (!path.startsWith(root + sep)) return res.writeHead(404).end();
    let data = await readFile(path);
    if (extname(path) === '.html') data = data.toString().replace('<script type="module"', '<script src="/__qa/bridge.js"></script><script type="module"').replace('<body>', '<body><div style="position:fixed;z-index:99999;right:12px;top:6px;background:#9f2929;color:white;padding:5px 12px;border-radius:6px;font:12px system-ui;pointer-events:none">QA 模拟 IPC · 不影响真实代理</div>');
    res.writeHead(200, { 'Content-Type': types[extname(path)] || 'application/octet-stream', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' }).end(data);
  } catch (error) { json(res, url.pathname.startsWith('/__qa/') ? 400 : 404, { error: error.message }); }
});
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { epoch++; server.closeAllConnections(); server.close(() => process.exit(0)); });
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({ pid: process.pid, url: `http://127.0.0.1:${port}`, controls: `http://127.0.0.1:${port}/__qa`, purpose: 'QA simulated IPC only' })));
