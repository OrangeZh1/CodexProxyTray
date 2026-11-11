// Production bundle mounted in a Node DOM model. No browser or desktop automation.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {setTimeout as delay} from 'node:timers/promises';
import {transform} from 'esbuild';
import {Window} from 'happy-dom';

const script=await readFile(new URL('../dist/app.js',import.meta.url),'utf8');
const {code}=await transform(script,{format:'iife',target:'chrome109'});
const boot=await readFile(new URL('../ui/theme-init.js',import.meta.url),'utf8');
const styles=await readFile(new URL('../dist/app.css',import.meta.url),'utf8');
const now=Date.now();
const sample={revision:7,enabled:true,guard_ready:true,backend_ready:true,proxy_url:'http://127.0.0.1:7890',proxy_reachable:true,proxy_latency_ms:0.25,probe_history:[{at:now,latency_ms:0.25}],package_count:1,package_version:'26.1',codex_running:true,desktop_pid:42,tcp_connections:12,helper_memory_bytes:8388608,started_at:now-1000,updated_at:now,cli_detected:true,cli_ready:true,launch_target:'none',network_enabled:true,connectivity_enabled:true,traffic_enabled:false,traffic_service:{state:'not_installed',version:null,error:null},traffic:{state:'disabled',message:'尚未启用统计'},connectivity:{status:'available',checked_at:now,http_status:200,latency_ms:1200,ip:'203.0.113.1',country:'US'},update:{status:'unchecked'},logs:[{id:now,at:now,level:'info',category:'生命周期',message:'守护进程已启动',details:'客户端启动回归'}]};
for(const view of ['main','tray'])for(const theme of ['light','dark']){
  const window=new Window({url:'http://tauri.localhost/',settings:{enableJavaScriptEvaluation:true,disableJavaScriptFileLoading:true,disableCSSFileLoading:true,suppressInsecureJavaScriptEnvironmentWarning:true}});
  const errors=[],commands=[];
  try{
    window.document.body.innerHTML='<div id="root"></div>';
    const style=window.document.createElement('style');style.textContent=styles;window.document.head.append(style);
    if(view==='tray')window.document.documentElement.dataset.view='tray';
    // Edge/WebView2 150+ returns a Promise from scrollTo. It must never become an effect cleanup.
    if(view==='main')window.HTMLElement.prototype.scrollTo=function(){return Promise.resolve();};
    window.localStorage.setItem('color-theme',theme);
    window.console.error=(...args)=>errors.push(args.map(String).join(' '));
    window.addEventListener('error',event=>errors.push(String(event.error||event.message)));
    let focusChanged,hideCount=0,refreshResolve;
    const appWindow={label:view,hide:async()=>{hideCount++;},setTheme:async()=>{},setBackgroundColor:async()=>{throw Error('Do not repaint the native background after mounting.');},isMaximized:async()=>false,onResized:async()=>()=>{},onFocusChanged:async callback=>{focusChanged=callback;return()=>{};}};
    window.__TAURI__={core:{invoke:async(command,args)=>{const firstReady=!commands.includes('frontend_ready');commands.push(command);if(command==='report_ui_error'){errors.push(args.message);return null;}if(command==='frontend_ready'){if(view==='tray'&&firstReady)focusChanged?.({payload:true});return null;}if(command==='get_status')return structuredClone(sample);if(command==='refresh_status')return new Promise(resolve=>{refreshResolve=resolve;});throw Error(`Unexpected startup command: ${command}`);}},window:{getCurrentWindow:()=>appWindow},event:{listen:async()=>()=>{}}};
    window.eval(boot);window.eval(code);
    for(let i=0;i<150&&!errors.length&&(!commands.includes('frontend_ready')||!(view==='tray'?window.document.querySelector('.tray-header')?.textContent.includes('守护已就绪'):window.document.querySelector('.sidebar-footer')?.textContent.includes('后台运行中')));i++)await delay(20);
    assert.deepEqual(errors,[],`${view}/${theme}: client render errors`);
    assert.ok(commands.includes('frontend_ready'),`${theme}: frontend did not finish mounting`);
    assert.ok(commands.includes('get_status'),`${theme}: status was not requested`);
    assert.equal(window.document.querySelectorAll('.app-shell').length,view==='main'?1:0,'Page layout must not be copied into a Fluent portal.');
    assert.equal(window.document.querySelectorAll('.tray-surface').length,view==='tray'?1:0,'Tray layout must not be copied into a Fluent portal.');
    if(view==='main'){
      assert.equal(window.document.querySelector('h1')?.textContent,'仪表盘');
      assert.match(window.document.querySelector('.sidebar-footer')?.textContent||'',/后台运行中/);
      assert.match(window.document.querySelector('#root').textContent,/网页可达/);
      const settingsEntry=[...window.document.querySelectorAll('button')].find(button=>button.textContent.trim()==='前往设置')||window.document.querySelector('[data-page="settings"]');
      settingsEntry.click();await delay(50);
      assert.deepEqual(errors,[],'Dashboard to settings must not call a scroll Promise as a cleanup function.');
      assert.equal(window.document.querySelector('h1')?.textContent,'设置');
      for(const tab of ['about','general']){window.document.querySelector(`#settings-tab-${tab}`).click();await delay(30);assert.equal(window.document.querySelector(`#settings-panel-${tab}`).hidden,false);assert.deepEqual(errors,[]);}
      for(const page of ['dashboard','proxy','logs','settings','dashboard']){window.document.querySelector(`[data-page="${page}"]`).click();await delay(30);assert.deepEqual(errors,[],`Navigation failed at ${page}`);assert.ok(window.document.querySelector('.sidebar'));}
    }else{
      assert.equal(window.document.querySelectorAll('[role="menuitem"]').length,8);
      assert.equal(window.document.activeElement?.getAttribute('role'),'menu');
      const refresh=window.document.querySelector('#tray-refresh'),toggle=window.document.querySelector('#tray-toggle');
      refresh.click();await delay(50);
      assert.equal(refresh.getAttribute('aria-busy'),'true');assert.equal(toggle.getAttribute('aria-busy'),'false');assert.equal(hideCount,0);
      refreshResolve(structuredClone(sample));
      for(let i=0;i<50&&!hideCount;i++)await delay(20);
      assert.equal(refresh.getAttribute('aria-busy'),'false');assert.equal(hideCount,1);
      assert.deepEqual(errors,[]);
    }
    assert.equal(window.document.documentElement.dataset.theme,theme);
    console.log(`Client startup passed: ${view}/${theme}, production bundle and backend reply.`);
  }finally{await window.happyDOM.close();}
}
