// Actual React/Fluent JSX rendered in Node. No browser or desktop automation.
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {createRequire} from 'node:module';
import {mkdir,readFile,rm,rmdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import {runInNewContext} from 'node:vm';
import {root,version,iconsPlugin} from './build-ui.mjs';

const directory=resolve(root,'.local/ui-check');
const output=resolve(directory,'app.cjs');
await mkdir(directory,{recursive:true});
await build({absWorkingDir:root,stdin:{contents:"export {App,GuardButton,ConnectivityCard,TrafficCard,CollectorSettings,trafficAmount} from './ui/app.jsx'; export {About,LicenseText} from './ui/about.jsx'; export {StatusLabel} from './ui/status-label.jsx'; export {readThemeMode,resolveTheme} from './ui/themes.mjs';",resolveDir:root,sourcefile:'ui-check-entry.mjs',loader:'js'},outfile:output,bundle:true,platform:'node',format:'cjs',target:'node24',packages:'external',jsx:'automatic',loader:{'.css':'empty'},define:{'process.env.NODE_ENV':'"production"','__APP_VERSION__':JSON.stringify(version)},plugins:[iconsPlugin]});
const require=createRequire(new URL('../package.json',import.meta.url));
const React=require('react');
const {renderToStaticMarkup:renderReactMarkup}=require('react-dom/server');
const {SSRProvider}=require('@fluentui/react-components');
const renderToStaticMarkup=node=>renderReactMarkup(React.createElement(SSRProvider,null,node));
const {App,About,LicenseText,GuardButton,ConnectivityCard,TrafficCard,CollectorSettings,StatusLabel,trafficAmount,readThemeMode,resolveTheme}=require(output);
const labels=['仪表盘','代理','运行日志','设置'];
const render=(page,status=null,initialSettingsTab='general')=>renderToStaticMarkup(React.createElement(App,{initialPage:page,initialStatus:status,initialSettingsTab}));
function sections(html) {
  // App pages are sibling sections; nested settings groups use divs/articles.
  return [...html.matchAll(/<section\b([^>]*)>([\s\S]*?)<\/section>/g)].map(([,attributes,body])=>({name:/aria-label="([^"]+)"/.exec(attributes)?.[1],visible:!/(?:^|\s)hidden(?:=|\s|$)/.test(attributes),body}));
}
function active(html,label) {
  const pages=sections(html);
  assert.deepEqual(pages.filter(page=>page.visible).map(page=>page.name),[label]);
  return pages.find(page=>page.name===label).body;
}
function settingsPanel(html,name){
  const body=active(html,'设置');
  const panels=[...body.matchAll(/<article\b([^>]*)>([\s\S]*?)<\/article>/g)].map(([,attributes,content])=>({id:/\bid="([^"]+)"/.exec(attributes)?.[1],visible:!/(?:^|\s)hidden(?:=|\s|$)/.test(attributes),body:content}));
  assert.equal(panels.length,2);
  assert.deepEqual(panels.filter(panel=>panel.visible).map(panel=>panel.id),[`settings-panel-${name}`]);
  return panels.find(panel=>panel.id===`settings-panel-${name}`).body;
}
const sample={revision:7,enabled:true,guard_ready:true,proxy_url:'http://127.0.0.1:7890',proxy_reachable:true,proxy_latency_ms:0.25,probe_history:[{at:1000,latency_ms:0.5},{at:2000,latency_ms:null},{at:3000,latency_ms:0.25}],package_count:1,package_version:'26.1',codex_running:true,desktop_pid:42,tcp_connections:12,helper_memory_bytes:8388608,started_at:1000,updated_at:3662000,cli_detected:true,cli_ready:true,autostart:false,launch_desktop:false,backend_ready:true,logs:[{id:1,at:1000,level:'info',category:'检测',message:'检测完成',details:'耗时：23 ms\n<script>unsafe()</script>'}]};
let checks=0;
try {
  for(const [page,label] of [['dashboard','仪表盘'],['proxy','代理'],['logs','运行日志'],['settings','设置']]) {
    const html=render(page,sample),body=page==='settings'?settingsPanel(html,'general'):active(html,label);
    assert.equal(sections(html).length,4);
    if(page!=='settings')assert.doesNotMatch(body,/登录时自动启动|启动后打开/);
    else {assert.match(body,/登录时自动启动/);assert.match(body,/启动后打开/);}
    assert.doesNotMatch(html,/保存启动设置|关闭设置窗口后，守护继续运行/);
    checks++;
  }
  const settingsAbout=render('settings',sample,'about');
  const aboutPanel=settingsPanel(settingsAbout,'about');
  assert.match(aboutPanel,/版本 /);assert.match(aboutPanel,/本机数据与隐私/);assert.match(aboutPanel,/检查更新/);
  assert.doesNotMatch(aboutPanel,/登录时自动启动|启动后打开|aria-label="界面主题"/);
  assert.match(active(settingsAbout,'设置'),/aria-label="设置分类"/);
  assert.doesNotMatch(settingsAbout,/\bdata-page="about"/);
  assert.equal(sections(settingsAbout).length,4);
  settingsPanel(render('settings',sample,'invalid'),'general');
  settingsPanel(render('about',sample),'about');
  assert.match(active(render('not-a-page',sample),'仪表盘'),/本地代理连接耗时/);
  checks++;
  const html=render('dashboard',sample);
  const navigation=/<nav\b[^>]*>([\s\S]*?)<\/nav>/.exec(html)?.[1];
  assert.ok(navigation);assert.deepEqual([...navigation.matchAll(/\bdata-page="([^"]+)"/g)].map(match=>match[1]),['dashboard','proxy','logs','settings']);
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
  for(const label of ['本地代理连接耗时','ChatGPT TCP 连接','助手运行时长'])assert.ok(dashboard.includes(label));
  assert.match(dashboard,/0\.25/);assert.match(dashboard,/01:01:01/);
  assert.match(dashboard,/本地代理 TCP 连接耗时趋势/);
  assert.doesNotMatch(dashboard,/系统代理|虚拟网卡|出站模式/);
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
  const buttonLabel=(markup,id)=>new RegExp(`<button\\b(?=[^>]*\\bid="${id}")[^>]*>([\\s\\S]*?)<\\/button>`).exec(markup)?.[1]||'';
  const updateState={checking:true,checked_at:Date.UTC(2026,10,11,12,34,56),status:'current',latest:version,notes:'',error:null};
  const background=about({network_enabled:true,auto_check_updates:false,update_interval_hours:3,update:updateState});
  assert.match(background,/后台检查中/);assert.match(control(background,'button','check-updates'),/aria-busy="false"/);
  assert.equal(disabled(control(background,'button','check-updates')),false);
  assert.equal(disabled(control(background,'button','update-interval')),false);
  assert.match(buttonLabel(background,'update-interval'),/3 小时/);
  assert.doesNotMatch(background,/<img\b|about-brand|北京时间/);
  assert.match(background,/20:34:56/);
  assert.match(background,/不记录或上报对话、提示词、用户 CLI 参数/);
  assert.match(background,/启动参数仅原样转发给已安装的 CLI/);
  assert.match(background,/目标应用代理和本机 TCP 探测继续运行/);
  assert.match(background,/MIT 许可/);assert.match(background,/aria-haspopup="dialog"/);assert.doesNotMatch(background,/<details\b/);
  const license=renderToStaticMarkup(React.createElement(LicenseText));
  assert.match(license,/MIT License/);assert.match(license,/Copyright \(c\) 2026 OrangeZh1/);assert.match(license,/THE SOFTWARE IS PROVIDED/);
  checks++;

  const offline=about({network_enabled:false,auto_check_updates:true,update_interval_hours:12,update:{status:'unchecked'}});
  assert.equal(disabled(control(offline,'button','check-updates')),true);
  assert.equal(disabled(control(offline,'input','auto-check-updates')),true);
  assert.match(control(offline,'input','auto-check-updates'),/\bchecked(?:=|\s|>)/);
  assert.match(buttonLabel(offline,'update-interval'),/12 小时/);
  assert.equal(disabled(control(offline,'button','open-source')),false);
  checks++;

  const manual=about({network_enabled:true,update:updateState},new Set(['check-updates']));
  assert.match(control(manual,'button','check-updates'),/aria-busy="true"/);
  for(const id of ['network-enabled','auto-check-updates'])assert.match(control(manual,'input',id),/aria-busy="false"/);
  assert.match(control(manual,'button','update-interval'),/aria-busy="false"/);
  const interval=about({network_enabled:true,auto_check_updates:false},new Set(['update-interval']));
  assert.match(control(interval,'button','update-interval'),/aria-busy="true"/);
  assert.match(control(interval,'button','check-updates'),/aria-busy="false"/);
  assert.equal(disabled(control(interval,'button','check-updates')),false);
  const firstTime=about({network_enabled:true});
  assert.match(firstTime,/上次检查：尚未检查/);assert.match(buttonLabel(firstTime,'update-interval'),/6 小时/);
  checks++;

  const unpublished=about({update:{status:'unpublished',notes:'尚未发布 CodexProxyTray 正式安装包。'}});
  assert.equal((unpublished.match(/暂无公开发布版本/g)||[]).length,1);
  assert.doesNotMatch(unpublished,/尚未发布 CodexProxyTray 正式安装包/);
  checks++;

  const preference=Object.getOwnPropertyDescriptor(globalThis,'localStorage');
  const media=Object.getOwnPropertyDescriptor(globalThis,'matchMedia');
  try {
    for(const [saved,dark,resolved] of [['light',true,'light'],['dark',false,'dark'],['system',true,'dark'],['system',false,'light']]) {
      Object.defineProperty(globalThis,'localStorage',{configurable:true,value:{getItem:key=>key==='color-theme'?saved:null}});
      Object.defineProperty(globalThis,'matchMedia',{configurable:true,value:()=>({matches:dark})});
      const themed=render('settings',sample);
      assert.ok(themed.includes(`data-theme="${resolved}"`));
      assert.match(active(themed,'设置'),/aria-label="界面主题"/);
      assert.match(buttonLabel(themed,'color-theme'),new RegExp({light:'浅色',dark:'深色',system:'跟随系统'}[saved]));
      assert.equal(resolveTheme(readThemeMode(),dark),resolved);
    }
    assert.equal(readThemeMode({getItem:()=> 'invalid'}),'system');
    assert.equal(readThemeMode({getItem:()=>{throw Error('storage unavailable');}}),'system');
  } finally {if(preference)Object.defineProperty(globalThis,'localStorage',preference);else delete globalThis.localStorage;if(media)Object.defineProperty(globalThis,'matchMedia',media);else delete globalThis.matchMedia;}
  checks++;

  const boot=await readFile(resolve(root,'ui/theme-init.js'),'utf8');
  const initial=await readFile(resolve(root,'ui/index.html'),'utf8');
  assert.match(initial,/<title>CodexProxyTray<\/title>/);assert.match(initial,/rel="icon" href="icon\.svg"/);
  assert.ok(initial.indexOf('src="theme-init.js"')<initial.indexOf('rel="stylesheet"'));
  for(const [saved,dark,resolved] of [[null,false,'light'],[null,true,'dark'],['light',true,'light'],['dark',false,'dark'],['system',true,'dark'],['invalid',true,'dark']]) {
    const element={dataset:{},style:{}};
    runInNewContext(boot,{document:{documentElement:element},window:{addEventListener(){},matchMedia:()=>({matches:dark})},localStorage:{getItem:()=>saved}});
    assert.equal(element.dataset.theme,resolved);assert.equal(element.style.colorScheme,resolved);
  }
  const denied={dataset:{},style:{}};
  runInNewContext(boot,{document:{documentElement:denied},window:{addEventListener(){},matchMedia:()=>({matches:true})},localStorage:{getItem:()=>{throw Error('unavailable');}}});
  assert.equal(denied.dataset.theme,'dark');
  checks++;

  assert.match(html,/<main\b[^>]*class="main-scroll"[^>]*>\s*<div\b[^>]*class="main-content"/);
  assert.match(render('logs',sample),/role="tablist"[^>]*aria-label="日志级别"|aria-label="日志级别"[^>]*role="tablist"/);
  assert.match(render('logs',sample),/role="tabpanel"/);
  assert.equal((html.match(/\bid="toggle"/g)||[]).length,1);
  for(const page of ['proxy','settings','logs','about'])assert.doesNotMatch(render(page,sample),/\bid="toggle"/);
  checks++;

  const originalNow=Date.now;
  try {
    Date.now=()=>5000;
    const running=renderToStaticMarkup(React.createElement(GuardButton,{status:{enabled:true,guard_uptime_ms:1000,updated_at:4000},observed:true,writable:true,busy:false,onToggle:noop}));
    assert.match(running,/aria-label="暂停守护"/);assert.match(running,/00:00:02/);
    const paused=renderToStaticMarkup(React.createElement(GuardButton,{status:{enabled:false,guard_uptime_ms:2000,updated_at:4000},observed:true,writable:true,busy:false,onToggle:noop}));
    assert.match(paused,/aria-label="恢复守护"/);assert.match(paused,/guard-paused/);assert.match(paused,/class="guard-elapsed" aria-hidden="true"/);assert.doesNotMatch(paused,/00:00:02/);
    const toggling=renderToStaticMarkup(React.createElement(GuardButton,{status:{enabled:true,guard_uptime_ms:0,updated_at:5000},observed:true,writable:true,busy:true,onToggle:noop}));
    assert.match(control(toggling,'button','toggle'),/aria-busy="true"/);assert.match(control(toggling,'button','toggle'),/aria-disabled="true"/);assert.equal(disabled(control(toggling,'button','toggle')),false);assert.match(toggling,/00:00:00/);
    const unavailable=renderToStaticMarkup(React.createElement(GuardButton,{observed:false,writable:false,busy:false,onToggle:noop}));
    assert.equal(disabled(control(unavailable,'button','toggle')),true);assert.match(unavailable,/—/);
  } finally {Date.now=originalNow;}
  checks++;

  const connectivity=(result,extra={})=>renderToStaticMarkup(React.createElement(ConnectivityCard,{result,observed:true,writable:true,enabled:true,busy:false,onCheck:noop,...extra}));
  const available=connectivity({status:'available',latency_ms:1234,ip:'203.0.113.7',country:'CN',http_status:200,checked_at:1000});
  assert.match(available,/ChatGPT 网页连通性/);assert.match(available,/完整 HTTP 探测耗时/);assert.match(available,/探测出口/);assert.match(available,/203\.0\.113\.7/);assert.match(available,/>CN</);assert.match(available,/网页可达/);
  for(const http of [403,429]){
    const restricted=connectivity({status:'available',latency_ms:200,http_status:http,ip:'203.0.113.7',country:'CN'});
    assert.match(restricted,/访问受限/);assert.doesNotMatch(restricted,/网页可达|203\.0\.113\.7/);
  }
  const connectionBackground=connectivity({checking:true,status:'unchecked'});
  assert.match(connectionBackground,/后台检测中/);assert.match(connectionBackground,/aria-busy="false"/);
  const connectionUnknown=connectivity({}, {observed:false});
  assert.match(connectionUnknown,/尚未检测/);assert.doesNotMatch(connectionUnknown,/0 ms|0\.0/);
  checks++;

  const runningService={state:'running',version};
  const traffic=(result,service=runningService,trafficEnabled=true)=>renderToStaticMarkup(React.createElement(TrafficCard,{result,service,trafficEnabled,observed:true}));
  const collecting=traffic({state:'collecting',receive_bps:2048,send_bps:1024,received_bytes:4096,sent_bytes:0,target_count:2});
  assert.match(collecting,/2\.0 KB\/s/);assert.match(collecting,/1\.0 KB\/s/);assert.match(collecting,/4\.0 KB/);assert.match(collecting,/累计 0 B/);
  assert.match(collecting,/可能漏掉短命进程，不是网卡计费量/);
  for(const state of ['disabled','starting','needs_permission','unavailable']){
    const inactive=traffic({state,receive_bps:2048,send_bps:1024,received_bytes:4096,sent_bytes:0,target_count:2});
    assert.doesNotMatch(inactive,/2\.0 KB\/s|累计 0 B|授权采集/);
    assert.doesNotMatch(inactive,/<button|前往设置|安装辅助服务|卸载辅助服务/);
  }
  const trafficStarting=traffic({state:'starting'});
  assert.match(trafficStarting,/正在启动/);assert.doesNotMatch(trafficStarting,/<button/);
  const missingService=traffic({state:'collecting',receive_bps:2048},{state:'not_installed'});
  assert.match(missingService,/暂不可用/);assert.doesNotMatch(missingService,/安装辅助服务|前往设置|2\.0 KB\/s/);
  const disabledTraffic=traffic({state:'collecting',receive_bps:2048},runningService,false);
  assert.match(disabledTraffic,/未开启/);assert.doesNotMatch(disabledTraffic,/2\.0 KB\/s|<button/);
  assert.doesNotMatch(active(render('dashboard',sample),'仪表盘'),/前往设置|安装辅助服务|卸载辅助服务|记录网络流量|目标进程流量/);
  assert.match(settingsPanel(render('settings',sample),'general'),/目标进程流量/);
  assert.equal(trafficAmount(null,true),'—');assert.equal(trafficAmount(-1),'—');
  checks++;

  const collector=(service,result={state:'disabled'},pending=new Set())=>renderToStaticMarkup(React.createElement(CollectorSettings,{result,service,trafficEnabled:false,observed:true,writable:true,busy:name=>pending.has(name),onEnable:noop,onDisable:noop,onInstall:noop,onUninstall:noop}));
  const uninstalled=collector({state:'not_installed',version:'9.9.9'});
  assert.match(uninstalled,/未安装/);assert.ok(control(uninstalled,'button','install-traffic-service'));assert.doesNotMatch(uninstalled,/\bid="uninstall-traffic-service"|版本 9\.9\.9/);
  assert.equal(disabled(control(uninstalled,'input','traffic-enabled')),true);
  const installed=collector({state:'stopped',version});
  assert.match(installed,/已停止/);assert.ok(installed.includes(`版本 ${version}`));assert.match(installed,/卸载辅助服务/);
  assert.doesNotMatch(installed,/\bid="install-traffic-service"/);
  assert.equal(disabled(control(installed,'input','traffic-enabled')),false);
  assert.doesNotMatch(installed,/授权采集|不安装 Windows 服务/);
  const outdated=collector({state:'outdated',version:'0.0.1'});
  assert.match(outdated,/版本不匹配/);assert.match(outdated,/辅助服务与当前应用不匹配，请先卸载后重新安装/);assert.match(outdated,/卸载辅助服务/);
  assert.doesNotMatch(outdated,/更新辅助服务|\bid="install-traffic-service"/);
  assert.equal(disabled(control(outdated,'input','traffic-enabled')),true);
  const deniedCollector=collector({state:'error',version:null,error:'无法读取服务状态'});
  assert.match(deniedCollector,/无法读取服务状态/);assert.doesNotMatch(deniedCollector,/\bid="(?:install|uninstall)-traffic-service"|版本 /);
  const cancelled=collector({state:'not_installed',error:'已取消权限授权'});
  assert.match(cancelled,/已取消权限授权/);
  const installing=collector({state:'not_installed'},{state:'disabled'},new Set(['install-traffic-service']));
  assert.match(control(installing,'button','install-traffic-service'),/aria-busy="true"/);
  assert.match(control(installing,'input','traffic-enabled'),/aria-busy="false"/);assert.equal(disabled(control(installing,'input','traffic-enabled')),true);
  checks++;

  const ownSettings=collector(runningService,{state:'starting'},new Set(['settings-enable-traffic']));
  assert.match(control(ownSettings,'input','traffic-enabled'),/aria-busy="true"/);
  assert.match(control(ownSettings,'button','uninstall-traffic-service'),/aria-busy="false"/);assert.equal(disabled(control(ownSettings,'button','uninstall-traffic-service')),true);
  const stoppingSettings=collector(runningService,{state:'collecting'},new Set(['settings-disable-traffic']));
  assert.match(control(stoppingSettings,'input','traffic-enabled'),/aria-busy="true"/);assert.match(control(stoppingSettings,'button','uninstall-traffic-service'),/aria-busy="false"/);
  checks++;
  const normalStatus=renderToStaticMarkup(React.createElement(StatusLabel,null,'待检测'));
  assert.match(normalStatus,/data-status="neutral"/);assert.match(normalStatus,/status-label-dot[^>]*aria-hidden="true"/);
  const readyStatus=renderToStaticMarkup(React.createElement(StatusLabel,{color:'success'},'已接入'));
  assert.match(readyStatus,/data-status="success"/);assert.doesNotMatch(readyStatus,/fui-Badge/);
  assert.doesNotMatch(render('dashboard'),/data-status="success"|fui-Badge/);
  assert.doesNotMatch(connectionBackground,/data-status="success"/);
  checks++;
  console.log(`React SSR UI checks passed: ${checks}. No browser or desktop automation was used.`);
} finally {
  // Only this script's own intermediate is removed; no repository/source cleanup.
  await rm(output,{force:true});
  await rmdir(directory).catch(error=>{if(!['ENOTEMPTY','ENOENT'].includes(error.code))throw error;});
}
