// Actual React/Fluent JSX rendered in Node. No browser or desktop automation.
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {createRequire} from 'node:module';
import {mkdir,readFile,rm,rmdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import {root,version,iconsPlugin} from './build-ui.mjs';

const directory=resolve(root,'.local/ui-check');
const output=resolve(directory,'app.cjs');
await mkdir(directory,{recursive:true});
await build({absWorkingDir:root,stdin:{contents:"export {App} from './ui/app.jsx'; export {About} from './ui/about.jsx';",resolveDir:root,sourcefile:'ui-check-entry.mjs',loader:'js'},outfile:output,bundle:true,platform:'node',format:'cjs',target:'node24',packages:'external',jsx:'automatic',loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"','__APP_VERSION__':JSON.stringify(version)},plugins:[iconsPlugin]});
const require=createRequire(new URL('../package.json',import.meta.url));
const React=require('react');
const {renderToStaticMarkup}=require('react-dom/server');
const {App,About}=require(output);
const labels=['仪表盘','代理','设置','运行日志','关于'];
const render=(page,status=null)=>renderToStaticMarkup(React.createElement(App,{initialPage:page,initialStatus:status}));
function sections(html) {
  // App pages are sibling sections; nested settings groups use divs/articles.
  return [...html.matchAll(/<section\b([^>]*)>([\s\S]*?)<\/section>/g)].map(([,attributes,body])=>({name:/aria-label="([^"]+)"/.exec(attributes)?.[1],visible:!/(?:^|\s)hidden(?:=|\s|$)/.test(attributes),body}));
}
function active(html,label) {
  const pages=sections(html);
  assert.deepEqual(pages.filter(page=>page.visible).map(page=>page.name),[label]);
  return pages.find(page=>page.name===label).body;
}
const sample={revision:7,enabled:true,guard_ready:true,proxy_url:'http://127.0.0.1:7890',proxy_reachable:true,proxy_latency_ms:0.25,probe_history:[{at:1000,latency_ms:0.5},{at:2000,latency_ms:null},{at:3000,latency_ms:0.25}],package_count:1,package_version:'26.1',codex_running:true,desktop_pid:42,tcp_connections:12,helper_memory_bytes:8388608,started_at:1000,updated_at:3662000,cli_detected:true,cli_ready:true,autostart:false,launch_desktop:false,backend_ready:true,logs:[{id:1,at:1000,level:'info',category:'检测',message:'检测完成',details:'耗时：23 ms\n<script>unsafe()</script>'}]};
let checks=0;
try {
  for(const [page,label] of [['dashboard','仪表盘'],['proxy','代理'],['settings','设置'],['logs','运行日志'],['about','关于']]) {
    const html=render(page,sample),body=active(html,label);
    assert.equal(sections(html).length,5);
    if(page!=='settings')assert.doesNotMatch(body,/登录时自动启动|启动后打开/);
    else {assert.match(body,/登录时自动启动/);assert.match(body,/启动后打开/);}
    assert.doesNotMatch(html,/保存启动设置|关闭设置窗口后，守护继续运行/);
    checks++;
  }
  const html=render('dashboard',sample);
  assert.equal((html.match(/class="window-controls"/g)||[]).length,1);
  for(const label of ['最小化','最大化','关闭到托盘'])assert.ok(html.includes(`aria-label="${label}"`));
  const brand=/<div\b[^>]*class="brand"[^>]*>([\s\S]*?)<\/div>/.exec(html)?.[1];
  assert.ok(brand);assert.equal((brand.match(/CodexProxyTray/g)||[]).length,1);
  assert.doesNotMatch(html,/<(?:div|header)\b[^>]*class="titlebar"/);
  const config=JSON.parse(await readFile(resolve(root,'src-tauri/tauri.conf.json'),'utf8'));
  assert.equal(config.app.windows.find(window=>window.label==='main').decorations,false);
  checks++;

  const storage=Object.getOwnPropertyDescriptor(globalThis,'localStorage');
  try {
    Object.defineProperty(globalThis,'localStorage',{configurable:true,value:{getItem:()=> '1'}});
    const collapsed=render('dashboard',sample);
    assert.match(collapsed,/app-shell sidebar-collapsed/);
    assert.ok(collapsed.includes('aria-label="展开侧边栏"'));
    for(const label of labels){assert.ok(collapsed.includes(`aria-label="${label}"`));assert.ok(collapsed.includes(`title="${label}"`));}
  } finally {if(storage)Object.defineProperty(globalThis,'localStorage',storage);else delete globalThis.localStorage;}
  assert.ok(html.includes('aria-label="收起侧边栏"'));
  checks++;

  const dashboard=active(html,'仪表盘');
  for(const label of ['本地代理连接耗时','ChatGPT TCP 连接','本次守护时长'])assert.ok(dashboard.includes(label));
  assert.match(dashboard,/0\.25/);assert.match(dashboard,/01:01:01/);
  assert.match(dashboard,/本地代理 TCP 连接耗时趋势/);
  assert.doesNotMatch(dashboard,/系统代理|虚拟网卡|上传速度|下载速度|流量统计/);
  const unknown=active(render('dashboard'),'仪表盘');
  assert.match(unknown,/等待本地代理采样/);
  assert.doesNotMatch(unknown,/本地代理 TCP 连接耗时趋势|0\.00|NaN|Infinity/);
  const values=[...unknown.matchAll(/class="metric-value"[^>]*>([\s\S]*?)<\/strong>/g)].map(match=>match[1]);
  assert.ok(values.length>=3);assert.ok(values.every(value=>value==='—'));
  checks++;

  const logs=active(render('logs',sample),'运行日志');
  assert.match(logs,/检测完成/);assert.match(logs,/log-category[^>]*>检测/);assert.match(logs,/log-details/);assert.match(logs,/耗时：23 ms/);
  assert.ok(logs.includes('&lt;script&gt;unsafe()&lt;/script&gt;'));assert.doesNotMatch(logs,/<script>/);
  checks++;

  const noop=()=>{};
  const about=(status,pending=new Set())=>renderToStaticMarkup(React.createElement(About,{status,version,busy:name=>pending.has(name),writable:true,onPreference:noop,onCheck:noop,onSource:noop,onRelease:noop,onLogs:noop}));
  const control=(markup,tag,id)=>{
    const escaped=id.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
    const result=new RegExp(`<${tag}\\b[^>]*\\bid="${escaped}"[^>]*>`).exec(markup)?.[0];
    assert.ok(result,`Missing ${id}`);return result;
  };
  const disabled=markup=>/\bdisabled(?:=|\s|>)/.test(markup);
  const updateState={checking:true,checked_at:Date.UTC(2026,10,11,12,34,56),status:'current',latest:version,notes:'',error:null};
  const background=about({network_enabled:true,auto_check_updates:false,update_interval_hours:3,update:updateState});
  assert.match(background,/后台检查中/);assert.match(control(background,'button','check-updates'),/aria-busy="false"/);
  assert.equal(disabled(control(background,'button','check-updates')),false);
  assert.equal(disabled(control(background,'select','update-interval')),false);
  assert.match(background,/<option\b(?=[^>]*\bvalue="3")(?=[^>]*\bselected(?:=|\s|>))[^>]*>/);
  assert.doesNotMatch(background,/<img\b|about-brand|北京时间/);
  assert.match(background,/20:34:56/);
  assert.match(background,/不记录或上报对话、提示词、用户 CLI 参数/);
  assert.match(background,/启动参数仅原样转发给已安装的 CLI/);
  assert.match(background,/目标应用代理和本机 TCP 探测继续运行/);
  assert.match(background,/MIT 开源许可/);
  checks++;

  const offline=about({network_enabled:false,auto_check_updates:true,update_interval_hours:12,update:{status:'unchecked'}});
  assert.equal(disabled(control(offline,'button','check-updates')),true);
  assert.equal(disabled(control(offline,'input','auto-check-updates')),true);
  assert.match(control(offline,'input','auto-check-updates'),/\bchecked(?:=|\s|>)/);
  assert.match(offline,/<option\b(?=[^>]*\bvalue="12")(?=[^>]*\bselected(?:=|\s|>))[^>]*>/);
  assert.equal(disabled(control(offline,'button','open-source')),false);
  checks++;

  const manual=about({network_enabled:true,update:updateState},new Set(['check-updates']));
  assert.match(control(manual,'button','check-updates'),/aria-busy="true"/);
  for(const id of ['network-enabled','auto-check-updates'])assert.match(control(manual,'input',id),/aria-busy="false"/);
  assert.match(control(manual,'select','update-interval'),/aria-busy="false"/);
  const interval=about({network_enabled:true,auto_check_updates:false},new Set(['update-interval']));
  assert.match(control(interval,'select','update-interval'),/aria-busy="true"/);
  assert.match(control(interval,'button','check-updates'),/aria-busy="false"/);
  assert.equal(disabled(control(interval,'button','check-updates')),false);
  const firstTime=about({network_enabled:true});
  assert.match(firstTime,/上次检查：尚未检查/);assert.match(firstTime,/<option\b(?=[^>]*\bvalue="6")(?=[^>]*\bselected(?:=|\s|>))[^>]*>/);
  checks++;
  console.log(`React SSR UI checks passed: ${checks}. No browser or desktop automation was used.`);
} finally {
  // Only this script's own intermediate is removed; no repository/source cleanup.
  await rm(output,{force:true});
  await rmdir(directory).catch(error=>{if(!['ENOTEMPTY','ENOENT'].includes(error.code))throw error;});
}
