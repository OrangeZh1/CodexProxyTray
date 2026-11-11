import React, {useEffect,useRef,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {FluentProvider,webDarkTheme,Button,Input,Select,Switch,Badge,MessageBar,MessageBarBody,MessageBarActions} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {createActions,acceptSnapshot,systemPreferences} from './actions.mjs';
import {duration,probePath} from './metrics.mjs';
import {About} from './about.jsx';
import './style.css';

const native=typeof window==='undefined'?null:window.__TAURI__;
const invoke=native?.core?.invoke;
const appWindow=native?.window?.getCurrentWindow();
const version=__APP_VERSION__;
const pages={dashboard:'仪表盘',proxy:'代理',settings:'设置',logs:'运行日志',about:'关于'};
const theme={...webDarkTheme,fontFamilyBase:'"Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", sans-serif',colorNeutralBackground1:'#242a35',colorNeutralBackground2:'#202631',colorNeutralForeground1:'#f2f3f5',colorNeutralForeground2:'#c1c6d0',colorBrandBackground:'#0f6cbd',colorBrandBackgroundHover:'#115ea3',colorBrandForeground1:'#75bbff'};
const names={dashboard:'grid-24-regular',settings:'settings-24-regular',logs:'text-bullet-list-ltr-24-regular',about:'info-24-regular',shield:'shield-checkmark-24-regular',proxy:'plug-connected-24-regular',desktop:'desktop-24-regular',terminal:'window-console-20-regular',power:'power-24-regular',open:'open-24-regular',refresh:'arrow-sync-24-regular',save:'save-24-regular',success:'checkmark-circle-24-regular',warning:'warning-24-regular',dismiss:'dismiss-24-regular',minimize:'subtract-24-regular',maximize:'maximize-24-regular',restore:'square-multiple-24-regular',collapse:'panel-left-contract-24-regular',expand:'panel-left-expand-24-regular',pulse:'pulse-24-regular',clock:'clock-24-regular',chart:'data-line-24-regular'};
function Symbol({name,spin=false}){return <Icon icon={icons[names[name]]} className={spin?'symbol spin':'symbol'} aria-hidden="true"/>;}
const errorText=error=>error?.name==='AbortError'?'操作已取消。':String(error?.message||error||'操作失败，请重试。');
const time=at=>new Date(at).toLocaleTimeString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false});
function Row({icon,title,description,children,className=''}){return <div className={`setting-row ${className}`}><Symbol name={icon}/><div className="row-copy"><div className="row-title">{title}</div>{description&&<div className="row-description">{description}</div>}</div><div className="row-control">{children}</div></div>;}
function Metric({icon,label,value,detail}){return <article className="metric-card"><div className="metric-label"><Symbol name={icon}/>{label}</div><strong className="metric-value">{value}</strong><div className="metric-detail">{detail}</div></article>;}
function EventList({logs,compact=false}){return <ol className={`logs-list ${compact?'compact-events':''}`}>{logs.length?logs.map((log,index)=><li key={log.id||`${log.at}-${index}`} className={`log-entry level-${log.level}`}><time dateTime={new Date(log.at).toISOString()}>{time(log.at)}</time><Symbol name={log.level==='error'||log.level==='warning'?'warning':'success'}/><div className="log-copy"><div className="log-message"><span className="log-category">{log.category||'运行'}</span>{log.message}</div>{!compact&&log.details&&<div className="log-details">{log.details}</div>}</div></li>):<li className="empty-state">暂无日志</li>}</ol>;}

