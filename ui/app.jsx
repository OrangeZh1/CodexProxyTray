import React, {useEffect,useLayoutEffect,useRef,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {FluentProvider,Button,Input,Switch,MessageBar,MessageBarBody,MessageBarActions,TabList,Tab,Tooltip,Accordion,AccordionItem,AccordionHeader,AccordionPanel} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {createActions,acceptSnapshot,systemPreferences} from './actions.mjs';
import {duration,probePath,probeScale,appendTraffic,trafficWindow,trafficPath,trafficAge} from './metrics.mjs';
import {About,UpdateSettings,Disclaimer,PrivacyDetails} from './about.jsx';
import {ConfirmAction,CliManagement,RuntimeDrawer,NoticeToasts} from './management.jsx';
import {UsageView} from './usage-view.jsx';
import {SidebarToggleIcon} from './navigation-icon.jsx';
import {BrandIcon} from './brand-icons.jsx';
import {CountryFlag} from './country-flag.jsx';
import {StatusLabel} from './status-label.jsx';
import {Choice} from './choice.jsx';
import {LogFilters} from './log-filters.jsx';
import {EventList} from './log-list.jsx';
import {FlowLogs} from './flow-logs.jsx';
import {filterLogs,logDate,logDateRange} from './logs.mjs';
import {themes,themeKey,themeModes,systemThemeQuery,readThemeMode,resolveTheme,prefersDark,applyDocumentTheme} from './themes.mjs';
import './style.css';

const native=typeof window==='undefined'?null:window.__TAURI__;
const invoke=native?.core?.invoke;
const appWindow=native?.window?.getCurrentWindow();
const version=__APP_VERSION__;
const settingsPages={home:['设置','settings',''],about:['关于','about','产品介绍、许可与数据隐私'],privacy:['本机数据与隐私','shield',''],disclaimer:['免责声明','shield','']};
const pages={dashboard:'仪表盘',proxy:'代理',usage:'用量统计',logs:'运行日志',settings:'设置'};
const names={dashboard:'grid-24-regular',settings:'settings-24-regular',usage:'data-usage-24-regular',logs:'text-bullet-list-ltr-24-regular',about:'info-24-regular',shield:'shield-checkmark-24-regular',proxy:'plug-connected-24-regular',desktop:'desktop-24-regular',terminal:'window-console-20-regular',power:'power-24-regular',open:'open-24-regular',refresh:'arrow-sync-24-regular',save:'save-24-regular',success:'checkmark-circle-24-regular',warning:'warning-24-regular',dismiss:'dismiss-24-regular',minimize:'subtract-24-regular',maximize:'maximize-24-regular',restore:'square-multiple-24-regular',collapse:'panel-left-contract-24-regular',expand:'panel-left-expand-24-regular',pulse:'pulse-24-regular',clock:'clock-24-regular',memory:'ram-16-regular',chart:'data-line-24-regular',latency:'timer-24-regular',pause:'pause-24-regular',play:'play-24-regular',pin:'pin-24-regular',pinned:'pin-24-filled'};
export function Symbol({name,spin=false}){if(name==='chatgpt')return <BrandIcon name="chatgpt"/>;return <Icon icon={icons[names[name]]} className={spin?'symbol spin':'symbol'} aria-hidden="true"/>;}
const useThemeLayoutEffect=typeof document==='undefined'?useEffect:useLayoutEffect;
function useInputModality(){useEffect(()=>{const root=document.documentElement;root.dataset.inputModality='pointer';const pointer=()=>{root.dataset.inputModality='pointer';},keyboard=event=>{if(['Tab','Enter',' ','ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End'].includes(event.key))root.dataset.inputModality='keyboard';};document.addEventListener('pointerdown',pointer,true);document.addEventListener('keydown',keyboard,true);return()=>{document.removeEventListener('pointerdown',pointer,true);document.removeEventListener('keydown',keyboard,true);};},[]);}
async function syncNativeAppearance(mode){
  if(!appWindow)return;
  await appWindow.setTheme?.(mode==='system'?null:mode);
}
function renderGeometry(){
  const root=document.getElementById('root'),shell=root?.querySelector('.app-shell,.tray-surface');
  const describe=node=>{if(!node)return null;const rect=node.getBoundingClientRect(),style=getComputedStyle(node);return{tag:node.nodeName,class:node.getAttribute('class'),id:node.id,role:node.getAttribute('role'),x:rect.x,y:rect.y,width:rect.width,height:rect.height,scroll_height:node.scrollHeight,display:style.display,visibility:style.visibility,opacity:style.opacity,position:style.position,z_index:style.zIndex,transform:style.transform,content_visibility:style.contentVisibility,background:style.backgroundColor,color:style.color,grid_columns:style.gridTemplateColumns,grid_rows:style.gridTemplateRows};};
  return{viewport_width:innerWidth,viewport_height:innerHeight,root:describe(root),shell:describe(shell),tray_height:Math.ceil((shell?.scrollHeight||0)+14),nodes:['.sidebar','.brand','.sidebar nav','.main-scroll','.main-content','.page-header','h1','.window-controls','.dashboard-grid','.metric-card','.guard-floating','.traffic-axis','.traffic-plot','.traffic-time-axis','.traffic-chart-card','.traffic-chart-card .probe-chart','.traffic-time-axis > span:first-child','.traffic-time-axis > span:last-child'].map(selector=>({selector,...describe(root?.querySelector(selector))})),body_children:[...document.body.children].slice(0,8).map(describe),layers:[[100,100],[innerWidth/2,innerHeight/2],[innerWidth-40,20]].map(([x,y])=>({x,y,nodes:(document.elementsFromPoint?.(x,y)||[]).slice(0,5).map(describe)}))};
}
const errorText=error=>error?.name==='AbortError'?'操作已取消。':String(error?.message||error||'操作失败，请重试。');
const time=at=>new Date(at).toLocaleTimeString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false});
function Row({icon,title,description,children,className=''}){return <div className={`setting-row ${className}`}><Symbol name={icon}/><div className="row-copy"><div className="row-title">{title}</div>{description&&<div className="row-description">{description}</div>}</div><div className="row-control">{children}</div></div>;}
export function Metric({icon,label,value,detail,className=''}){return <article className={`metric-card ${className}`}><div className="metric-label"><Symbol name={icon}/>{label}</div><strong className="metric-value">{value}</strong><div className="metric-detail">{detail}</div></article>;}

export function GuardButton({status,observed,writable,busy,onToggle}){
  const [now,setNow]=useState(()=>Date.now());
  // Retain text for the exit fade; the paused square clips and hides the clock.
  const renderedTime=useRef('—');
  const [pendingVisible,setPendingVisible]=useState(false);
  const pendingShownAt=useRef(0);
  const enabled=!!status?.enabled;
  useEffect(()=>{
    const timer=busy?setTimeout(()=>{pendingShownAt.current=performance.now();setPendingVisible(true);},180):setTimeout(()=>setPendingVisible(false),Math.max(0,120-(performance.now()-pendingShownAt.current)));
    return()=>clearTimeout(timer);
  },[busy]);
  useEffect(()=>{
    if(!observed||!enabled)return;
    setNow(Date.now());const timer=setInterval(()=>setNow(Date.now()),1000);
    return()=>clearInterval(timer);
  },[observed,enabled]);
  const known=observed&&Number.isFinite(status?.guard_uptime_ms);
  const milliseconds=known?status.guard_uptime_ms+(enabled?Math.max(0,now-(status.updated_at||now)):0):null;
  if(enabled)renderedTime.current=known?duration(milliseconds):'—';
  const label=enabled?'暂停守护':'恢复守护';
  const pending=busy||pendingVisible;
  return <Button id="toggle" className={`guard-floating ${enabled?'guard-running':'guard-paused'} ${observed?'':'guard-unknown'}`} appearance="primary" aria-label={label} title={label} aria-busy={pending} aria-disabled={pending||undefined} disabled={!observed||!writable} onClick={()=>{if(!pending)onToggle?.();}}><Symbol name={pendingVisible?'refresh':enabled?'pause':'play'} spin={pendingVisible}/><span className="guard-elapsed" aria-hidden={!enabled}>{renderedTime.current}</span></Button>;
}

