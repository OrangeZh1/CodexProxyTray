import React, {useEffect,useLayoutEffect,useRef,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {FluentProvider,Button,Input,Select,Switch,MessageBar,MessageBarBody,MessageBarActions,TabList,Tab,Tooltip} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {createActions,acceptSnapshot,systemPreferences} from './actions.mjs';
import {duration,probePath} from './metrics.mjs';
import {About} from './about.jsx';
import {StatusLabel} from './status-label.jsx';
import {themes,themeKey,themeModes,systemThemeQuery,readThemeMode,resolveTheme,prefersDark,applyDocumentTheme} from './themes.mjs';
import './style.css';

const native=typeof window==='undefined'?null:window.__TAURI__;
const invoke=native?.core?.invoke;
const appWindow=native?.window?.getCurrentWindow();
const version=__APP_VERSION__;
const pages={dashboard:'仪表盘',proxy:'代理',logs:'运行日志',settings:'设置'};
const names={dashboard:'grid-24-regular',settings:'settings-24-regular',logs:'text-bullet-list-ltr-24-regular',about:'info-24-regular',shield:'shield-checkmark-24-regular',proxy:'plug-connected-24-regular',desktop:'desktop-24-regular',terminal:'window-console-20-regular',power:'power-24-regular',open:'open-24-regular',refresh:'arrow-sync-24-regular',save:'save-24-regular',success:'checkmark-circle-24-regular',warning:'warning-24-regular',dismiss:'dismiss-24-regular',minimize:'subtract-24-regular',maximize:'maximize-24-regular',restore:'square-multiple-24-regular',collapse:'navigation-24-regular',expand:'navigation-24-regular',pulse:'pulse-24-regular',clock:'clock-24-regular',chart:'data-line-24-regular',pause:'pause-24-regular',play:'play-24-regular'};
export function Symbol({name,spin=false}){return <Icon icon={icons[names[name]]} className={spin?'symbol spin':'symbol'} aria-hidden="true"/>;}
const useThemeLayoutEffect=typeof document==='undefined'?useEffect:useLayoutEffect;
async function syncNativeAppearance(mode){
  if(!appWindow)return;
  await appWindow.setTheme?.(mode==='system'?null:mode);
  const background=typeof document==='undefined'?'':getComputedStyle(document.documentElement).getPropertyValue('--app-bg').trim();
  if(background)await appWindow.setBackgroundColor?.(background);
}
const errorText=error=>error?.name==='AbortError'?'操作已取消。':String(error?.message||error||'操作失败，请重试。');
const time=at=>new Date(at).toLocaleTimeString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false});
function Row({icon,title,description,children,className=''}){return <div className={`setting-row ${className}`}><Symbol name={icon}/><div className="row-copy"><div className="row-title">{title}</div>{description&&<div className="row-description">{description}</div>}</div><div className="row-control">{children}</div></div>;}
export function Metric({icon,label,value,detail}){return <article className="metric-card"><div className="metric-label"><Symbol name={icon}/>{label}</div><strong className="metric-value">{value}</strong><div className="metric-detail">{detail}</div></article>;}
function EventList({logs,compact=false}){return <ol className={`logs-list ${compact?'compact-events':''}`}>{logs.length?logs.map((log,index)=><li key={log.id||`${log.at}-${index}`} className={`log-entry level-${log.level}`}><time dateTime={new Date(log.at).toISOString()}>{time(log.at)}</time><Symbol name={log.level==='error'||log.level==='warning'?'warning':'success'}/><div className="log-copy"><div className="log-message"><span className="log-category">{log.category||'运行'}</span>{log.message}</div>{!compact&&log.details&&<div className="log-details">{log.details}</div>}</div></li>):<li className="empty-state">暂无日志</li>}</ol>;}

export function GuardButton({status,observed,writable,busy,onToggle}){
  const [now,setNow]=useState(()=>Date.now());
  const enabled=!!status?.enabled;
  useEffect(()=>{
    if(!observed||!enabled)return;
    setNow(Date.now());const timer=setInterval(()=>setNow(Date.now()),1000);
    return()=>clearInterval(timer);
  },[observed,enabled]);
  const known=observed&&Number.isFinite(status?.guard_uptime_ms);
  const milliseconds=known?status.guard_uptime_ms+(enabled?Math.max(0,now-(status.updated_at||now)):0):null;
  const label=enabled?'暂停守护':'恢复守护';
  return <Button id="toggle" className={`guard-floating ${enabled?'guard-running':'guard-paused'} ${observed?'':'guard-unknown'}`} appearance="primary" aria-label={label} title={label} aria-busy={busy} disabled={!observed||!writable||busy} icon={<Symbol name={busy?'refresh':enabled?'pause':'play'} spin={busy}/>} onClick={onToggle}>{enabled||!observed?<span className="guard-elapsed">{known?duration(milliseconds):'—'}</span>:null}</Button>;
}