export function App({initialStatus=null,initialPage='dashboard'}={}){
  const [page,setPage]=useState(Object.hasOwn(pages,initialPage)?initialPage:'dashboard');
  const [collapsed,setCollapsed]=useState(()=>{try{return localStorage.getItem('sidebar-collapsed')==='1';}catch{return false;}});
  const [maximized,setMaximized]=useState(false),[status,setStatus]=useState(initialStatus),current=useRef(initialStatus);
  const [online,setOnline]=useState(!!initialStatus),[,setActionVersion]=useState(0);
  const actions=useRef(null);if(!actions.current)actions.current=createActions(()=>setActionVersion(v=>v+1),{minimumVisibleMs:650});
  const [notice,setNotice]=useState(null),[proxy,setProxy]=useState(initialStatus?.proxy_url||'');
  const proxyEdit=useRef({dirty:false,version:0}),systemPending=useRef(new Set());
  const [system,setSystem]=useState(()=>systemPreferences({},initialStatus||{},new Set()));
  const [directory,setDirectory]=useState(initialStatus?.cli_workdir||''),directoryEdit=useRef({dirty:false,version:0});
  const [logFilter,setLogFilter]=useState('all'),reads=useRef(0);
  const busy=name=>actions.current.has(name);
  const writable=!!invoke&&online&&status?.backend_ready&&!status?.read_only;
  const observed=online&&!!status&&status.revision>0;
  const show=(text,intent='info')=>setNotice({text,intent});
  function accept(data){
    if(!data||!Number.isFinite(data.revision)||!Array.isArray(data.logs))throw Error('后台状态无效。');
    const next=acceptSnapshot(current.current,data);current.current=next;reads.current++;setStatus(next);setOnline(true);
    setNotice(value=>value?.connection?null:value);
    if(!proxyEdit.current.dirty)setProxy(next.proxy_url);
    setSystem(value=>systemPreferences(value,next,systemPending.current));
    if(!directoryEdit.current.dirty)setDirectory(next.cli_workdir||'');
    return next;
  }
  useEffect(()=>{
    let disposed=false,pending=false;
    async function poll(){if(!invoke||pending)return;pending=true;const before=reads.current;try{const data=await invoke('get_status');if(!disposed)accept(data);}catch{if(!disposed&&reads.current===before){setOnline(false);setNotice(value=>value&&!value.connection?value:{text:'后台连接中断，正在重试。',intent:'warning',connection:true});}}finally{pending=false;}}
    poll();const timer=setInterval(poll,2500);return()=>{disposed=true;clearInterval(timer);};
  },[]);
  useEffect(()=>{
    let disposed=false;const stops=[];
    const failed=()=>{if(!disposed)show('窗口状态同步失败，请重新打开助手。','warning');};
    const registered=promise=>promise.then(stop=>disposed?stop():stops.push(stop)).catch(failed);
    if(native?.event?.listen)registered(native.event.listen('navigate',event=>{if(Object.hasOwn(pages,event.payload))setPage(event.payload);}));
    if(appWindow){const sync=()=>appWindow.isMaximized().then(value=>{if(!disposed)setMaximized(value);}).catch(failed);sync();registered(appWindow.onResized(sync));}
    return()=>{disposed=true;for(const stop of stops)stop();};
  },[]);
  async function run(name,command,args,success){await actions.current.run(name,async()=>{try{const data=await invoke(command,args);if(data)accept(data);if(success)show(success,'success');}catch(error){show(errorText(error),'error');}});}
  async function saveProxy(event){
    event.preventDefault();if(!writable||busy('toggle'))return;const submitted=proxyEdit.current.version;
    await actions.current.run('save',async()=>{try{const data=await invoke('save_proxy',{proxyUrl:proxy.trim()});if(submitted===proxyEdit.current.version)proxyEdit.current.dirty=false;const next=accept(data);show(proxyEdit.current.dirty?'已应用；当前输入还有未保存的更改。':next.restart_required?'已应用，重开 ChatGPT 后生效。':'代理设置已应用。',proxyEdit.current.dirty?'info':'success');}catch(error){show(errorText(error),'error');}});
  }
  async function changeSystem(key,value){
    const name={autostart:'autostart',launchTarget:'launch-target',cliWorkdir:'cli-workdir',networkEnabled:'network-enabled',autoCheckUpdates:'auto-check-updates',updateIntervalHours:'update-interval'}[key];if(!name||!writable||busy(name))return;
    systemPending.current.add(key);setSystem(previous=>({...previous,[key]:value}));
    await actions.current.run(name,async()=>{try{const data=await invoke('save_system',{autostart:null,launchTarget:null,cliWorkdir:null,networkEnabled:null,autoCheckUpdates:null,updateIntervalHours:null,[key]:value});systemPending.current.delete(key);accept(data);}catch(error){systemPending.current.delete(key);setSystem(previous=>systemPreferences(previous,current.current||{},systemPending.current));show(errorText(error),'error');}});
  }
  async function saveDirectory(event){event?.preventDefault();if(!directoryEdit.current.dirty||busy('cli-workdir'))return;const submitted=directoryEdit.current.version;await changeSystem('cliWorkdir',directory.trim());if(submitted===directoryEdit.current.version&&current.current?.cli_workdir===directory.trim()){directoryEdit.current.dirty=false;setDirectory(current.current.cli_workdir);}}
  async function checkUpdates(){await actions.current.run('check-updates',async()=>{try{await invoke('check_updates');accept(await invoke('get_status'));}catch(error){try{accept(await invoke('get_status'));}catch{}show(errorText(error),'error');}});}
  async function windowAction(action){try{await appWindow?.[action]();}catch(error){show(errorText(error),'error');}}
  function fold(){setCollapsed(value=>{try{localStorage.setItem('sidebar-collapsed',value?'0':'1');}catch{}return !value;});}
  const button=(id,text,icon,onClick,{primary=false,disabled=false,type='button',iconOnly=false}={})=><Button id={id} type={type} className={iconOnly?'icon-action':'text-action'} appearance={primary?'primary':'subtle'} icon={busy(id)?<Symbol name="refresh" spin/>:icon?<Symbol name={icon}/>:undefined} aria-label={iconOnly?text:undefined} title={iconOnly?text:undefined} aria-busy={busy(id)} disabled={!invoke||busy(id)||disabled} onClick={onClick}>{iconOnly?null:text}</Button>;
  const ready=status?.enabled&&(status?.guard_ready||status?.cli_ready)&&status?.proxy_reachable&&!status?.error;
  const guardText=!status?'正在连接后台':!online?'后台未连接':!status.backend_ready?'助手未启动':!status.enabled?'守护已暂停':status.error?'守护需要检查':ready?'守护已就绪':!status.proxy_reachable?'等待本地代理':'等待应用接入';
  const logs=status?.logs.slice().reverse()||[],filtered=logs.filter(log=>logFilter==='all'||log.level===logFilter);
  const samples=status?.probe_history||[],path=probePath(samples);
  const guardControl=<div className="switch-control">{busy('toggle')&&<Symbol name="refresh" spin/>}<Switch id="toggle" aria-label="代理守护" checked={!!status?.enabled} disabled={!writable||busy('toggle')||busy('save')} aria-busy={busy('toggle')} onChange={(_,data)=>run('toggle','set_enabled',{enabled:data.checked})}/></div>;
  const aboutStatus={...(status||{}),network_enabled:system.networkEnabled,auto_check_updates:system.autoCheckUpdates,update_interval_hours:system.updateIntervalHours};

  return <FluentProvider theme={theme} className={`app-shell ${collapsed?'sidebar-collapsed':''}`}>
    <div className="window-drag" data-tauri-drag-region/>
    <div className="window-controls" aria-label="窗口控制"><Button appearance="transparent" aria-label="最小化" title="最小化" disabled={!appWindow} icon={<Symbol name="minimize"/>} onClick={()=>windowAction('minimize')}/><Button appearance="transparent" aria-label={maximized?'还原':'最大化'} title={maximized?'还原':'最大化'} disabled={!appWindow} icon={<Symbol name={maximized?'restore':'maximize'}/>} onClick={()=>windowAction('toggleMaximize')}/><Button appearance="transparent" className="window-close" aria-label="关闭到托盘" title="关闭到托盘" disabled={!appWindow} icon={<Symbol name="dismiss"/>} onClick={()=>windowAction('close')}/></div>
    <aside className="sidebar">
      <div className="brand" data-tauri-drag-region><img src="icon.svg" alt="" data-tauri-drag-region/>{!collapsed&&<strong data-tauri-drag-region>CodexProxyTray</strong>}<Button appearance="subtle" className="sidebar-toggle" aria-label={collapsed?'展开侧边栏':'收起侧边栏'} title={collapsed?'展开侧边栏':'收起侧边栏'} icon={<Symbol name={collapsed?'expand':'collapse'}/>} onClick={fold}/></div>
      <nav aria-label="页面导航">{Object.entries(pages).map(([key,label])=><Button key={key} data-page={key} appearance="subtle" icon={<Symbol name={key}/>} className={`nav-item ${page===key?'selected':''}`} aria-current={page===key?'page':undefined} aria-label={label} title={collapsed?label:undefined} onClick={()=>setPage(key)}>{!collapsed&&label}</Button>)}</nav>
      <div className="sidebar-footer" title={status?.read_only?'只读预览':online?'后台运行中':'后台未连接'}><span className={`status-indicator ${online&&status?.backend_ready?'active':''}`}/>{!collapsed&&<><span>{status?.read_only?'只读预览':online?'后台运行中':'后台未连接'}</span><span className="version">v{version}</span></>}</div>
    </aside>
    <main>
      <header className="page-header" data-tauri-drag-region><h1 data-tauri-drag-region>{pages[page]}</h1><div className="header-actions">{['dashboard','proxy'].includes(page)&&button('refresh','重新检测','refresh',()=>run('refresh','refresh_status'),{iconOnly:true})}{page==='logs'&&button('refresh-logs','刷新日志','refresh',()=>run('refresh-logs','get_status'),{iconOnly:true})}</div></header>
      {notice&&<MessageBar intent={notice.intent} icon={<Symbol name={notice.intent==='success'?'success':'warning'}/>}><MessageBarBody>{notice.text}</MessageBarBody><MessageBarActions containerAction={<Button appearance="transparent" size="small" aria-label="关闭提示" icon={<Symbol name="dismiss"/>} onClick={()=>setNotice(null)}/>}/></MessageBar>}
      {!invoke&&<MessageBar intent="warning" icon={<Symbol name="warning"/>}><MessageBarBody>请运行 CodexProxyTray 桌面程序。</MessageBarBody></MessageBar>}
      {status?.read_only&&<div className="preview-note">本机真实状态 · 修改设置请使用桌面窗口</div>}
      <section hidden={page!=='dashboard'} aria-label="仪表盘">
        <div className="dashboard-grid">
          <article className="metric-card probe-card"><div className="metric-label"><Symbol name="chart"/>本地代理连接耗时<span className="live-caption">{samples.length?`${samples.length} 次${online?'':'历史'}采样`:'等待采样'}</span></div><div className="probe-value">{observed&&Number.isFinite(status?.proxy_latency_ms)?status.proxy_latency_ms.toFixed(2):'—'}<span>ms</span><Badge appearance="tint" color={observed&&status?.proxy_reachable?'success':'subtle'}>{!observed?'待检测':status.proxy_reachable?'可连接':'未连接'}</Badge></div>{path?<svg viewBox="0 0 600 150" preserveAspectRatio="none" role="img" aria-label="本地代理 TCP 连接耗时趋势"><path d={path}/></svg>:<div className="chart-empty">等待本地代理采样</div>}<div className="chart-caption"><span>{samples.length?time(samples[0].at):'—'}</span><code>{status?.proxy_url||'—'}</code><span>{samples.length?time(samples.at(-1).at):'—'}</span></div></article>
          <article className="metric-card guard-card"><div className="metric-label"><Symbol name="shield"/>代理守护</div><strong className="guard-value">{guardText}</strong>{guardControl}<div className="guard-targets"><span>ChatGPT</span><Badge appearance="tint" color={observed&&status?.guard_ready?'success':'subtle'}>{!observed?'待检测':status.guard_ready?'已就绪':'未接入'}</Badge><span>Codex CLI</span><Badge appearance="tint" color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</Badge></div></article>
          <Metric icon="desktop" label="ChatGPT" value={!observed?'—':status.codex_running?'运行中':'未运行'} detail={!observed?'等待应用状态':status.desktop_pid?`PID ${status.desktop_pid}`:status.package_version?`已安装 ${status.package_version}`:'尚未检测到应用'}/>
          <Metric icon="pulse" label="ChatGPT TCP 连接" value={observed&&Number.isFinite(status?.tcp_connections)?status.tcp_connections:'—'} detail="应用进程树中的已建立连接"/>
          <Metric icon="clock" label="本次守护时长" value={observed&&status?.started_at?duration(status.updated_at-status.started_at):'—'} detail={observed&&Number.isFinite(status?.helper_memory_bytes)?`守护进程工作集 ${(status.helper_memory_bytes/1048576).toFixed(1)} MB`:'守护进程资源采样中'}/>
        </div>
        {status?.error&&<div className="inline-note error-text"><Symbol name="warning"/>{status.error}</div>}
        {status?.restart_required&&status.enabled&&<div className="inline-note restart-hint"><Symbol name="about"/><span>当前 ChatGPT 需完整退出并重开一次。</span></div>}
        <div className="section-heading"><h2>最近事件</h2><Button appearance="subtle" size="small" onClick={()=>setPage('logs')}>全部日志</Button></div><EventList logs={logs.slice(0,4)} compact/>
      </section>
      <section hidden={page!=='proxy'} aria-label="代理">
        <form onSubmit={saveProxy} className="settings-group"><Row icon="proxy" title="本地 HTTP 代理" description={<><span className={`status-indicator ${observed&&status?.proxy_reachable?'active':''}`}/>{!observed?'待检测':status.proxy_reachable?'端口可用':'暂未连接'}</>} className="proxy-row"><Input id="proxy-url" aria-label="本地 HTTP 代理地址" value={proxy} readOnly={!writable} spellCheck={false} onChange={(_,data)=>{proxyEdit.current.dirty=true;proxyEdit.current.version++;setProxy(data.value);}}/>{button('save','应用',null,undefined,{type:'submit',primary:true,disabled:!writable||busy('toggle')})}</Row></form>
        <h2>应用接入</h2><div className="settings-group targets"><Row icon="desktop" title="ChatGPT" description={!observed?'等待应用状态':status.package_count?`版本 ${status.package_version||'已安装'}`:'未检测到桌面应用'}><Badge appearance="tint" color={observed&&status?.codex_running?'success':'subtle'}>{!observed?'待检测':status.codex_running?'运行中':'未运行'}</Badge>{button('launch','打开 ChatGPT','open',()=>run('launch','launch_codex'),{disabled:!writable||!status?.package_count,iconOnly:true})}</Row><Row icon="terminal" title="Codex CLI" description={!observed?'等待 CLI 状态':status.cli_ready?'通过专用入口在终端中运行。':status.cli_detected?'已找到 Windows CLI':'未检测到 Windows CLI'}><Badge appearance="tint" color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</Badge>{button('launch-cli','打开 Codex CLI','open',()=>run('launch-cli','launch_cli'),{disabled:!writable||!status?.cli_ready,iconOnly:true})}</Row></div>
      </section>
      <section hidden={page!=='settings'} aria-label="设置"><h2 className="first-section">启动</h2><div className="settings-group"><Row icon="power" title="登录时自动启动" description="登录 Windows 后在托盘运行。"><div className="switch-control">{busy('autostart')&&<Symbol name="refresh" spin/>}<Switch id="autostart" aria-label="登录时自动启动" aria-busy={busy('autostart')} checked={system.autostart} disabled={!writable||busy('autostart')} onChange={(_,data)=>changeSystem('autostart',data.checked)}/></div></Row><Row icon="desktop" title="启动后打开" description="助手启动时自动打开所选应用。"><div className="switch-control">{busy('launch-target')&&<Symbol name="refresh" spin/>}<Select id="launch-target" aria-label="启动后打开" aria-busy={busy('launch-target')} value={system.launchTarget} icon={<Icon icon={icons['chevron-down-24-regular']} width={16} height={16} aria-hidden="true"/>} disabled={!writable||busy('launch-target')} onChange={(_,data)=>changeSystem('launchTarget',data.value)}><option value="none">不打开应用</option><option value="chatgpt">ChatGPT</option><option value="cli">Codex CLI</option><option value="both">ChatGPT 与 Codex CLI</option></Select></div></Row></div><h2>Codex CLI</h2><form className="settings-group" onSubmit={saveDirectory}><Row icon="terminal" title="工作目录" description="留空使用用户目录。离开输入框时自动保存。" className="workdir-row"><Input id="cli-workdir" aria-label="Codex CLI 工作目录" value={directory} placeholder="使用用户目录" aria-busy={busy('cli-workdir')} readOnly={!writable||busy('cli-workdir')} spellCheck={false} onChange={(_,data)=>{directoryEdit.current.dirty=true;directoryEdit.current.version++;setDirectory(data.value);}} onBlur={saveDirectory}/>{busy('cli-workdir')&&<Symbol name="refresh" spin/>}</Row></form></section>
      <section hidden={page!=='logs'} aria-label="运行日志"><div className="log-toolbar"><div className="log-filters">{[['all','全部'],['warning','警告'],['error','错误']].map(([key,label])=><Button key={key} appearance={logFilter===key?'secondary':'subtle'} size="small" aria-pressed={logFilter===key} onClick={()=>setLogFilter(key)}>{label}</Button>)}</div><span>保留最近 200 条</span></div>{status?.log_storage_error&&<div className="inline-note error-text">历史日志保存异常：{status.log_storage_error}</div>}<EventList logs={filtered}/></section>
      <section hidden={page!=='about'} aria-label="关于"><About status={aboutStatus} version={version} busy={busy} writable={writable} onPreference={changeSystem} onCheck={checkUpdates} onSource={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/CodexProxyTray','_blank','noopener,noreferrer');return;}run('open-source','open_source');}} onRelease={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/chatgpt-proxy-launcher/releases','_blank','noopener,noreferrer');return;}run('open-release','open_release');}} onLogs={writable?()=>run('open-logs','open_logs'):undefined}/></section>
    </main>
  </FluentProvider>;
}
if(typeof document!=='undefined')createRoot(document.getElementById('root')).render(<App/>);