export function ConnectivityCard({result,observed,writable,enabled,busy,onCheck,onOpen}){
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
  return <article className="metric-card connectivity-card">{onOpen&&<button type="button" className="card-hitarea" aria-label="查看网页连通性详情" aria-haspopup="dialog" onClick={onOpen}/>}<div className="metric-label"><Symbol name="desktop"/>ChatGPT 网页连通性<Tooltip content={scope} relationship="description"><Button className="metric-help" appearance="transparent" size="small" aria-label="连通性探测范围说明" icon={<Symbol name="about"/>}/></Tooltip><StatusLabel color={checking?'subtle':state==='available'?'success':state==='blocked'?'warning':state==='error'?'danger':'subtle'}>{label}</StatusLabel>{onOpen&&<Icon className="card-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/>}</div><div className="connectivity-details"><div><span className="metric-detail">完整 HTTP 探测耗时</span><strong className="connectivity-latency">{latency}</strong></div><dl><div><dt>探测出口</dt><dd>{hasTrace&&typeof data.ip==='string'&&data.ip?data.ip:'—'}</dd></div><div><dt>国家/地区</dt><dd>{hasTrace?<CountryFlag code={data.country}/>:'—'}</dd></div></dl></div><div className="connectivity-footer"><span>{checking&&!busy?'后台检测中':`上次检测：${checked}`}{measurable&&Number.isFinite(data.http_status)?` · HTTP ${data.http_status}`:''}</span></div>{!checking&&state==='error'&&typeof data.error==='string'&&data.error&&<div className="metric-detail error-text">{data.error}</div>}</article>;
}

export function trafficAmount(value,rate=false){
  if(!Number.isFinite(value)||value<0)return '—';
  const units=rate?['B/s','KB/s','MB/s','GB/s']:['B','KB','MB','GB'];
  let amount=value,index=0;while(amount>=1024&&index<units.length-1){amount/=1024;index++;}
  return `${amount.toFixed(index||rate?1:0)} ${units[index]}`;
}
const trafficLabels={disabled:'未启用',starting:'正在启动',collecting:'采集中',needs_installation:'未开启',needs_permission:'暂不可用',unavailable:'暂不可用',unknown:'等待统计状态'};
const serviceLabels={not_installed:'未安装',running:'运行中',stopped:'已停止',outdated:'需要更新服务',error:'状态异常',unknown:'待检测'};
const serviceReady=service=>['running','stopped'].includes(service?.state);
const trafficOperationKeys=['settings-enable-traffic','settings-disable-traffic'];
const trafficPending=busy=>trafficOperationKeys.some(key=>busy(key));
export function TrafficCard({result,service,trafficEnabled,guardEnabled,observed,history=[]}){
  const [hoverAt,setHoverAt]=useState(null);
  const data=result||{},collecting=observed&&trafficEnabled&&guardEnabled&&data.state==='collecting'&&service?.state==='running';
  const paused=observed&&trafficEnabled&&!guardEnabled&&data.state==='disabled';
  const label=!observed?'等待统计状态':!trafficEnabled?'未开启':paused?'已随守护暂停':!serviceReady(service)||data.state==='unavailable'?'暂不可用':!guardEnabled?'正在暂停采集':data.state==='disabled'?'正在恢复采集':data.state==='collecting'&&!collecting?'暂不可用':collecting&&data.partial?'采集中 · 累计不完整':trafficLabels[data.state]||'暂不可用';
  const window=trafficWindow(history),{first,span,points,measured}=window;
  const values=points.flatMap(point=>[point.receive_bps,point.send_bps]).filter(value=>Number.isFinite(value)&&value>=0);
  const scale=2**Math.ceil(Math.log2(Math.max(1024,...values)*1.15));
  const tick=value=>new Intl.NumberFormat('zh-CN',{maximumFractionDigits:1}).format(value/(scale>=1048576?1048576:scale>=1024?1024:1)),unit=scale>=1048576?'MB/s':scale>=1024?'KB/s':'B/s';
  const down=trafficPath(window,'receive_bps',scale),up=trafficPath(window,'send_bps',scale);
  const active=collecting&&span>0?measured.find(point=>point.at===hoverAt):null,activeX=active?(active.at-first)/span*100:0;
  const move=event=>{const rect=event.currentTarget.getBoundingClientRect();if(!rect.width)return;const at=first+Math.max(0,Math.min(1,(event.clientX-rect.left)/rect.width))*span;const nearest=measured.reduce((best,point)=>!best||Math.abs(point.at-at)<Math.abs(best.at-at)?point:best,null);setHoverAt(nearest&&Math.abs(nearest.at-at)<=5000?nearest.at:null);};
  const keyMove=event=>{if(event.key==='Escape'){setHoverAt(null);return;}if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)||!measured.length)return;event.preventDefault();const current=measured.findIndex(point=>point.at===hoverAt),index=event.key==='Home'?0:event.key==='End'?measured.length-1:current<0?measured.length-1:Math.max(0,Math.min(measured.length-1,current+(event.key==='ArrowLeft'?-1:1)));setHoverAt(measured[index].at);};
  const isolated=(point,index,key)=>Number.isFinite(point[key])&&point[key]>=0&&![points[index-1],points[index+1]].some(other=>other&&Number.isFinite(other[key])&&other[key]>=0&&Math.abs(other.at-point.at)<=10_000);
  const reason=!trafficEnabled?'在“设置 → 流量统计”开启；需先安装辅助服务。':paused?'恢复守护后，将按流量统计设置继续采集。':label==='正在暂停采集'?'正在停止采集，等待辅助服务确认。':label==='正在恢复采集'?'等待辅助服务恢复有效采样。':collecting&&measured.length===1?'等待第二个有效样本。':data.message||'等待有效的目标进程采样。';
  return <article className="metric-card traffic-chart-card" style={{'--traffic-axis-width':`${String(tick(scale)).length+unit.length+1}ch`}} aria-label="目标进程流量"><div className="metric-label"><Symbol name="pulse"/>网络流量<Tooltip content="仅限已识别的 ChatGPT / Codex CLI，自采集会话启动起累计；可能遗漏短命进程，不能当作网卡计费量。" relationship="description"><Button className="metric-help" appearance="transparent" aria-label="流量统计范围" icon={<Symbol name="about"/>}/></Tooltip><StatusLabel color={collecting?(data.partial?'warning':'success'):'subtle'}>{label}</StatusLabel></div><div className="traffic-rates"><span className="traffic-upload"><i aria-hidden="true"/>上传 <strong>{collecting?trafficAmount(data.send_bps,true):'—'}</strong></span><span className="traffic-download"><i aria-hidden="true"/>下载 <strong>{collecting?trafficAmount(data.receive_bps,true):'—'}</strong></span></div>
    <div className="probe-chart"><div className="probe-axis traffic-axis" aria-hidden="true"><span>{tick(scale)} {unit}</span><span>{tick(scale/2)}</span><span>0</span></div>{collecting&&(down||up)?<div className="traffic-plot" onPointerLeave={()=>setHoverAt(null)}><svg viewBox="0 0 600 80" preserveAspectRatio="none" role="img" tabIndex="0" aria-label="目标进程上传与下载速率趋势" aria-describedby={active?'traffic-hover-tooltip':undefined} onPointerMove={move} onFocus={()=>{if(document.documentElement.dataset.inputModality==='keyboard')setHoverAt(measured.at(-1)?.at??null);}} onBlur={()=>setHoverAt(null)} onKeyDown={keyMove}><line x1="0" y1="2" x2="600" y2="2"/><line x1="0" y1="40" x2="600" y2="40"/><line x1="0" y1="78" x2="600" y2="78"/><path className="traffic-download-line" d={down}/><path className="traffic-upload-line" d={up}/>{[['receive_bps','download'],['send_bps','upload']].flatMap(([key,color])=>points.map((point,index)=>isolated(point,index,key)?<circle key={`${key}-${point.at}`} className={`traffic-sample traffic-${color}-sample`} cx={(point.at-first)/span*600} cy={78-Math.min(scale,point[key])/scale*76} r="1.5"/>:null))}</svg>{active&&<><span className="traffic-hover-guide" style={{left:`${activeX}%`}} aria-hidden="true"/>{[['receive_bps','download'],['send_bps','upload']].map(([key,color])=>Number.isFinite(active[key])&&active[key]>=0?<span key={key} className={`traffic-hover-dot traffic-${color}-dot`} style={{left:`${activeX}%`,top:`${(78-Math.min(scale,active[key])/scale*76)/80*100}%`}} aria-hidden="true"/>:null)}<div id="traffic-hover-tooltip" className="traffic-hover-tooltip" role="tooltip" style={{left:`${Math.max(20,Math.min(80,activeX))}%`}}><time dateTime={new Date(active.at).toISOString()}>{time(active.at)}</time><span className="traffic-upload"><i/>上传<strong>{trafficAmount(active.send_bps,true)}</strong></span><span className="traffic-download"><i/>下载<strong>{trafficAmount(active.receive_bps,true)}</strong></span></div></>}</div>:<div className="chart-empty">{reason}</div>}</div>
    <div className="traffic-time-axis" aria-label="实际采样时间范围"><span>{collecting&&span>0?trafficAge(span):'—'}</span><span>{collecting&&span>0?trafficAge(span/2):'—'}</span><span>{collecting&&span>0?'现在':'—'}</span></div><div className="traffic-totals"><span>累计上传 <strong>{collecting?trafficAmount(data.sent_bytes):'—'}</strong></span><span>累计下载 <strong>{collecting?trafficAmount(data.received_bytes):'—'}</strong></span><span>目标进程 {collecting&&Number.isFinite(data.target_count)?data.target_count:'—'}</span></div></article>;
}
export function CollectorSettings({result,service,trafficEnabled,observed,writable,busy,onEnable,onDisable,onInstall,onUninstall}){
  const data=service||{},state=observed?data.state||'error':'unknown';
  const installed=['running','stopped','outdated'].includes(state)||(state==='error'&&typeof data.version==='string'&&!!data.version);
  const installedVersion=installed&&typeof data.version==='string'&&data.version?data.version:null;
  const servicePending=busy('install-traffic-service')||busy('uninstall-traffic-service');
  const recordBusy=busy('settings-enable-traffic')||busy('settings-disable-traffic');
  const recordPending=trafficPending(busy);
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={!observed||!writable||servicePending||recordPending||typeof callback!=='function'} icon={busy(id)?<Symbol name="refresh" spin/>:undefined} onClick={callback}>{label}</Button>;
  const recordDescription=!observed?'等待辅助服务状态':state==='not_installed'?'请先安装辅助服务。':state==='outdated'?(data.error||'辅助服务协议或最低版本不满足要求，请卸载后重新安装一次。'):state==='error'?'辅助服务状态异常，请查看下方错误说明。':state==='unknown'?'正在读取辅助服务状态，请稍候。':'辅助服务按需运行，正常启停统计不再反复授权。';
  const unavailableReason=!observed?'正在读取辅助服务状态，请稍候。':!writable?'当前后台不可写，暂不能更改统计设置。':servicePending?'辅助服务操作进行中，完成后可更改统计。':!serviceReady(service)?recordDescription:'';
  const detailError=state==='outdated'?recordDescription:typeof data.error==='string'&&data.error?data.error:['needs_installation','needs_permission','unavailable'].includes(result?.state)&&typeof result?.message==='string'?result.message:'';
  return <><div className="settings-group"><Row icon="pulse" title="记录网络流量" description={<span className="traffic-setting-copy"><span>只计算字节数，数据只保存在本机。暂停守护会停止采集，恢复后按此开关设置继续。</span>{unavailableReason&&<span id="traffic-disabled-reason" className="disabled-reason">{unavailableReason}</span>}</span>}><div className="switch-control">{recordBusy&&<Symbol name="refresh" spin/>}<Switch id="traffic-enabled" aria-label="记录网络流量" aria-describedby={unavailableReason?'traffic-disabled-reason':undefined} title={!serviceReady(service)?recordDescription:undefined} checked={!!trafficEnabled} aria-busy={recordBusy} disabled={!observed||!writable||!serviceReady(service)||servicePending||recordPending} onChange={(_,value)=>value.checked?onEnable?.():onDisable?.()}/></div></Row></div><h3 className="collector-section-title">目标进程的流量</h3><div className="settings-group collector-settings"><Row icon="settings" title="流量辅助服务" description={<span className="service-meta"><StatusLabel color={state==='running'?'success':state==='outdated'?'warning':state==='error'?'danger':'subtle'}>{Object.hasOwn(serviceLabels,state)?serviceLabels[state]:'状态异常'}</StatusLabel>{installedVersion&&<span className="service-version">版本 {installedVersion}</span>}</span>}><div className="traffic-service-actions">{(state==='not_installed'||busy('install-traffic-service'))&&action('install-traffic-service','安装辅助服务',onInstall)}{(installed||busy('uninstall-traffic-service'))&&action('uninstall-traffic-service','卸载辅助服务',onUninstall)}</div></Row><div className="collector-copy"><p>Windows 需要辅助服务读取 ChatGPT / Codex CLI 进程的网络流量。只计算字节，不读取内容或记录连接对象。</p><p>首次安装或卸载时请求 Windows 授权；正常开启、关闭统计不再反复授权，服务仅按需运行。</p>{detailError&&<p className="warning-text">{detailError}</p>}</div></div></>;
}
export function App({initialStatus=null,initialPage=globalThis.__QA_NAV__?.page||'dashboard',initialSettingsTab=globalThis.__QA_NAV__?.settings||'home'}={}){
  useInputModality();
  const [page,setPage]=useState(initialPage==='about'?'settings':Object.hasOwn(pages,initialPage)?initialPage:'dashboard');
  const [settingsTab,setSettingsTab]=useState(initialPage==='about'?'about':Object.hasOwn(settingsPages,initialSettingsTab)?initialSettingsTab:'home');
  const mainScroll=useRef(null);
  const [confirmation,setConfirmation]=useState(null),[inspection,setInspection]=useState(null);
  const inspectionEpoch=useRef(0),runtimeRequests=useRef(new Map()),navigationEpoch=useRef(0);
  const [collapsed,setCollapsed]=useState(()=>{try{return localStorage.getItem('sidebar-collapsed')==='1';}catch{return false;}});
  const [themeMode,setThemeMode]=useState(readThemeMode),[systemDark,setSystemDark]=useState(prefersDark);
  const currentThemeMode=useRef(themeMode);currentThemeMode.current=themeMode;
  const resolvedTheme=resolveTheme(themeMode,systemDark);
  const [maximized,setMaximized]=useState(false),[pinned,setPinned]=useState(null),pinEpoch=useRef(0),[status,setStatus]=useState(initialStatus),current=useRef(initialStatus);
  const [online,setOnline]=useState(!!initialStatus),[,setActionVersion]=useState(0);
  const [trafficHistory,setTrafficHistory]=useState(()=>Array.isArray(initialStatus?.traffic_history)?initialStatus.traffic_history:appendTraffic([],initialStatus?.traffic,initialStatus?.traffic_enabled));
  const actions=useRef(null);if(!actions.current)actions.current=createActions(()=>setActionVersion(v=>v+1),{minimumVisibleMs:150});
  const [notice,setNotice]=useState(null),[proxy,setProxy]=useState(initialStatus?.proxy_url||'');
  const proxyEdit=useRef({dirty:false,version:0}),systemPending=useRef(new Set());
  const [system,setSystem]=useState(()=>systemPreferences({},initialStatus||{},new Set()));
  const [directory,setDirectory]=useState(initialStatus?.cli_workdir||''),directoryEdit=useRef({dirty:false,version:0}),directoryWrite=useRef(null);
  const [logTab,setLogTab]=useState('events');
  const [logFilter,setLogFilter]=useState('all'),[logPeriod,setLogPeriod]=useState('all'),[customStart,setCustomStart]=useState(''),[customEnd,setCustomEnd]=useState(''),[logQuery,setLogQuery]=useState(''),reads=useRef(0),cliStateEpoch=useRef(0);
  const busy=name=>actions.current.has(name);
  const writable=!!invoke&&online&&status?.backend_ready&&!status?.read_only&&!(status?.app_update?.busy&&status.app_update.operation==='install');
  const observed=online&&!!status&&status.revision>0;
  const updateOutcomeSeen=useRef(null);
  useEffect(()=>{const result=status?.app_update;if(!result?.completed_at||result.completed_at<status.started_at||result.completed_at===updateOutcomeSeen.current)return;if(!['installed','rolled_back','installed_cleanup_pending','error'].includes(result.phase))return;updateOutcomeSeen.current=result.completed_at;setNotice({text:result.phase==='installed'?`助手已更新至 ${result.version}。`:result.error||'助手更新未完成，请查看更新设置。',intent:result.phase==='installed'?'success':'error'});},[status?.app_update?.completed_at,status?.started_at]);

  const show=(text,intent='info')=>setNotice({text,intent});
  function navigate(target){if(target==='about'){setSettingsTab('about');setPage('settings');}else if(Object.hasOwn(pages,target)){setPage(target);}}
  useThemeLayoutEffect(()=>{mainScroll.current?.scrollTo({top:0,left:0,behavior:'instant'});if(globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches)return;const surface=mainScroll.current?.querySelector('.main-content > section:not([hidden])'),motion=surface?.animate?.([{translate:'0 6px'},{translate:'0 0'}],{duration:220,easing:'cubic-bezier(.2,0,0,1)'});return()=>motion?.cancel();},[page,settingsTab]);
  useThemeLayoutEffect(()=>applyDocumentTheme(resolvedTheme),[resolvedTheme]);
  useThemeLayoutEffect(()=>{
    if(!appWindow||!invoke)return;
    let disposed=false;
    // A committed DOM is ready even when a hidden window receives no animation
    // frames or the optional native theme synchronization is still pending.
    const report=async()=>{try{const requested=await invoke('frontend_ready',{rendered:document.getElementById('root')?.childElementCount>0,geometry:renderGeometry()});if(!disposed)navigate(requested);}catch{if(!disposed)show('窗口初始化失败，请重新打开助手。','warning');}};
    report();return()=>{disposed=true;};
  },[]);
  useEffect(()=>{
    let disposed=false;
    const media=globalThis.matchMedia?.(systemThemeQuery);
    const failed=()=>{if(!disposed)show('窗口主题同步失败。','warning');};
    const changed=event=>{if(disposed)return;setSystemDark(event.matches);if(currentThemeMode.current==='system'){applyDocumentTheme(resolveTheme('system',event.matches));syncNativeAppearance('system').catch(failed);}};
    media?.addEventListener('change',changed);
    syncNativeAppearance(currentThemeMode.current).catch(failed);
    return()=>{disposed=true;media?.removeEventListener('change',changed);};
  },[]);
  async function readStatus(){
    const epoch=cliStateEpoch.current,data=await invoke('get_status');
    // A poll started before a CLI check completed must not restore its in-progress snapshot.
    return epoch===cliStateEpoch.current?data:{...data,cli_management:current.current?.cli_management??data.cli_management};
  }
  function acceptCli(data){
    if(!data||typeof data!=='object'||Array.isArray(data)||typeof data.source!=='string'||typeof data.supported!=='boolean'||data.operation!==null&&typeof data.operation!=='string')throw Error('CLI 检查结果无效，已保留现有状态。');
    const next={...current.current,cli_management:data};current.current=next;reads.current++;setStatus(next);setOnline(true);
  }
  function accept(data){
    if(!data||!Number.isFinite(data.revision)||!Array.isArray(data.logs))throw Error('后台状态无效。');
    const next=acceptSnapshot(current.current,data);current.current=next;reads.current++;setStatus(next);setOnline(true);setTrafficHistory(history=>Array.isArray(next.traffic_history)?next.traffic_history:appendTraffic(history,next.traffic,next.traffic_enabled));
    setNotice(value=>value?.connection?null:value);
    if(!proxyEdit.current.dirty)setProxy(next.proxy_url);
    setSystem(value=>systemPreferences(value,next,systemPending.current));
    if(!directoryEdit.current.dirty)setDirectory(next.cli_workdir||'');
    return next;
  }
  useEffect(()=>{
    let disposed=false,pending=false;
    async function poll(){if(!invoke||pending)return;pending=true;const before=reads.current;try{const data=await readStatus();if(!disposed)accept(data);}catch{if(!disposed&&reads.current===before){setOnline(false);setNotice(value=>value&&!value.connection?value:{text:'后台连接中断，正在重试。',intent:'warning',connection:true});}}finally{pending=false;}}
    poll();const timer=setInterval(poll,2500);return()=>{disposed=true;clearInterval(timer);};
  },[]);
  useEffect(()=>{
    let disposed=false;const stops=[];
    const failed=()=>{if(!disposed)show('窗口状态同步失败，请重新打开助手。','warning');};
    const registered=promise=>promise.then(stop=>disposed?stop():stops.push(stop)).catch(failed);
    if(native?.event?.listen)registered(native.event.listen('navigate',event=>navigate(event.payload)));
    if(appWindow){const sync=async()=>{const epoch=pinEpoch.current,pinPending=busy('window-pin');try{const [maximum,top]=await Promise.all([appWindow.isMaximized(),appWindow.isAlwaysOnTop?.()]);if(!disposed){setMaximized(maximum);if(!pinPending&&pinEpoch.current===epoch&&typeof top==='boolean')setPinned(top);}}catch{failed();}};sync();registered(appWindow.onResized(sync));if(appWindow.onFocusChanged)registered(appWindow.onFocusChanged(event=>{if(event.payload)sync();}));}
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
  async function saveDirectory(event){event?.preventDefault();if(!directoryEdit.current.dirty||busy('cli-workdir'))return;const submitted=directoryEdit.current.version,write=changeSystem('cliWorkdir',directory.trim());directoryWrite.current=write;try{await write;}finally{if(directoryWrite.current===write)directoryWrite.current=null;}if(submitted===directoryEdit.current.version&&current.current?.cli_workdir===directory.trim()){directoryEdit.current.dirty=false;setDirectory(current.current.cli_workdir);}}
  async function chooseDirectory(){if(!writable||!appWindow||busy('choose-cli-workdir'))return;const epoch=navigationEpoch.current;let selected=null;await actions.current.run('choose-cli-workdir',async()=>{try{selected=await invoke('choose_cli_workdir',{initialDirectory:directory.trim()||null});if(selected!==null&&typeof selected!=='string')throw Error('文件夹选择结果无效。');}catch(error){selected=null;if(epoch===navigationEpoch.current)show(errorText(error),'error');}});if(!selected||epoch!==navigationEpoch.current)return;while(directoryWrite.current)await directoryWrite.current;if(epoch!==navigationEpoch.current)return;directoryEdit.current.dirty=true;const submitted=++directoryEdit.current.version;setDirectory(selected);const write=changeSystem('cliWorkdir',selected);directoryWrite.current=write;try{await write;}finally{if(directoryWrite.current===write)directoryWrite.current=null;}if(submitted===directoryEdit.current.version&&current.current?.cli_workdir===selected){directoryEdit.current.dirty=false;setDirectory(selected);}}
  async function checkUpdates(){await actions.current.run('check-updates',async()=>{try{await invoke('check_updates');accept(await readStatus());}catch(error){try{accept(await readStatus());}catch{}show(errorText(error),'error');}});}
  async function checkConnectivity(){await actions.current.run('check-connectivity',async()=>{try{await invoke('check_connectivity');accept(await readStatus());}catch(error){try{accept(await readStatus());}catch{}show(errorText(error),'error');}});}
  async function trafficAction(name,command,args){if(trafficPending(busy))return;await actions.current.run(name,async()=>{try{await invoke(command,args);accept(await readStatus());}catch(error){try{accept(await readStatus());}catch{}show(errorText(error),'error');}});}
  async function requestDesktop(mode,actionKey=`desktop-prepare-${mode}`){const epoch=navigationEpoch.current;await actions.current.run(actionKey,async()=>{try{const target=await invoke('capture_desktop_target');if(navigationEpoch.current===epoch)setConfirmation({kind:`desktop-${mode}`,target});}catch(error){if(navigationEpoch.current===epoch)show(errorText(error),'error');}});}
  async function manageCli(kind){const data=status?.cli_management;if(kind==='install'?!data?.install_token:!data?.supported||!data.target_token)return;if(kind==='install'){await checkedOperation('cli-install','install_cli',{targetToken:data.install_token},'Codex CLI 已安装。');return;}if(kind==='update'){await checkedOperation('cli-update','update_cli',{targetToken:data.target_token},'Codex CLI 更新操作已完成。');return;}setConfirmation({kind:`cli-${kind}`,targetToken:data.target_token,current:data.current,latest:data.latest});}
  async function checkedOperation(name,command,args,success){let succeeded=false;await actions.current.run(name,async()=>{
    if(name==='cli-check'){
      cliStateEpoch.current++;
      try{const data=await invoke(command,args);cliStateEpoch.current++;acceptCli(data);succeeded=true;}
      catch(error){cliStateEpoch.current++;const before=current.current?.cli_management;if(before){const operation=before.operation==='check'?null:before.operation;acceptCli({...before,operation,progress:operation?before.progress:'',error:errorText(error)});}show(errorText(error),'error');}
      return;
    }
    try{await invoke(command,args);accept(await readStatus());if(success)show(success,'success');succeeded=true;}catch(error){try{accept(await readStatus());}catch{}show(errorText(error),'error');}
  });return succeeded;}
  async function confirmOperation(){const request=confirmation;if(!request)return;const name=request.kind;const ok=name.startsWith('desktop-')?await checkedOperation('confirm-action','manage_desktop',{target:request.target,restart:name==='desktop-restart'},name==='desktop-restart'?'已正常关闭，并请求重新打开 ChatGPT。':'ChatGPT 已正常关闭。'):await checkedOperation('confirm-action',name==='cli-update'?'update_cli':'uninstall_cli',{targetToken:request.targetToken},name==='cli-update'?'Codex CLI 更新操作已完成。':'Codex CLI 已卸载，配置和历史已保留。');setConfirmation(null);}
  function readRuntime(kind){if(runtimeRequests.current.has(kind))return runtimeRequests.current.get(kind);const request=kind==='connectivity'?Promise.resolve({...current.current?.connectivity,sampled_at:current.current?.connectivity?.checked_at}):invoke(kind==='connections'?'get_runtime_connections':'get_runtime_memory');runtimeRequests.current.set(kind,request);request.finally(()=>{if(runtimeRequests.current.get(kind)===request)runtimeRequests.current.delete(kind);}).catch(()=>{});return request;}
  async function inspectRuntime(kind,background=false){const epoch=background?inspectionEpoch.current:++inspectionEpoch.current;if(!background)setInspection({kind,loading:true,data:null,error:null,selected:null});const task=async()=>{try{const data=await readRuntime(kind);if(inspectionEpoch.current===epoch)setInspection(value=>value?.kind===kind?{...value,loading:false,data,error:null}:value);}catch(error){if(inspectionEpoch.current===epoch)setInspection(value=>value?.kind===kind?{...value,loading:false,error:errorText(error)}:value);}};if(background)await task();else{const key=`details-${kind}-${epoch}`;await actions.current.run(key,task);}}
  useEffect(()=>{if(!inspection)return;const timer=setInterval(()=>{if(!runtimeRequests.current.has(inspection.kind))inspectRuntime(inspection.kind,true);},3000);return()=>clearInterval(timer);},[inspection?.kind]);
  useEffect(()=>{navigationEpoch.current++;inspectionEpoch.current++;setInspection(null);setConfirmation(null);},[page,settingsTab]);
  async function windowAction(action){try{await appWindow?.[action]();}catch(error){show(errorText(error),'error');}}
  function recordUi(event){invoke?.('record_ui_event',{event}).catch(()=>{});}
  async function togglePin(){if(!appWindow||pinned===null||busy('window-pin'))return;pinEpoch.current++;await actions.current.run('window-pin',async()=>{try{const desired=!pinned;await appWindow.setAlwaysOnTop(desired);const actual=await appWindow.isAlwaysOnTop();setPinned(actual);if(actual!==desired)throw Error('窗口置顶状态未能更新。');recordUi(actual?'window_pin_on':'window_pin_off');}catch(error){recordUi('window_pin_failed');show(errorText(error),'error');}});}
  async function changeTheme(mode){
    if(!themeModes.includes(mode)||busy('appearance'))return;
    currentThemeMode.current=mode;setThemeMode(mode);applyDocumentTheme(resolveTheme(mode,systemDark));try{localStorage.setItem(themeKey,mode);}catch{}
    await actions.current.run('appearance',async()=>{try{await syncNativeAppearance(mode);recordUi('appearance_changed');}catch{show('窗口主题同步失败。','warning');}});
  }
  function fold(){setCollapsed(value=>{try{localStorage.setItem('sidebar-collapsed',value?'0':'1');}catch{}return !value;});}
  const button=(id,text,icon,onClick,{primary=false,disabled=false,type='button',iconOnly=false}={})=><Button id={id} type={type} className={iconOnly?'icon-action':'text-action'} appearance={primary?'primary':'subtle'} icon={busy(id)?<Symbol name="refresh" spin/>:icon?<Symbol name={icon}/>:undefined} aria-label={iconOnly?text:undefined} title={iconOnly?text:undefined} aria-busy={busy(id)} disabled={!invoke||busy(id)||disabled} onClick={onClick}>{iconOnly?null:text}</Button>;
  const ready=status?.enabled&&(status?.guard_ready||status?.cli_ready)&&status?.proxy_reachable&&!status?.error;
  const guardText=!status?'正在连接后台':!online?'后台未连接':!status.backend_ready?'助手未启动':!status.enabled?'守护已暂停':status.error?'守护需要检查':ready?'守护已就绪':!status.proxy_reachable?'等待本地代理':'等待应用接入';
  const {start:logStart,end:logEnd}=logPeriod==='custom'?{start:customStart,end:customEnd}:logDateRange(logPeriod);
  const logs=status?.logs.slice().reverse()||[],filtered=filterLogs(logs,{level:logFilter,start:logStart,end:logEnd,query:logQuery});
  const samples=status?.probe_history||[],path=probePath(samples),scale=probeScale(samples);
  const aboutStatus={...(status||{}),network_enabled:system.networkEnabled,auto_check_updates:system.autoCheckUpdates,update_interval_hours:system.updateIntervalHours};

  return <FluentProvider theme={themes[resolvedTheme]} data-theme={resolvedTheme} style={{backgroundColor:'transparent'}}><div className={`app-shell ${collapsed?'sidebar-collapsed':''}`}>
    <div className="window-drag" data-tauri-drag-region/>
    <div className="window-controls" aria-label="窗口控制"><Button id="window-pin" className="window-pin" appearance="transparent" aria-label="窗口置顶" title={pinned?'取消窗口置顶':'窗口置顶'} aria-pressed={pinned??false} aria-busy={busy('window-pin')} aria-disabled={busy('window-pin')||undefined} disabled={!appWindow||pinned===null} icon={<Symbol name={pinned?'pinned':'pin'}/>} onClick={togglePin}/><Button appearance="transparent" aria-label="最小化" title="最小化" disabled={!appWindow} icon={<Symbol name="minimize"/>} onClick={()=>windowAction('minimize')}/><Button appearance="transparent" aria-label={maximized?'还原':'最大化'} title={maximized?'还原':'最大化'} disabled={!appWindow} icon={<Symbol name={maximized?'restore':'maximize'}/>} onClick={()=>windowAction('toggleMaximize')}/><Button appearance="transparent" className="window-close" aria-label="关闭到托盘" title="关闭到托盘" disabled={!appWindow} icon={<Symbol name="dismiss"/>} onClick={()=>windowAction('close')}/></div>
    <aside className="sidebar">
      <div className="brand" data-tauri-drag-region><Button appearance="subtle" className="sidebar-toggle" aria-label={collapsed?'展开侧边栏':'收起侧边栏'} aria-expanded={!collapsed} aria-controls="app-sidebar-nav" title={collapsed?'展开侧边栏':'收起侧边栏'} icon={<SidebarToggleIcon collapsed={collapsed}/>} onClick={fold}/><div className="brand-label" aria-hidden={collapsed} data-tauri-drag-region><img src="icon.svg" alt="" data-tauri-drag-region/><strong data-tauri-drag-region>CodexProxyTray</strong></div></div>
      <nav id="app-sidebar-nav" aria-label="页面导航">{Object.entries(pages).map(([key,label])=><Button key={key} data-page={key} appearance="subtle" className={`nav-item ${page===key?'selected':''}`} aria-current={page===key?'page':undefined} aria-label={label} title={collapsed?label:undefined} onClick={()=>setPage(key)}><Symbol name={key}/><span className="nav-label" aria-hidden={collapsed}>{label}</span></Button>)}</nav>

    </aside>
    <main className="main-scroll" ref={mainScroll}><div className={page==='dashboard'?'main-content dashboard-content':'main-content'}>
      <header className="page-header" data-tauri-drag-region><div className="page-title">{page==='settings'&&settingsTab!=='home'&&<Button appearance="subtle" className="about-back" aria-label={['privacy','disclaimer'].includes(settingsTab)?'返回关于':'返回设置'} onClick={()=>setSettingsTab(['privacy','disclaimer'].includes(settingsTab)?'about':'home')} icon={<Icon icon={icons['arrow-left-24-regular']} aria-hidden="true"/>}/>}<h1 key={`${page}-${settingsTab}`} data-tauri-drag-region>{page==='settings'?settingsPages[settingsTab][0]:pages[page]}</h1></div></header>
      {notice?.connection&&<MessageBar intent={notice.intent} icon={<Symbol name={notice.intent==='success'?'success':'warning'}/>}><MessageBarBody>{notice.text}</MessageBarBody><MessageBarActions containerAction={<Button appearance="transparent" size="small" aria-label="关闭提示" icon={<Symbol name="dismiss"/>} onClick={()=>setNotice(null)}/>}/></MessageBar>}
      {!invoke&&<MessageBar intent="warning" icon={<Symbol name="warning"/>}><MessageBarBody>请运行 CodexProxyTray 桌面程序。</MessageBarBody></MessageBar>}
      {status?.read_only&&<div className="preview-note">本机真实状态 · 修改设置请使用桌面窗口</div>}
      <section hidden={page!=='dashboard'} aria-label="仪表盘">
        <div className={`restart-hint-shell ${(status?.restart_required||status?.desktop_proxy?.state==='unknown')&&status.enabled?'is-visible':''}`} aria-hidden={!(status?.restart_required||status?.desktop_proxy?.state==='unknown')||!status.enabled}><div className="restart-hint-clip"><div className="inline-note restart-hint"><Symbol name="about"/><span title={status?.desktop_proxy?.reason}>{status?.restart_required?'重开 ChatGPT 后应用代理设置':'当前进程的代理配置尚未确认'}</span><div className="restart-actions">{button('desktop-prepare-close','关闭 ChatGPT','power',()=>requestDesktop('close'),{disabled:!writable||!!status?.desktop_operation?.busy})}{button('desktop-prepare-restart','重启 ChatGPT','refresh',()=>requestDesktop('restart'),{disabled:!writable||!!status?.desktop_operation?.busy})}</div></div></div></div>
        <div className="dashboard-grid">
          <TrafficCard result={status?.traffic} service={status?.traffic_service} trafficEnabled={!!status?.traffic_enabled} guardEnabled={!!status?.enabled} observed={observed} history={trafficHistory}/>
          <article className="metric-card guard-card"><div className="metric-label"><Symbol name="shield"/>代理守护</div><strong className="guard-value">{guardText}</strong><div className="guard-targets"><span>ChatGPT</span><StatusLabel color={observed&&status?.guard_ready?'success':'subtle'}>{!observed?'待检测':status.guard_ready?'已就绪':'未接入'}</StatusLabel><span>Codex CLI</span><StatusLabel color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</StatusLabel></div></article>
          <div className="dashboard-summary"><Metric className="desktop-card" icon="chatgpt" label="ChatGPT" value={!observed?'—':status.codex_running?'运行中':'未运行'} detail={!observed?'等待应用状态':status.desktop_pid?`PID ${status.desktop_pid}`:status.package_version?`已安装 ${status.package_version}`:'尚未检测到应用'}/><article className="metric-card local-probe-card"><div className="metric-label"><Symbol name="latency"/>本地 TCP 延迟<Tooltip content="本机 TCP 连接采样，约 2 秒一次；不是 ChatGPT 远端延迟。" relationship="description"><Button className="metric-help" appearance="transparent" aria-label="本地采样说明" icon={<Symbol name="about"/>}/></Tooltip></div><div className="metric-value">{observed&&Number.isFinite(status?.proxy_latency_ms)?status.proxy_latency_ms.toFixed(2):'—'}<small> ms</small></div><StatusLabel color={observed&&status?.proxy_reachable?'success':'subtle'}>{!observed?'待检测':status.proxy_reachable?'可连接':'未连接'}</StatusLabel></article><button type="button" className="metric-card inspect-card" id="open-connections" aria-haspopup="dialog" aria-busy={inspection?.kind==='connections'&&inspection.loading||false} onClick={()=>inspectRuntime('connections')}><span className="metric-label"><Symbol name="pulse"/>实时连接<Icon className="card-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/></span><strong className="metric-value">{observed&&Number.isFinite(status?.tcp_connections)?status.tcp_connections:'—'}</strong><span className="metric-detail">ChatGPT TCP 连接</span></button><button type="button" className="metric-card inspect-card" id="open-memory" aria-haspopup="dialog" aria-busy={inspection?.kind==='memory'&&inspection.loading||false} onClick={()=>inspectRuntime('memory')}><span className="metric-label"><Symbol name="memory"/>内存信息<Icon className="card-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/></span><strong className="metric-value">{observed&&Number.isFinite(status?.helper_memory_bytes)?<>{(status.helper_memory_bytes/1048576).toFixed(1)}<small> MB</small></>:<span className="metric-pending">待采样</span>}</strong><span className="metric-detail">助手主进程工作集</span></button></div>
          <ConnectivityCard result={status?.connectivity} observed={observed} writable={writable} enabled={system.networkEnabled&&system.connectivityEnabled} busy={busy('check-connectivity')} onCheck={checkConnectivity} onOpen={()=>inspectRuntime('connectivity')}/>

        </div>
        {status?.error&&<div className="inline-note error-text"><Symbol name="warning"/>{status.error}</div>}

      </section>
      <section hidden={page!=='proxy'} aria-label="代理">
        <form onSubmit={saveProxy} className="settings-group"><Row icon="proxy" title="本地 HTTP 代理" description={<><span className={`status-indicator ${observed&&status?.proxy_reachable?'active':''}`}/>{!observed?'待检测':status.proxy_reachable?'端口可用':'暂未连接'}</>} className="proxy-row"><Input id="proxy-url" aria-label="本地 HTTP 代理地址" value={proxy} readOnly={!writable} spellCheck={false} onChange={(_,data)=>{proxyEdit.current.dirty=true;proxyEdit.current.version++;setProxy(data.value);}}/>{button('save','应用','save',undefined,{type:'submit',primary:true,disabled:!writable||busy('toggle')})}</Row></form>
        <h2>应用接入</h2><Accordion className="target-accordion" multiple collapsible><AccordionItem value="desktop"><AccordionHeader expandIconPosition="end" icon={<Symbol name="chatgpt"/>}><span className="target-copy"><strong>ChatGPT</strong><small>{!observed?'等待应用状态':status.package_count?`版本 ${status.package_version||'已安装'}`:'未检测到桌面应用'}</small></span><StatusLabel color={observed&&status?.codex_running?'success':'subtle'}>{!observed?'待检测':status.codex_running?'运行中':'未运行'}</StatusLabel></AccordionHeader><AccordionPanel><div className="target-actions">{button('launch','打开 ChatGPT','open',()=>run('launch','launch_codex'),{disabled:!writable||!status?.package_count||!!status?.desktop_operation?.busy})}{status?.codex_running&&<>{button('desktop-close','关闭 ChatGPT','power',()=>requestDesktop('close','desktop-close'),{disabled:!writable||!!status?.desktop_operation?.busy})}{button('desktop-restart','重启 ChatGPT','refresh',()=>requestDesktop('restart','desktop-restart'),{disabled:!writable||!!status?.desktop_operation?.busy})}</>}</div></AccordionPanel></AccordionItem><AccordionItem value="cli"><AccordionHeader expandIconPosition="end" icon={<Symbol name="terminal"/>}><span className="target-copy"><strong>Codex CLI</strong><small>{status?.cli_management?.current?`版本 ${status.cli_management.current}`:!observed?'等待 CLI 状态':status.cli_ready?'通过专用入口在终端中运行':'未检测到 Windows CLI'}</small></span><StatusLabel color={observed&&status?.cli_ready?'success':'subtle'}>{!observed?'待检测':status.cli_ready?'已接入':'未接入'}</StatusLabel></AccordionHeader><AccordionPanel><div className="target-actions">{button('launch-cli','打开 Codex CLI','open',()=>run('launch-cli','launch_cli'),{disabled:!writable||!status?.cli_ready||!!status?.cli_management?.operation})}</div><CliManagement data={status?.cli_management} writable={writable} networkEnabled={system.networkEnabled} busy={busy} onCheck={()=>checkedOperation('cli-check','check_cli_updates')} onInstall={()=>manageCli('install')} onUpdate={()=>manageCli('update')} onUninstall={()=>manageCli('uninstall')}/></AccordionPanel></AccordionItem></Accordion>

      </section>
      <section hidden={page!=='settings'} aria-label="设置"><article className="settings-panel" id="settings-panel-home" hidden={settingsTab!=='home'}>{page==='settings'&&settingsTab==='home'&&<><h2 className="first-section">外观</h2><div className="settings-group"><Row icon="settings" title="主题" description="跟随系统，或固定使用浅色、深色。"><div className="switch-control">{busy('appearance')&&<Symbol name="refresh" spin/>}<Choice id="color-theme" label="界面主题" busy={busy('appearance')} value={themeMode} disabled={busy('appearance')} options={[{value:'system',label:'跟随系统'},{value:'light',label:'浅色'},{value:'dark',label:'深色'}]} onChange={changeTheme}/></div></Row></div><h2>启动</h2><div className="settings-group"><Row icon="power" title="登录时自动启动" description="登录 Windows 后在托盘运行。"><div className="switch-control">{busy('autostart')&&<Symbol name="refresh" spin/>}<Switch id="autostart" aria-label="登录时自动启动" aria-busy={busy('autostart')} checked={system.autostart} disabled={!writable||busy('autostart')} onChange={(_,data)=>changeSystem('autostart',data.checked)}/></div></Row><Row icon="desktop" title="启动后打开" description="助手启动时自动打开所选应用。"><div className="switch-control">{busy('launch-target')&&<Symbol name="refresh" spin/>}<Choice id="launch-target" label="启动后打开" busy={busy('launch-target')} value={system.launchTarget} disabled={!writable||busy('launch-target')} options={[{value:'none',label:'不打开应用'},{value:'chatgpt',label:'ChatGPT'},{value:'cli',label:'Codex CLI'},{value:'both',label:'ChatGPT 与 Codex CLI'}]} onChange={value=>changeSystem('launchTarget',value)}/></div></Row></div><h2>网络与流量</h2><div className="settings-group"><Row icon="desktop" title="ChatGPT 连通检测" description="经当前代理入口探测网页，不验证账号或对话。"><div className="switch-control">{busy('connectivity-enabled')&&<Symbol name="refresh" spin/>}<Switch id="connectivity-enabled" aria-label="ChatGPT 连通检测" aria-busy={busy('connectivity-enabled')} checked={system.connectivityEnabled} disabled={!writable||!system.networkEnabled||busy('connectivity-enabled')} onChange={(_,data)=>changeSystem('connectivityEnabled',data.checked)}/></div></Row></div><CollectorSettings result={status?.traffic} service={status?.traffic_service} trafficEnabled={!!status?.traffic_enabled} observed={observed} writable={writable} busy={busy} onEnable={()=>trafficAction('settings-enable-traffic','enable_traffic')} onDisable={()=>trafficAction('settings-disable-traffic','disable_traffic')} onInstall={()=>trafficAction('install-traffic-service','install_traffic_service')} onUninstall={()=>trafficAction('uninstall-traffic-service','uninstall_traffic_service')}/><h2>Codex CLI</h2><form className="settings-group" onSubmit={saveDirectory}><Row icon="terminal" title="工作目录" description="留空使用用户目录。离开输入框时自动保存。" className="workdir-row"><Input id="cli-workdir" aria-label="Codex CLI 工作目录" value={directory} placeholder="使用用户目录" aria-busy={busy('cli-workdir')} readOnly={!writable||busy('cli-workdir')||busy('choose-cli-workdir')} spellCheck={false} onChange={(_,data)=>{directoryEdit.current.dirty=true;directoryEdit.current.version++;setDirectory(data.value);}} onBlur={saveDirectory}/>{busy('cli-workdir')&&<Symbol name="refresh" spin/>}<Tooltip content={appWindow?'选择工作目录':'请在桌面窗口中选择目录'} relationship="description"><Button id="choose-cli-workdir" className="text-action" appearance="subtle" aria-label="浏览工作目录" aria-busy={busy('choose-cli-workdir')} aria-disabled={busy('choose-cli-workdir')||undefined} disabled={!writable||!appWindow} icon={<Symbol name={busy('choose-cli-workdir')?'refresh':'open'} spin={busy('choose-cli-workdir')}/>} onPointerDown={event=>event.preventDefault()} onClick={chooseDirectory}>浏览目录</Button></Tooltip></Row></form><UpdateSettings status={aboutStatus} busy={busy} writable={writable} onPreference={changeSystem} onCheck={checkUpdates} version={version} onRelease={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/CodexProxyTray/releases','_blank','noopener,noreferrer');return;}run('open-release','open_release');}} onDownload={appWindow?latest=>checkedOperation('download-app-update','download_app_update',{expectedVersion:latest}):undefined} onInstall={appWindow?latest=>checkedOperation('install-app-update','install_app_update',{expectedVersion:latest}):undefined}/><h2>其他</h2><div className="settings-group"><button type="button" className="about-detail-entry" data-settings-page="about" onClick={()=>setSettingsTab('about')}><Symbol name="about"/><span>关于</span><Icon className="card-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/></button></div></>}</article>
        <article className="settings-panel" id="settings-panel-about" aria-label="about" hidden={settingsTab!=='about'}>{page==='settings'&&settingsTab==='about'&&<><About onPrivacy={()=>setSettingsTab('privacy')} onDisclaimer={()=>setSettingsTab('disclaimer')} status={aboutStatus} version={version} busy={busy} writable={writable} onPreference={changeSystem} onCheck={checkUpdates} onSource={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/CodexProxyTray','_blank','noopener,noreferrer');return;}run('open-source','open_source');}} onRelease={()=>{if(status?.read_only||!invoke){window.open('https://github.com/OrangeZh1/CodexProxyTray/releases','_blank','noopener,noreferrer');return;}run('open-release','open_release');}} onLogs={writable?()=>run('open-logs','open_logs'):undefined}/></>}</article>

        <article className="settings-panel" id="settings-panel-privacy" hidden={settingsTab!=='privacy'}>{page==='settings'&&settingsTab==='privacy'&&<PrivacyDetails/>}</article>
        <article className="settings-panel" id="settings-panel-disclaimer" hidden={settingsTab!=='disclaimer'}>{page==='settings'&&settingsTab==='disclaimer'&&<Disclaimer/>}</article>
      </section>
      <section hidden={page!=='usage'} aria-label="用量统计"><UsageView invoke={invoke} active={page==='usage'}/></section>
      <section hidden={page!=='logs'} aria-label="运行日志">
        <TabList aria-label="日志类型" selectedValue={logTab} onTabSelect={(_,value)=>setLogTab(value.value)}><Tab id="log-tab-events" value="events" aria-controls="log-panel-events">运行事件</Tab><Tab id="log-tab-traffic" value="traffic" aria-controls="log-panel-traffic">流量日志</Tab></TabList>
        <div role="tabpanel" id="log-panel-traffic" aria-labelledby="log-tab-traffic" hidden={logTab!=='traffic'}>{page==='logs'&&logTab==='traffic'&&<FlowLogs invoke={appWindow?invoke:null} writable={writable&&!!appWindow}/>}</div>
        <div role="tabpanel" id="log-panel-events" aria-labelledby="log-tab-events" hidden={logTab!=='events'}>
        <LogFilters level={logFilter} period={logPeriod} query={logQuery} start={logStart} end={logEnd} onLevel={setLogFilter} onPeriod={value=>{if(value==='custom'){setCustomStart(logStart);setCustomEnd(logEnd);}setLogPeriod(value);}} onQuery={setLogQuery} onStart={setCustomStart} onEnd={setCustomEnd} onReset={()=>{setLogPeriod('all');setCustomStart('');setCustomEnd('');setLogQuery('');setLogFilter('all');}}/>
        {logStart&&logEnd&&logStart>logEnd&&<div className="inline-note error-text" role="alert">开始日期不能晚于结束日期。</div>}
        {status?.log_storage_error&&<div className="inline-note error-text">历史日志保存异常：{status.log_storage_error}</div>}
        <div id="log-results" role="region" aria-label="筛选后的运行日志">{page==='logs'&&logTab==='events'&&<EventList logs={filtered} emptyLabel={logStart||logEnd||logQuery||logFilter!=='all'?'没有符合条件的日志':'暂无日志'}/>}</div>
        </div>
      </section>

    </div></main>
    {page==='dashboard'&&<GuardButton status={status} observed={observed} writable={writable&&!busy('save')} busy={busy('toggle')} onToggle={()=>run('toggle','set_enabled',{enabled:!status?.enabled})}/>}
  </div><NoticeToasts notice={notice} onDismiss={()=>setNotice(null)}/><ConfirmAction request={confirmation} pending={busy('confirm-action')} onCancel={()=>setConfirmation(null)} onConfirm={confirmOperation}/><RuntimeDrawer view={inspection} onClose={()=>{inspectionEpoch.current++;setInspection(null);}} onConnection={row=>setInspection(value=>value?{...value,selected:row}:value)}/></FluentProvider>;
}
export function TrayApp({initialStatus=null}={}){
  useInputModality();
  const [status,setStatus]=useState(initialStatus),[online,setOnline]=useState(!!initialStatus),[active,setActive]=useState(false),[notice,setNotice]=useState(''),[bootstrapped,setBootstrapped]=useState(false);
  const [mode,setMode]=useState(readThemeMode),[systemDark,setSystemDark]=useState(prefersDark),[,changed]=useState(0);
  const actions=useRef(null);if(!actions.current)actions.current=createActions(()=>changed(v=>v+1),{minimumVisibleMs:150});
  const busy=name=>actions.current.has(name),resolved=resolveTheme(mode,systemDark);
  const mounted=useRef(true),menu=useRef(null);
  useThemeLayoutEffect(()=>applyDocumentTheme(resolved),[resolved]);
  useEffect(()=>{
    let disposed=false;mounted.current=true;let stop;
    const media=globalThis.matchMedia?.(systemThemeQuery),onMedia=event=>setSystemDark(event.matches),onStorage=()=>setMode(readThemeMode());
    media?.addEventListener('change',onMedia);globalThis.addEventListener?.('storage',onStorage);
    const start=async()=>{try{await appWindow?.setTheme?.(mode==='system'?null:mode);if(appWindow?.onFocusChanged){const unlisten=await appWindow.onFocusChanged(({payload})=>{if(!disposed){setActive(payload);if(payload){queueMicrotask(()=>menu.current?.focus({preventScroll:true}));}}});if(disposed)unlisten();else stop=unlisten;}const first=await invoke?.('get_status');if(!disposed&&first){setStatus(current=>acceptSnapshot(current,first));setOnline(true);}}catch(error){if(!disposed)setNotice(errorText(error));}finally{if(!disposed)setBootstrapped(true);}};
    start();
    return()=>{disposed=true;mounted.current=false;stop?.();media?.removeEventListener('change',onMedia);globalThis.removeEventListener?.('storage',onStorage);};
  },[]);
  useEffect(()=>{appWindow?.setTheme?.(mode==='system'?null:mode).catch(()=>{});},[mode]);
  useEffect(()=>{if(!bootstrapped||!invoke)return;const surface=document.querySelector('.tray-surface');if(!surface)return;let lastHeight=0;const report=()=>{const geometry=renderGeometry();if(geometry.tray_height===lastHeight)return;lastHeight=geometry.tray_height;invoke('frontend_ready',{rendered:true,geometry}).catch(()=>{});};report();if(!globalThis.ResizeObserver)return;const observer=new ResizeObserver(report);observer.observe(surface);return()=>observer.disconnect();},[bootstrapped]);
  useEffect(()=>{
    if(!active||!invoke)return;let disposed=false,pending=false;
    const poll=async()=>{if(pending)return;pending=true;try{const next=await invoke('get_status');if(!disposed){setStatus(current=>acceptSnapshot(current,next));setOnline(true);}}catch{if(!disposed)setOnline(false);}finally{pending=false;}};
    poll();const timer=setInterval(poll,2500);return()=>{disposed=true;clearInterval(timer);};
  },[active]);
  async function action(name,command,args){let succeeded=false;await actions.current.run(name,async()=>{try{const result=await invoke(command,args);if(result&&mounted.current)setStatus(current=>acceptSnapshot(current,result));succeeded=true;}catch(error){if(mounted.current)setNotice(errorText(error));}});if(succeeded)await appWindow?.hide?.();}
  const pending=['dashboard','logs','settings','launch','launch-cli','toggle','quit'].some(busy);
  const writable=!!invoke&&online&&status?.backend_ready;
  const ready=writable&&status?.enabled&&!status?.error&&(status?.guard_ready||status?.cli_ready)&&status?.proxy_reachable;
  const summary=!online?'正在读取状态':!status?.backend_ready?'后台未就绪':!status.enabled?'守护已暂停':status.error?'守护需要检查':ready?'守护已就绪':'等待应用接入';
  const item=(id,label,icon,command,args,disabled=false)=><Button role="menuitem" id={`tray-${id}`} className={`tray-item ${id==='quit'?'tray-quit':''}`} appearance="subtle" aria-busy={busy(id)} disabled={!invoke||pending||disabled} icon={<Symbol name={busy(id)?'refresh':icon} spin={busy(id)}/>} onClick={()=>action(id,command,args)}><span>{label}</span>{['dashboard','logs','settings'].includes(id)&&<Icon className="tray-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/>}</Button>;
  function keys(event){if(event.key==='Escape'){event.preventDefault();appWindow?.hide?.();return;}if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;const items=[...menu.current.querySelectorAll('[role="menuitem"]:not(:disabled)')];if(!items.length)return;const current=items.indexOf(document.activeElement),index=event.key==='Home'?0:event.key==='End'?items.length-1:event.key==='ArrowDown'?(current+1)%items.length:(current-1+items.length)%items.length;event.preventDefault();items[index].focus();}
  return <FluentProvider theme={themes[resolved]} data-theme={resolved} style={{backgroundColor:'transparent'}}><div className={`tray-surface ${active?'tray-open':''}`}><header className="tray-header"><img src="icon.svg" alt=""/><div><strong>CodexProxyTray</strong><StatusLabel color={ready?'success':status?.error?'danger':'subtle'}>{summary}</StatusLabel></div><span className="tray-version">v{version}</span></header><div className="tray-proxy"><span className={`status-indicator ${online&&status?.proxy_reachable?'active':''}`}/><span>{status?.proxy_url||'等待本地代理'}</span></div>{notice&&<div className="tray-notice" role="alert">{notice}</div>}<nav role="menu" aria-label="托盘菜单" ref={menu} tabIndex={-1} onKeyDown={keys}><div className="tray-group">{item('dashboard','打开仪表盘','dashboard','navigate_main',{page:'dashboard'})}{item('logs','运行日志','logs','navigate_main',{page:'logs'})}{item('settings','设置','settings','navigate_main',{page:'settings'})}</div><div className="tray-group">{item('launch','打开 ChatGPT','desktop','launch_codex',undefined,!writable||!status?.package_count)}{item('launch-cli','打开 Codex CLI','terminal','launch_cli',undefined,!writable||!status?.cli_ready)}</div><div className="tray-group">{item('toggle',status?.enabled?'暂停守护':'恢复守护',status?.enabled?'pause':'play','set_enabled',{enabled:!status?.enabled},!writable)}</div><div className="tray-group tray-last">{item('quit','退出 CodexProxyTray','power','quit_helper')}</div></nav></div></FluentProvider>;
}
if(typeof document!=='undefined')createRoot(document.getElementById('root'),{onUncaughtError:error=>window.reportUiError?.(error)}).render(document.documentElement.dataset.view==='tray'?<TrayApp/>:<App/>);