export function ConnectivityCard({result,observed,writable,enabled,busy,onCheck}){
  const data=result||{};
  const blocked=data.status==='blocked'||[403,429].includes(data.http_status);
  const state=!observed?'unchecked':!enabled?'disabled':blocked?'blocked':data.status||'unchecked';
  const checking=enabled&&!!data.checking;
  const labels={unchecked:'尚未检测',available:'网页可达',blocked:'访问受限',error:'检测失败',disabled:'检测已关闭'};
  const label=checking?'正在检测':Object.hasOwn(labels,state)?labels[state]:'尚未检测';
  const measurable=observed&&enabled&&!checking&&['available','blocked','error'].includes(state);
  const latency=measurable&&Number.isFinite(data.latency_ms)?`${data.latency_ms.toFixed(0)} ms`:'—';
  const hasTrace=measurable&&state==='available';
  const scope='由助手经当前代理入口探测 ChatGPT 网页。若代理按进程分流，可能与 ChatGPT 会话路径不同；结果不保证账号或对话可用。';
  const checked=Number.isFinite(data.checked_at)?new Date(data.checked_at).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false}):'尚未检测';
  return <article className="metric-card connectivity-card"><div className="metric-label"><Symbol name="desktop"/>ChatGPT 网页连通性<Tooltip content={scope} relationship="description"><Button className="metric-help" appearance="transparent" size="small" aria-label="连通性探测范围说明" icon={<Symbol name="about"/>}/></Tooltip><StatusLabel color={checking?'subtle':state==='available'?'success':state==='blocked'?'warning':state==='error'?'danger':'subtle'}>{label}</StatusLabel></div><div className="connectivity-details"><div><span className="metric-detail">完整 HTTP 探测耗时</span><strong className="connectivity-latency">{latency}</strong></div><dl><div><dt>探测出口</dt><dd>{hasTrace&&typeof data.ip==='string'&&data.ip?data.ip:'—'}</dd></div><div><dt>国家/地区</dt><dd>{hasTrace&&typeof data.country==='string'&&data.country?data.country:'—'}</dd></div></dl></div><div className="connectivity-footer"><span>{checking&&!busy?'后台检测中':`上次检测：${checked}`}{measurable&&Number.isFinite(data.http_status)?` · HTTP ${data.http_status}`:''}</span><Button className="icon-action" appearance="subtle" aria-label="检测 ChatGPT 网页连通性" title="检测 ChatGPT 网页连通性" aria-busy={busy} disabled={!writable||!observed||!enabled||busy} icon={<Symbol name="refresh" spin={busy}/>} onClick={onCheck}/></div>{!checking&&state==='error'&&typeof data.error==='string'&&data.error&&<div className="metric-detail error-text">{data.error}</div>}</article>;
}

export function trafficAmount(value,rate=false){
  if(!Number.isFinite(value)||value<0)return '—';
  const units=rate?['B/s','KB/s','MB/s','GB/s']:['B','KB','MB','GB'];
  let amount=value,index=0;while(amount>=1024&&index<units.length-1){amount/=1024;index++;}
  return `${amount.toFixed(index||rate?1:0)} ${units[index]}`;
}
const trafficLabels={disabled:'未启用',starting:'正在启动',collecting:'采集中',needs_installation:'需要安装',needs_permission:'采集不可用',unavailable:'暂不可用',unknown:'等待统计状态'};
const serviceLabels={not_installed:'未安装',running:'运行中',stopped:'已停止',outdated:'版本不匹配',error:'状态异常',unknown:'待检测'};
const serviceReady=service=>['running','stopped'].includes(service?.state);
const trafficOperationKeys=['dashboard-enable-traffic','dashboard-disable-traffic','settings-enable-traffic','settings-disable-traffic'];
const trafficPending=busy=>trafficOperationKeys.some(key=>busy(key));
export function TrafficControls({state,service,trafficEnabled,observed,writable,busy,onEnable,onDisable,onSettings,idPrefix=''}){
  const enableBusy=busy('dashboard-enable-traffic'),disableBusy=busy('dashboard-disable-traffic');
  const related=trafficPending(busy)||busy('install-traffic-service')||busy('uninstall-traffic-service');
  const action=(id,label,loading,callback)=><Button id={idPrefix?`${idPrefix}-${id}`:id} className="text-action" appearance="subtle" aria-label={label} aria-busy={loading} disabled={!observed||!writable||related||typeof callback!=='function'} icon={loading?<Symbol name="refresh" spin/>:undefined} onClick={callback}>{label}</Button>;
  if(state==='needs_installation'||!serviceReady(service))return <div className="traffic-actions"><Button className="text-action" appearance="subtle" onClick={onSettings} disabled={typeof onSettings!=='function'}>前往设置</Button></div>;
  return <div className="traffic-actions">{((!trafficEnabled&&['disabled','unavailable'].includes(state))||enableBusy)&&action('enable-traffic','开启统计',enableBusy,onEnable)}{(trafficEnabled||['starting','collecting','needs_permission','unavailable'].includes(state)||disableBusy)&&action('disable-traffic','停止统计',disableBusy,onDisable)}</div>;
}
export function TrafficCard({result,service,trafficEnabled,observed,writable,busy,onEnable,onDisable,onSettings,idPrefix=''}){
  const data=result||{},state=observed?data.state||'disabled':'unknown';
  const collecting=state==='collecting'&&service?.state==='running';
  const label=!observed?'等待统计状态':state==='needs_installation'?'需要安装':service?.state==='not_installed'?'请先安装辅助服务':service?.state==='outdated'?'辅助服务版本不匹配':service?.state==='error'?'辅助服务状态异常':!serviceReady(service)?'等待辅助服务状态':state==='collecting'&&!collecting?'等待采集':Object.hasOwn(trafficLabels,state)?trafficLabels[state]:'暂不可用';
  return <article className="metric-card traffic-card"><div className="metric-label"><Symbol name="pulse"/>目标进程流量<StatusLabel color={collecting?'success':'subtle'}>{label}</StatusLabel><TrafficControls {...{state,service,trafficEnabled,observed,writable,busy,onEnable,onDisable,onSettings,idPrefix}}/></div><div className="traffic-values"><div><span className="metric-detail">下载</span><strong>{collecting?trafficAmount(data.receive_bps,true):'—'}</strong><span className="metric-detail">累计 {collecting?trafficAmount(data.received_bytes):'—'}</span></div><div><span className="metric-detail">上传</span><strong>{collecting?trafficAmount(data.send_bps,true):'—'}</strong><span className="metric-detail">累计 {collecting?trafficAmount(data.sent_bytes):'—'}</span></div></div><div className="metric-detail">已识别目标进程：{collecting&&Number.isFinite(data.target_count)?data.target_count:'—'} · 从开启统计起累计</div>{!collecting&&typeof data.message==='string'&&data.message&&<div className="metric-detail traffic-notice">{data.message}</div>}{collecting&&(data.lost_events>0||data.lost_buffers>0)&&<div className="metric-detail traffic-notice warning-text">采集有遗漏：{Number.isFinite(data.lost_events)?data.lost_events:'—'} 个事件 / {Number.isFinite(data.lost_buffers)?data.lost_buffers:'—'} 个缓冲区</div>}<div className="metric-detail traffic-scope">仅统计已识别的 ChatGPT / Codex CLI 进程，可能漏掉短命进程，不是网卡计费量。</div></article>;
}
export function CollectorSettings({result,service,trafficEnabled,observed,writable,busy,onEnable,onDisable,onInstall,onUninstall}){
  const data=service||{},state=observed?data.state||'error':'unknown';
  const installed=['running','stopped','outdated'].includes(state)||(state==='error'&&typeof data.version==='string'&&!!data.version);
  const installedVersion=installed&&typeof data.version==='string'&&data.version?data.version:null;
  const servicePending=busy('install-traffic-service')||busy('uninstall-traffic-service');
  const recordBusy=busy('settings-enable-traffic')||busy('settings-disable-traffic');
  const recordPending=trafficPending(busy);
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={!observed||!writable||servicePending||recordPending||typeof callback!=='function'} icon={busy(id)?<Symbol name="refresh" spin/>:undefined} onClick={callback}>{label}</Button>;
  const recordDescription=!observed?'等待辅助服务状态':state==='not_installed'?'请先安装辅助服务。':state==='outdated'?'辅助服务与当前应用不匹配，请先卸载后重新安装。':'辅助服务按需运行，正常启停统计不再反复授权。';
  return <div className="settings-group collector-settings"><Row icon="pulse" title="记录网络流量" description={recordDescription}><div className="switch-control">{recordBusy&&<Symbol name="refresh" spin/>}<Switch id="traffic-enabled" aria-label="记录网络流量" checked={!!trafficEnabled} aria-busy={recordBusy} disabled={!observed||!writable||!serviceReady(service)||servicePending||recordPending} onChange={(_,value)=>value.checked?onEnable?.():onDisable?.()}/></div></Row><Row icon="settings" title="流量辅助服务" description={<><StatusLabel color={state==='running'?'success':state==='outdated'?'warning':state==='error'?'danger':'subtle'}>{Object.hasOwn(serviceLabels,state)?serviceLabels[state]:'状态异常'}</StatusLabel>{installedVersion&&<span>版本 {installedVersion}</span>}</>}><div className="traffic-service-actions">{(state==='not_installed'||busy('install-traffic-service'))&&action('install-traffic-service','安装辅助服务',onInstall)}{(installed||busy('uninstall-traffic-service'))&&action('uninstall-traffic-service','卸载辅助服务',onUninstall)}</div></Row><div className="collector-copy"><p>安装或卸载辅助服务时请求 Windows 授权；主界面始终保持普通权限。</p><p>只统计目标进程 ID 的字节，不抓取内容或保留连接 IP、端口，无遥测。可能漏掉短命进程，不是网卡计费量。</p>{typeof data.error==='string'&&data.error&&<p className="warning-text">{data.error}</p>}{typeof result?.message==='string'&&result.message&&<p className="warning-text">{result.message}</p>}</div></div>;
}
export function App({initialStatus=null,initialPage='dashboard',initialSettingsTab='general'}={}){
  const [page,setPage]=useState(initialPage==='about'?'settings':Object.hasOwn(pages,initialPage)?initialPage:'dashboard');
  const [settingsTab,setSettingsTab]=useState(initialPage==='about'?'about':['general','about'].includes(initialSettingsTab)?initialSettingsTab:'general');
  const mainScroll=useRef(null);
  const [collapsed,setCollapsed]=useState(()=>{try{return localStorage.getItem('sidebar-collapsed')==='1';}catch{return false;}});
  const [themeMode,setThemeMode]=useState(readThemeMode),[systemDark,setSystemDark]=useState(prefersDark);
  const currentThemeMode=useRef(themeMode);currentThemeMode.current=themeMode;
  const resolvedTheme=resolveTheme(themeMode,systemDark);
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
  function navigate(target){if(target==='about'){setSettingsTab('about');setPage('settings');}else if(Object.hasOwn(pages,target)){setPage(target);}}
  useThemeLayoutEffect(()=>mainScroll.current?.scrollTo({top:0,left:0,behavior:'instant'}),[page,settingsTab]);
  useThemeLayoutEffect(()=>applyDocumentTheme(resolvedTheme),[resolvedTheme]);
  useEffect(()=>{
    let disposed=false;
    const media=globalThis.matchMedia?.(systemThemeQuery);
    const failed=()=>{if(!disposed)show('窗口主题同步失败。','warning');};
    const changed=event=>{if(disposed)return;setSystemDark(event.matches);if(currentThemeMode.current==='system'){applyDocumentTheme(resolveTheme('system',event.matches));syncNativeAppearance('system').catch(failed);}};
    media?.addEventListener('change',changed);
    const ready=async()=>{
      try{await syncNativeAppearance(currentThemeMode.current);}catch{failed();}
      finally{if(appWindow&&invoke){try{const requested=await invoke('frontend_ready');if(!disposed)navigate(requested);}catch{if(!disposed)show('窗口初始化失败，请重新打开助手。','warning');}}}
    };
    ready();
    return()=>{disposed=true;media?.removeEventListener('change',changed);};
  },[]);
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
    if(native?.event?.listen)registered(native.event.listen('navigate',event=>navigate(event.payload)));
    if(appWindow){const sync=()=>appWindow.isMaximized().then(value=>{if(!disposed)setMaximized(value);}).catch(failed);sync();registered(appWindow.onResized(sync));}
    return()=>{disposed=true;for(const stop of stops)stop();};
  },[]);
  async function run(name,command,args,success){await actions.current.run(name,async()=>{try{const data=await invoke(command,args);if(data)accept(data);if(success)show(success,'success');}catch(error){show(errorText(error),'error');}});}
  async function saveProxy(event){
    event.preventDefault();if(!writable||busy('toggle'))return;const submitted=proxyEdit.current.version;
    await actions.current.run('save',async()=>{try{const data=await invoke('save_proxy',{proxyUrl:proxy.trim()});if(submitted===proxyEdit.current.version)proxyEdit.current.dirty=false;const next=accept(data);show(proxyEdit.current.dirty?'已应用；当前输入还有未保存的更改。':next.restart_required?'已应用，重开 ChatGPT 后生效。':'代理设置已应用。',proxyEdit.current.dirty?'info':'success');}catch(error){show(errorText(error),'error');}});
  }
  async function changeSystem(key,value){
    const name={autostart:'autostart',launchTarget:'launch-target',cliWorkdir:'cli-workdir',networkEnabled:'network-enabled',autoCheckUpdates:'auto-check-updates',updateIntervalHours:'update-interval',connectivityEnabled:'connectivity-enabled'}[key];if(!name||!writable||busy(name))return;
    systemPending.current.add(key);setSystem(previous=>({...previous,[key]:value}));
    await actions.current.run(name,async()=>{try{const data=await invoke('save_system',{autostart:null,launchTarget:null,cliWorkdir:null,networkEnabled:null,autoCheckUpdates:null,updateIntervalHours:null,connectivityEnabled:null,[key]:value});systemPending.current.delete(key);accept(data);}catch(error){systemPending.current.delete(key);setSystem(previous=>systemPreferences(previous,current.current||{},systemPending.current));show(errorText(error),'error');}});
  }
  async function saveDirectory(event){event?.preventDefault();if(!directoryEdit.current.dirty||busy('cli-workdir'))return;const submitted=directoryEdit.current.version;await changeSystem('cliWorkdir',directory.trim());if(submitted===directoryEdit.current.version&&current.current?.cli_workdir===directory.trim()){directoryEdit.current.dirty=false;setDirectory(current.current.cli_workdir);}}
  async function checkUpdates(){await actions.current.run('check-updates',async()=>{try{await invoke('check_updates');accept(await invoke('get_status'));}catch(error){try{accept(await invoke('get_status'));}catch{}show(errorText(error),'error');}});}
  async function checkConnectivity(){await actions.current.run('check-connectivity',async()=>{try{await invoke('check_connectivity');accept(await invoke('get_status'));}catch(error){try{accept(await invoke('get_status'));}catch{}show(errorText(error),'error');}});}
  async function trafficAction(name,command,args){if(trafficPending(busy))return;await actions.current.run(name,async()=>{try{await invoke(command,args);accept(await invoke('get_status'));}catch(error){try{accept(await invoke('get_status'));}catch{}show(errorText(error),'error');}});}
  async function windowAction(action){try{await appWindow?.[action]();}catch(error){show(errorText(error),'error');}}
  async function changeTheme(mode){
    if(!themeModes.includes(mode)||busy('appearance'))return;
    currentThemeMode.current=mode;setThemeMode(mode);applyDocumentTheme(resolveTheme(mode,systemDark));try{localStorage.setItem(themeKey,mode);}catch{}
    await actions.current.run('appearance',async()=>{try{await syncNativeAppearance(mode);}catch{show('窗口主题同步失败。','warning');}});
  }
  function fold(){setCollapsed(value=>{try{localStorage.setItem('sidebar-collapsed',value?'0':'1');}catch{}return !value;});}
  const button=(id,text,icon,onClick,{primary=false,disabled=false,type='button',iconOnly=false}={})=><Button id={id} type={type} className={iconOnly?'icon-action':'text-action'} appearance={primary?'primary':'subtle'} icon={busy(id)?<Symbol name="refresh" spin/>:icon?<Symbol name={icon}/>:undefined} aria-label={iconOnly?text:undefined} title={iconOnly?text:undefined} aria-busy={busy(id)} disabled={!invoke||busy(id)||disabled} onClick={onClick}>{iconOnly?null:text}</Button>;
  const ready=status?.enabled&&(status?.guard_ready||status?.cli_ready)&&status?.proxy_reachable&&!status?.error;
  const guardText=!status?'正在连接后台':!online?'后台未连接':!status.backend_ready?'助手未启动':!status.enabled?'守护已暂停':status.error?'守护需要检查':ready?'守护已就绪':!status.proxy_reachable?'等待本地代理':'等待应用接入';
  const logs=status?.logs.slice().reverse()||[],filtered=logs.filter(log=>logFilter==='all'||log.level===logFilter);
  const samples=status?.probe_history||[],path=probePath(samples);
  const aboutStatus={...(status||{}),network_enabled:system.networkEnabled,auto_check_updates:system.autoCheckUpdates,update_interval_hours:system.updateIntervalHours};

  return <FluentProvider theme={themes[resolvedTheme]} data-theme={resolvedTheme} className={`app-shell ${collapsed?'sidebar-collapsed':''}`}>
    <div className="window-drag" data-tauri-drag-region/>
    <div className="window-controls" aria-label="窗口控制"><Button appearance="transparent" aria-label="最小化" title="最小化" disabled={!appWindow} icon={<Symbol name="minimize"/>} onClick={()=>windowAction('minimize')}/><Button appearance="transparent" aria-label={maximized?'还原':'最大化'} title={maximized?'还原':'最大化'} disabled={!appWindow} icon={<Symbol name={maximized?'restore':'maximize'}/>} onClick={()=>windowAction('toggleMaximize')}/><Button appearance="transparent" className="window-close" aria-label="关闭到托盘" title="关闭到托盘" disabled={!appWindow} icon={<Symbol name="dismiss"/>} onClick={()=>windowAction('close')}/></div>
    <aside className="sidebar">
      <div className="brand" data-tauri-drag-region><img src="icon.svg" alt="" data-tauri-drag-region/>{!collapsed&&<strong data-tauri-drag-region>CodexProxyTray</strong>}<Button appearance="subtle" className="sidebar-toggle" aria-label={collapsed?'展开侧边栏':'收起侧边栏'} title={collapsed?'展开侧边栏':'收起侧边栏'} icon={<Symbol name={collapsed?'expand':'collapse'}/>} onClick={fold}/></div>
      <nav aria-label="页面导航">{Object.entries(pages).map(([key,label])=><Button key={key} data-page={key} appearance="subtle" icon={<Symbol name={key}/>} className={`nav-item ${page===key?'selected':''}`} aria-current={page===key?'page':undefined} aria-label={label} title={collapsed?label:undefined} onClick={()=>setPage(key)}>{!collapsed&&label}</Button>)}</nav>
      <div className="sidebar-footer" title={status?.read_only?'只读预览':online?'后台运行中':'后台未连接'}><span className={`status-indicator ${online&&status?.backend_ready?'active':''}`}/>{!collapsed&&<><span>{status?.read_only?'只读预览':online?'后台运行中':'后台未连接'}</span><span className="version">v{version}</span></>}</div>
    </aside>
    <main className="main-scroll" ref={mainScroll}><div className="main-content">
      <header className="page-header" data-tauri-drag-region><h1 data-tauri-drag-region>{pages[page]}</h1><div className="header-actions">{['dashboard','proxy'].includes(page)&&button('refresh','重新检测','refresh',()=>run('refresh','refresh_status'),{iconOnly:true})}{page==='logs'&&button('refresh-logs','刷新日志','refresh',()=>run('refresh-logs','get_status'),{iconOnly:true})}</div></header>
      {notice&&<MessageBar intent={notice.intent} icon={<Symbol name={notice.intent==='success'?'success':'warning'}/>}><MessageBarBody>{notice.text}</MessageBarBody><MessageBarActions containerAction={<Button appearance="transparent" size="small" aria-label="关闭提示" icon={<Symbol name="dismiss"/>} onClick={()=>setNotice(null)}/>}/></MessageBar>}
      {!invoke&&<MessageBar intent="warning" icon={<Symbol name="warning"/>}><MessageBarBody>请运行 CodexProxyTray 桌面程序。</MessageBarBody></MessageBar>}
      {status?.read_only&&<div className="preview-note">本机真实状态 · 修改设置请使用桌面窗口</div>}
      <section hidden={page!=='dashboard'} aria-label="仪表盘">
        <div className="dashboard-grid">
          <article className="metric-card probe-card"><div className="metric-label"><Symbol name="chart"/>本地代理连接耗时<span className="live-caption">{samples.length?`${samples.length} 次${online?'':'历史'}采样`:'等待采样'}</span></div><div className="probe-value">{observed&&Number.isFinite(status?.proxy_latency_ms)?status.proxy_latency_ms.toFixed(2):'—'}<span>ms</span><StatusLabel color={observed&&status?.proxy_reachable?'success':'subtle'}>{!observed?'待检测':status.proxy_reachable?'可连接':'未连接'}</StatusLabel></div>{path?<svg viewBox="0 0 600 150" preserveAspectRatio="none" role="img" aria-label="本地代理 TCP 连接耗时趋势"><path d={path}/></svg>:<div className="chart-empty">等待本地代理采样</div>}<div className="chart-caption"><span>{samples.length?time(samples[0].at):'—'}</span><code>{status?.proxy_url||'—'}</code><span>{samples.length?time(samples.at(-1).at):'—'}</span></div></article>
          <article className="metric-card guard-card"><div className="metric-label"><Symbol name="shield"/>代理守护</div><strong className="guard-value">{guardText}</strong><div className="guard-targets"><span>ChatGPT</span><StatusLabel color={observed&&status?.guard_ready?'success':'subtle'}>{!observed?'待检测':status.guard_ready?'已就绪':'未接入'}</StatusLabel><span>Codex CLI</span><StatusLabel color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</StatusLabel></div></article>
          <Metric icon="desktop" label="ChatGPT" value={!observed?'—':status.codex_running?'运行中':'未运行'} detail={!observed?'等待应用状态':status.desktop_pid?`PID ${status.desktop_pid}`:status.package_version?`已安装 ${status.package_version}`:'尚未检测到应用'}/>
          <Metric icon="pulse" label="ChatGPT TCP 连接" value={observed&&Number.isFinite(status?.tcp_connections)?status.tcp_connections:'—'} detail="应用进程树中的已建立连接"/>
          <Metric icon="clock" label="助手运行时长" value={observed&&status?.started_at?duration(status.updated_at-status.started_at):'—'} detail={observed&&Number.isFinite(status?.helper_memory_bytes)?`守护进程工作集 ${(status.helper_memory_bytes/1048576).toFixed(1)} MB`:'守护进程资源采样中'}/>
          <ConnectivityCard result={status?.connectivity} observed={observed} writable={writable} enabled={system.networkEnabled&&system.connectivityEnabled} busy={busy('check-connectivity')} onCheck={checkConnectivity}/>
          <TrafficCard idPrefix="dashboard" result={status?.traffic} service={status?.traffic_service} trafficEnabled={!!status?.traffic_enabled} observed={observed} writable={writable} busy={busy} onEnable={()=>trafficAction('dashboard-enable-traffic','enable_traffic')} onDisable={()=>trafficAction('dashboard-disable-traffic','disable_traffic')} onSettings={()=>setPage('settings')}/>
        </div>
        {status?.error&&<div className="inline-note error-text"><Symbol name="warning"/>{status.error}</div>}
        {status?.restart_required&&status.enabled&&<div className="inline-note restart-hint"><Symbol name="about"/><span>当前 ChatGPT 需完整退出并重开一次。</span></div>}
        <div className="section-heading"><h2>最近事件</h2><Button appearance="subtle" size="small" onClick={()=>setPage('logs')}>全部日志</Button></div><EventList logs={logs.slice(0,4)} compact/>
      </section>
      <section hidden={page!=='proxy'} aria-label="代理">
        <form onSubmit={saveProxy} className="settings-group"><Row icon="proxy" title="本地 HTTP 代理" description={<><span className={`status-indicator ${observed&&status?.proxy_reachable?'active':''}`}/>{!observed?'待检测':status.proxy_reachable?'端口可用':'暂未连接'}</>} className="proxy-row"><Input id="proxy-url" aria-label="本地 HTTP 代理地址" value={proxy} readOnly={!writable} spellCheck={false} onChange={(_,data)=>{proxyEdit.current.dirty=true;proxyEdit.current.version++;setProxy(data.value);}}/>{button('save','应用',null,undefined,{type:'submit',primary:true,disabled:!writable||busy('toggle')})}</Row></form>
        <h2>应用接入</h2><div className="settings-group targets"><Row icon="desktop" title="ChatGPT" description={!observed?'等待应用状态':status.package_count?`版本 ${status.package_version||'已安装'}`:'未检测到桌面应用'}><StatusLabel color={observed&&status?.codex_running?'success':'subtle'}>{!observed?'待检测':status.codex_running?'运行中':'未运行'}</StatusLabel>{button('launch','打开 ChatGPT','open',()=>run('launch','launch_codex'),{disabled:!writable||!status?.package_count,iconOnly:true})}</Row><Row icon="terminal" title="Codex CLI" description={!observed?'等待 CLI 状态':status.cli_ready?'通过专用入口在终端中运行。':status.cli_detected?'已找到 Windows CLI':'未检测到 Windows CLI'}><StatusLabel color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</StatusLabel>{button('launch-cli','打开 Codex CLI','open',()=>run('launch-cli','launch_cli'),{disabled:!writable||!status?.cli_ready,iconOnly:true})}</Row></div>
      </section>
      <section hidden={page!=='settings'} aria-label="设置"><TabList className="settings-tabs" appearance="transparent" aria-label="设置分类" selectedValue={settingsTab} onTabSelect={(_,data)=>{if(['general','about'].includes(data.value))setSettingsTab(data.value);}}><Tab id="settings-tab-general" className="settings-tab" value="general" aria-controls="settings-panel-general">常规</Tab><Tab id="settings-tab-about" className="settings-tab" value="about" aria-controls="settings-panel-about">关于</Tab></TabList><article className="settings-panel" id="settings-panel-general" role="tabpanel" aria-labelledby="settings-tab-general" hidden={settingsTab!=='general'}><h2 className="first-section">外观</h2><div className="settings-group"><Row icon="settings" title="主题" description="跟随系统，或固定使用浅色、深色。"><div className="switch-control">{busy('appearance')&&<Symbol name="refresh" spin/>}<Select id="color-theme" aria-label="界面主题" aria-busy={busy('appearance')} value={themeMode} disabled={busy('appearance')} icon={<Icon icon={icons['chevron-down-24-regular']} width={16} height={16} aria-hidden="true"/>} onChange={(_,data)=>changeTheme(data.value)}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></Select></div></Row></div><h2>启动</h2><div className="settings-group"><Row icon="power" title="登录时自动启动" description="登录 Windows 后在托盘运行。"><div className="switch-control">{busy('autostart')&&<Symbol name="refresh" spin/>}<Switch id="autostart" aria-label="登录时自动启动" aria-busy={busy('autostart')} checked={system.autostart} disabled={!writable||busy('autostart')} onChange={(_,data)=>changeSystem('autostart',data.checked)}/></div></Row><Row icon="desktop" title="启动后打开" description="助手启动时自动打开所选应用。"><div className="switch-control">{busy('launch-target')&&<Symbol name="refresh" spin/>}<Select id="launch-target" aria-label="启动后打开" aria-busy={busy('launch-target')} value={system.launchTarget} icon={<Icon icon={icons['chevron-down-24-regular']} width={16} height={16} aria-hidden="true"/>} disabled={!writable||busy('launch-target')} onChange={(_,data)=>changeSystem('launchTarget',data.value)}><option value="none">不打开应用</option><option value="chatgpt">ChatGPT</option><option value="cli">Codex CLI</option><option value="both">ChatGPT 与 Codex CLI</option></Select></div></Row></div><h2>检测</h2><div className="settings-group"><Row icon="desktop" title="ChatGPT 连通检测" description="经当前代理入口探测网页，不验证账号或对话。"><div className="switch-control">{busy('connectivity-enabled')&&<Symbol name="refresh" spin/>}<Switch id="connectivity-enabled" aria-label="ChatGPT 连通检测" aria-busy={busy('connectivity-enabled')} checked={system.connectivityEnabled} disabled={!writable||!system.networkEnabled||busy('connectivity-enabled')} onChange={(_,data)=>changeSystem('connectivityEnabled',data.checked)}/></div></Row></div><h2>网络流量</h2><CollectorSettings result={status?.traffic} service={status?.traffic_service} trafficEnabled={!!status?.traffic_enabled} observed={observed} writable={writable} busy={busy} onEnable={()=>trafficAction('settings-enable-traffic','enable_traffic')} onDisable={()=>trafficAction('settings-disable-traffic','disable_traffic')} onInstall={()=>trafficAction('install-traffic-service','install_traffic_service')} onUninstall={()=>trafficAction('uninstall-traffic-service','uninstall_traffic_service')}/><h2>Codex CLI</h2><form className="settings-group" onSubmit={saveDirectory}><Row icon="terminal" title="工作目录" description="留空使用用户目录。离开输入框时自动保存。" className="workdir-row"><Input id="cli-workdir" aria-label="Codex CLI 工作目录" value={directory} placeholder="使用用户目录" aria-busy={busy('cli-workdir')} readOnly={!writable||busy('cli-workdir')} spellCheck={false} onChange={(_,data)=>{directoryEdit.current.dirty=true;directoryEdit.current.version++;setDirectory(data.value);}} onBlur={saveDirectory}/>{busy('cli-workdir')&&<Symbol name="refresh" spin/>}</Row></form></article><article className="settings-panel" id="settings-panel-about" role="tabpanel" aria-labelledby="settings-tab-about" hidden={settingsTab!=='about'}><About status={aboutStatus} version={version} busy={busy} writable={writable} onPreference={changeSystem} onCheck={checkUpdates} onSource={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/CodexProxyTray','_blank','noopener,noreferrer');return;}run('open-source','open_source');}} onRelease={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/chatgpt-proxy-launcher/releases','_blank','noopener,noreferrer');return;}run('open-release','open_release');}} onLogs={writable?()=>run('open-logs','open_logs'):undefined}/></article></section>
      <section hidden={page!=='logs'} aria-label="运行日志"><div className="log-toolbar"><TabList className="log-filters" appearance="transparent" size="small" aria-label="日志级别" selectedValue={logFilter} onTabSelect={(_,data)=>setLogFilter(data.value)}>{[['all','全部'],['warning','警告'],['error','错误']].map(([key,label])=><Tab key={key} id={`log-filter-${key}`} className="log-filter-tab" value={key} aria-controls="log-results">{label}</Tab>)}</TabList><span>保留最近 200 条</span></div>{status?.log_storage_error&&<div className="inline-note error-text">历史日志保存异常：{status.log_storage_error}</div>}<div id="log-results" role="tabpanel" aria-labelledby={`log-filter-${logFilter}`}><EventList logs={filtered}/></div></section>

    </div></main>
    {page==='dashboard'&&<GuardButton status={status} observed={observed} writable={writable&&!busy('save')} busy={busy('toggle')} onToggle={()=>run('toggle','set_enabled',{enabled:!status?.enabled})}/>}
  </FluentProvider>;
}
if(typeof document!=='undefined')createRoot(document.getElementById('root')).render(<App/>);
