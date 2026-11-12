// QA-only IPC transport. Serves the actual ui/ files and never contacts native services.
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = process.env.QA_DIST?resolve(process.env.QA_DIST):resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
const port = Number(process.env.QA_PORT || 4174);
if (!Number.isInteger(port) || port < 1 || port > 65535 || port === 4173) throw Error('QA_PORT must be a separate port; 4173 is reserved for the real preview.');
// Optional read-only public catalog evidence. No private source is discovered,
// fetched or uploaded, and no configured browser price override is changed.
async function loadPublicPriceCatalog(path){
  if(!path)return null;
  const info=await stat(path);if(!info.isFile()||info.size>2*1024*1024)throw Error('QA_PRICE_CATALOG must be a bounded public catalog snapshot.');
  const bytes=await readFile(path);if(bytes.length>2*1024*1024)throw Error('QA_PRICE_CATALOG exceeds the allowed size.');
  let value;try{value=JSON.parse(bytes.toString('utf8'));}catch{throw Error('QA_PRICE_CATALOG is not valid public snapshot JSON.');}
  const label=name=>typeof name==='string'&&/^[a-zA-Z0-9][a-zA-Z0-9._/-]{0,119}$/.test(name),rate=price=>typeof price==='number'&&Number.isFinite(price)&&price>=0;
  const entries=object=>object&&typeof object==='object'&&!Array.isArray(object)?Object.entries(object):null,models=entries(value?.models),catalog=entries(value?.catalog);
  if(value?.source!=='https://models.dev/api.json'||!Number.isSafeInteger(value.fetched_at)||value.fetched_at<=0||value.fetched_at>8_640_000_000_000_000||!models||models.length>2048||!catalog?.length||catalog.length>2048||!models.every(([id,price])=>label(id)&&price&&['input','cached','output'].every(key=>rate(price[key])))||!catalog.every(([id,entry])=>label(id)&&entry&&typeof entry.name==='string'&&entry.name.length<=200&&['input','cached','output'].every(key=>entry[key]===null||rate(entry[key]))&&(entry.status===null||typeof entry.status==='string'&&entry.status.length<=40)))throw Error('QA_PRICE_CATALOG has an invalid public snapshot schema.');
  const requested=Array.isArray(value.demoModels)?value.demoModels:[],demoModels=[...new Set(requested.filter(id=>label(id)&&Object.hasOwn(value.catalog,id)&&Object.hasOwn(value.models,id)))].slice(0,12);
  if(!demoModels.length)demoModels.push(...catalog.filter(([id])=>Object.hasOwn(value.models,id)).map(([id])=>id).slice(0,12));
  if(!demoModels.length)throw Error('QA_PRICE_CATALOG needs at least one complete public demo rate.');
  return{source:value.source,fetched_at:value.fetched_at,models:value.models,catalog:value.catalog,demoModels,provenance:{kind:'public_models_dev_snapshot',read_only:true,native_sync_verified:false}};
}
const publicPriceCatalog=await loadPublicPriceCatalog(process.env.QA_PRICE_CATALOG);
const priceCatalogInfo=publicPriceCatalog?{mode:'read_only_public_snapshot',source:publicPriceCatalog.source,fetched_at:publicPriceCatalog.fetched_at,models:Object.keys(publicPriceCatalog.catalog).length,priced_aliases:Object.keys(publicPriceCatalog.models).length,demoModels:publicPriceCatalog.demoModels,provenance:publicPriceCatalog.provenance}:{mode:'synthetic'};
const commands = ['get_status', 'frontend_ready', 'refresh_status', 'set_enabled', 'save_proxy', 'save_system', 'choose_cli_workdir', 'launch_codex', 'launch_cli', 'check_updates', 'check_connectivity', 'enable_traffic', 'disable_traffic', 'install_traffic_service', 'uninstall_traffic_service', 'open_release', 'open_source', 'open_logs','capture_desktop_target','manage_desktop','check_cli_updates','update_cli','uninstall_cli','get_runtime_connections','get_runtime_memory','get_usage','get_usage_prices','set_usage_price_sync','open_usage_prices_dir','reload_usage_prices','get_usage_exchange_rate','record_ui_event'];
const defaults = new Map(), queued = new Map(), history = [];
let online = true, epoch = 0, revision = 0, requestId = 0;
let pricesFetchedAt=Date.now(),manualPricesAt=0,priceAutoSyncEnabled=true;
const freshState = () => ({ revision: ++revision, enabled: true, guard_ready: true,guard_uptime_ms:125000,updated_at:Date.now(),proxy_url: 'http://127.0.0.1:7890', proxy_reachable: true, package_count: 1, package_version: 'QA', codex_running: true, restart_required: true, cli_detected: true, cli_ready: true, autostart: false, launch_target:'none',cli_workdir:'',network_enabled:true,auto_check_updates:false,update_interval_hours:6,update:{checking:false,checked_at:null,status:'unchecked',latest:null,notes:'',error:null},backend_ready: true, read_only: false, logs: [{ at: Date.now(), level: 'info',category:'QA',details:'仅测试，不修改真实应用或配置。',message: 'QA 模拟 IPC，无真实代理操作。' }] });
const emptyTraffic=()=>({state:'disabled',timestamp_ms:Date.now(),received_bytes:null,sent_bytes:null,receive_bps:null,send_bps:null,target_count:0,lost_events:null,lost_buffers:null,message:null});
const initialize=()=>({...freshState(),started_at:Date.now()-300000,desktop_pid:4242,tcp_connections:14,proxy_latency_ms:0.32,probe_history:Array.from({length:60},(_,index)=>({at:Date.now()-(59-index)*2000,latency_ms:0.3+(Math.sin(index*1.8)+1)*0.07})),connectivity_enabled:true,connectivity:{checking:false,status:'available',checked_at:Date.now(),latency_ms:175,ip:'192.0.2.1',country:'JP',http_status:200,error:null},traffic_enabled:false,traffic_service:{state:'not_installed',version:null,error:null},traffic:emptyTraffic()});
let state = initialize();
let logId=100;
const exchangeRate={source:'https://api.frankfurter.dev/v2/rate/usd/cny?providers=ecb',base:'USD',quote:'CNY',rate:7.25,date:'2026-11-11',fetched_at:Date.now(),stale:false,error:null,network_enabled:true,qa_provenance:'synthetic_fx'};
const uiEventMessages={appearance_changed:['外观','已调整界面主题'],window_pin_on:['窗口','已启用窗口置顶'],window_pin_off:['窗口','已取消窗口置顶'],window_pin_failed:['窗口','窗口置顶操作未完成'],usage_settings_changed:['用量','已调整用量显示设置'],usage_sync_started:['用量','开始手动同步用量记录'],usage_sync_success:['用量','手动用量同步已结束'],usage_sync_failed:['用量','手动用量同步未完成'],prices_sync_started:['价格','开始同步模型参考价格'],prices_sync_success:['价格','模型参考价格同步已结束'],prices_sync_failed:['价格','模型参考价格同步未完成'],currency_changed:['用量','已切换费用显示币种']};
function appendLog(category,message,level='info'){state.logs.push({id:++logId,at:Date.now(),level,category,message,details:'QA 模拟事件，不修改真实应用、配置或会话。'});if(state.logs.length>200)state.logs.splice(0,state.logs.length-200);state.revision=++revision;}
let demo=false;
function seedDemo(){
  const now=Date.now();state.traffic_enabled=true;state.traffic_service={state:'running',version:'0.1.0',error:null};state.restart_required=false;
  state.traffic_history=Array.from({length:60},(_,index)=>demoPoint(now-(59-index)*2500));
  const point=state.traffic_history.at(-1);state.traffic={state:'collecting',timestamp_ms:point.at,receive_bps:point.receive_bps,send_bps:point.send_bps,received_bytes:point.received_bytes,sent_bytes:point.sent_bytes,target_count:8,lost_events:0,lost_buffers:0,partial:false,message:null};
  state.helper_memory_bytes=48.6*1048576;state.cli_management={detected:true,supported:true,source:'npm_user_global',current:'0.100.0',latest:'0.101.0',target_token:'demo-cli-install',checked_at:now,operation:null,progress:'',error:null,reason:null};state.desktop_operation={busy:false,stage:'idle'};
  state.logs=[{id:1,at:now-90000,level:'info',category:'守护',message:'代理守护已就绪',details:'ChatGPT 与 Codex CLI 已接入当前本地代理。\n此记录为演示数据。'},{id:2,at:now-60000,level:'info',category:'流量',message:'目标进程流量采集已启动',details:'已识别 8 个目标进程。\n只计数字节，不读取通信内容。'},{id:3,at:now-30000,level:'info',category:'检测',message:'ChatGPT 网页探测完成',details:'HTTP 200\n完整 HTTP 耗时：175 ms\n此记录为演示数据，不代表真实网络。'}];state.revision=++revision;
}
function demoPoint(at){const phase=at/7000;return{at,receive_bps:Math.round((1.5+Math.sin(phase)*0.55+Math.sin(phase*2.7)*0.2)*1048576),send_bps:Math.round((65+Math.sin(phase*1.6)*25)*1024),received_bytes:Math.floor(2.3*1073741824+at%10000000*20),sent_bytes:Math.floor(74*1048576+at%10000000)};}
function advanceDemo(){if(!demo||!state.traffic_enabled)return;const point=demoPoint(Date.now());state.traffic_history=[...(state.traffic_history||[]).filter(item=>point.at-item.at<150000),point].slice(-60);Object.assign(state.traffic,{timestamp_ms:point.at,receive_bps:point.receive_bps,send_bps:point.send_bps,received_bytes:point.received_bytes,sent_bytes:point.sent_bytes});if(state.enabled)state.guard_uptime_ms+=Math.max(0,Date.now()-state.updated_at);state.updated_at=Date.now();state.revision=++revision;}
function extraUsageRecords(value){
  if(!Array.isArray(value)||value.length>20)throw Error('qa_usage_extra_records must contain at most 20 synthetic metadata records.');
  const label=item=>typeof item==='string'&&item.length>0&&item.length<=120&&!/[\u0000-\u001f\u007f]/.test(item),token=item=>Number.isSafeInteger(item)&&item>=0&&item<=1_000_000_000_000;
  return value.map(item=>{
    if(!item||typeof item!=='object'||Array.isArray(item))throw Error('Invalid synthetic usage record.');
    const provider=item.provider===undefined?'openai':item.provider,source_kind=item.source_kind??'token_usage_record',input_tokens=item.input_tokens,cached_input_tokens=item.cached_input_tokens??0,cache_write_input_tokens=item.cache_write_input_tokens??0,output_tokens=item.output_tokens,reasoning_output_tokens=item.reasoning_output_tokens??0,total_tokens=input_tokens+output_tokens;
    if(!Number.isSafeInteger(item.at)||item.at<0||item.at>8_640_000_000_000_000||!label(item.model)||(provider!==null&&!label(provider))||!['token_count','token_usage_record'].includes(source_kind)||![input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens].every(token)||cached_input_tokens+cache_write_input_tokens>input_tokens||reasoning_output_tokens>output_tokens||(item.total_tokens!==undefined&&item.total_tokens!==total_tokens))throw Error('Invalid synthetic usage metadata.');
    return{at:item.at,model:item.model,provider,source_kind,input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens,request_count:1,duration_ms:null,ttft_ms:null,output_tokens_per_second:null,timing_estimated:false,timing_source:null,http_status:null,estimated_cost_usd:null,missing_price:true};
  });
}
function demoUsage({startMs=Date.now()-86400000,endMs=Date.now()}){
  // Synthetic history has a fixed sixty-day horizon, including the "all" view.
  // Never enumerate dates from the Unix epoch or inspect real Codex sessions.
  const sampleStart=Math.max(startMs,endMs-60*86400000);
  const fields=['input_tokens','cached_input_tokens','cache_write_input_tokens','output_tokens','reasoning_output_tokens','total_tokens'];
  const summary=()=>({...Object.fromEntries(fields.map(key=>[key,0])),request_count:0,estimated_cost_usd:null,missing_price:true});
  const total=summary(),models=new Map(),series=[],records=[],step=endMs-startMs<=172800000?3600000:86400000;
  for(let start=Math.floor((sampleStart+28800000)/step)*step-28800000;start<endMs;start+=step){
    const index=Math.floor((start+28800000)/step);
    const bucket={start_ms:start,end_ms:start+step,...summary(),models:[]},perModel=new Map();
    const count=Math.max(2,Math.round(7+Math.sin(index*.7)*4+(index*7)%5));
    for(let j=0;j<count;j++){
      const input=200000+(index*17237+j*13931)%240000,output=800+(index*31+j*437)%2200,model=publicPriceCatalog?publicPriceCatalog.demoModels[(index+j)%publicPriceCatalog.demoModels.length]:j%3?'gpt-6.1-sol':'gpt-6.1-terra';
      const row={at:Math.floor(start+step*(j+.5)/count),model,provider:'openai',source_kind:j%2?'token_count':'token_usage_record',duration_ms:j%2?null:Math.round(1200+output*.7),ttft_ms:j%2?null:210+j*13,output_tokens_per_second:j%2?null:output*1000/(Math.round(1200+output*.7)-210-j*13),timing_estimated:j%2===0,timing_source:j%2?null:'codex_rollout_estimate',http_status:j%2?null:j%4?429:200,input_tokens:input,cached_input_tokens:Math.floor(input*.92),cache_write_input_tokens:0,output_tokens:output,reasoning_output_tokens:Math.floor(output*.15),total_tokens:input+output,estimated_cost_usd:null,missing_price:true};
      if(row.at<sampleStart||row.at>=endMs)continue;
      records.push(row);if(!models.has(model))models.set(model,{model,...summary()});if(!perModel.has(model))perModel.set(model,{model,...summary()});
      for(const target of [total,bucket,models.get(model),perModel.get(model)]){for(const key of fields)target[key]+=row[key];target.request_count++;}
    }
    bucket.models=[...perModel.values()];series.push(bucket);
  }
  for(const extra of state.qa_usage_extra_records||[]){
    if(extra.at<sampleStart||extra.at>=endMs)continue;const row={...extra},bucket=series.find(item=>row.at>=item.start_ms&&row.at<item.end_ms);if(!bucket)continue;
    records.push(row);if(!models.has(row.model))models.set(row.model,{model:row.model,...summary()});let bucketModel=bucket.models.find(item=>item.model===row.model);if(!bucketModel){bucketModel={model:row.model,...summary()};bucket.models.push(bucketModel);}
    for(const target of [total,bucket,models.get(row.model),bucketModel]){for(const key of fields)target[key]+=row[key];target.request_count++;}
  }
  const providers=new Map();for(const row of records){if(!providers.has(row.provider))providers.set(row.provider,{provider:row.provider,...summary(),models:[]});const group=providers.get(row.provider);let model=group.models.find(item=>item.model===row.model);if(!model){model={model:row.model,...summary()};group.models.push(model);}for(const target of [group,model]){for(const key of fields)target[key]+=row[key];target.request_count++;}}
  return{start_ms:startMs,end_ms:endMs,updated_at:Date.now(),source_available:true,scan_pending:false,partial:false,files_scanned:26,unreadable_files:0,skipped_records:0,duplicate_records:0,partial_tail_files:0,summary:total,models:[...models.values()],providers:[...providers.values()],series,records:records.sort((left,right)=>right.at-left.at).slice(0,500),records_truncated:records.length>500,request_count_kind:'deduplicated_usage_records',scope:'演示用量数据；只聚合模型与 token 元数据。',notes:[]};
}
const view = () => ({ online, state, price_catalog:{...priceCatalogInfo,auto_sync_enabled:priceAutoSyncEnabled}, defaults: Object.fromEntries(defaults), queued: Object.fromEntries(queued), history });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const fault = (message, name = 'Error') => Object.assign(Error(message), { name });
const json = (res, status, data) => { if (!res.destroyed) res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' }).end(JSON.stringify(data)); };

async function body(req) {
  const chunks = []; let size = 0;
  for await (const chunk of req) { size += chunk.length; if (size > 65536) throw Error('QA request too large'); chunks.push(chunk); }
  return JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}');
}

function control(input) {
  if (input.reset) { epoch++; online = true; defaults.clear(); queued.clear(); history.length = 0; state = initialize();pricesFetchedAt=Date.now();manualPricesAt=0;priceAutoSyncEnabled=true; }
  if (typeof input.online === 'boolean' && input.online !== online) { online = input.online; epoch++; }
  if (input.clearRules) { defaults.clear(); queued.clear(); }
  if(typeof input.demo==='boolean'){demo=input.demo;if(demo)seedDemo();}
  if (input.state && typeof input.state==='object' && !Array.isArray(input.state)) {const patch={...input.state};if(Object.hasOwn(patch,'qa_usage_extra_records'))patch.qa_usage_extra_records=extraUsageRecords(patch.qa_usage_extra_records);Object.assign(state,patch);state.revision=++revision; }
  if (input.rule) {
    const { command, delayMs = 0, outcome = 'success', once = true } = input.rule;
    if (!commands.includes(command) || !Number.isInteger(delayMs) || delayMs < 0 || delayMs > 30000 || !['success', 'error', 'abort'].includes(outcome) || typeof once !== 'boolean') throw Error('Invalid QA rule');
    const rule = { delayMs, outcome };
    if (once) { const list = queued.get(command) || []; if (list.length >= 20) throw Error('QA rule queue is full'); list.push(rule); queued.set(command, list); }
    else defaults.set(command, rule);
  }
}

function resultFor(command, args) {
  if(command==='frontend_ready')return null;
  if(command==='set_usage_price_sync'){if(typeof args.enabled!=='boolean')throw Error('Invalid QA automatic price setting');priceAutoSyncEnabled=args.enabled;appendLog('价格',args.enabled?'演示模型价格自动同步已开启':'演示模型价格自动同步已关闭');return{enabled:priceAutoSyncEnabled};}
  if(command==='open_usage_prices_dir'){appendLog('价格','演示价格目录打开请求已记录');return null;}
  if(command==='reload_usage_prices'){const snapshot=resultFor('get_usage_prices',{force:false});appendLog('价格',state.qa_prices_bad_cache?'演示本地价格文件无效，已保留原价格':'演示本地价格已重新读取',state.qa_prices_bad_cache?'warning':'info');return{...snapshot,error:state.qa_prices_bad_cache?'演示本地价格文件格式无效，已保留现有价格。':null,next_retry_at:null};}
  if(command==='choose_cli_workdir'){appendLog('CLI',state.qa_picker_cancel?'演示工作目录选择已取消':'演示工作目录选择已完成');return state.qa_picker_cancel?null:'C:\\QA\\CodexWorkspace';}
  if(command==='record_ui_event'){if(typeof args.event!=='string'||!Object.hasOwn(uiEventMessages,args.event))throw Error('Unknown QA UI event');const [category,message]=uiEventMessages[args.event];appendLog(category,message,args.event.endsWith('_failed')?'warning':'info');return null;}
  if(command==='get_usage')return demoUsage(args);
  if(command==='get_usage_exchange_rate')return{...exchangeRate,...(state.qa_fx_unavailable?{rate:null,date:null,fetched_at:null,error:'演示汇率暂不可用。'}:{}),...(state.network_enabled===false?{error:'演示联网已关闭，保留模拟汇率。'}:{}),network_enabled:state.network_enabled!==false};
  // Explicitly synthetic rates: the loopback fixture never fetches models.dev.
  if(command==='get_usage_prices'){
    if(publicPriceCatalog){if(args.force)appendLog('定价','模型参考价使用只读公开演示缓存','warning');return{...publicPriceCatalog,models:structuredClone(publicPriceCatalog.models),catalog:structuredClone(publicPriceCatalog.catalog),error:args.force?'此演示使用只读公开价格缓存，未执行原生联网同步。':state.network_enabled===false?'演示联网已关闭，保留公开缓存价格。':null,stale:Date.now()-publicPriceCatalog.fetched_at>=6*3600000,network_enabled:state.network_enabled!==false,auto_sync_enabled:priceAutoSyncEnabled,next_retry_at:null};}
    const now=Date.now(),network=state.network_enabled!==false,throttled=args.force&&manualPricesAt&&now-manualPricesAt<5000;
    if(args.force&&network&&!throttled){manualPricesAt=now;pricesFetchedAt=now;}
    if(args.force)appendLog('定价',!network?'演示价格同步因关闭联网而暂停':throttled?'演示价格同步等待重试':'演示模型参考价已同步',!network||throttled?'warning':'info');
    return{source:'https://models.dev/api.json',models:{'gpt-6.1-sol':{input:1,cached:.1,output:5},'gpt-6.1-terra':{input:2,cached:.2,output:8}},fetched_at:pricesFetchedAt,error:!network?'演示联网已关闭，保留缓存价格。':throttled?'演示同步过于频繁，请稍后重试。':null,stale:false,network_enabled:network,auto_sync_enabled:priceAutoSyncEnabled,next_retry_at:throttled?manualPricesAt+5000:null};
  }
  if(command==='capture_desktop_target')return {pid:4242,created_filetime:'134125000000000001',package_family:'OpenAI.Codex_2p2nqsd0c76g0'};
  if(command==='manage_desktop'){state.codex_running=!!args.restart;state.restart_required=false;appendLog('应用',args.restart?'演示 ChatGPT 重启已完成':'演示 ChatGPT 关闭已完成');return{pid:4242,restart_requested:!!args.restart};}
  if(command==='check_cli_updates'){state.cli_management={...state.cli_management,latest:'0.101.0',checked_at:Date.now(),operation:null,error:null};appendLog('CLI','演示 Codex CLI 更新检查已完成');return structuredClone(state.cli_management);}
  if(command==='update_cli'){state.cli_management={...state.cli_management,current:state.cli_management.latest,operation:null,progress:'演示更新完成'};appendLog('CLI','演示 Codex CLI 更新已完成');return structuredClone(state.cli_management);}
  if(command==='uninstall_cli'){state.cli_detected=false;state.cli_ready=false;state.cli_management={detected:false,supported:false,source:'not_installed',current:null,latest:null,target_token:null,operation:null,reason:'未检测到 Windows Codex CLI。'};appendLog('CLI','演示 Codex CLI 卸载已完成');return structuredClone(state.cli_management);}
  if(command==='get_runtime_connections')return{sampled_at:Date.now(),matched_total:3,target_count:8,row_limit:200,truncated:false,scope_note:'演示连接；正式版本只展示已验证目标进程当前的 TCP 连接，不写入日志或历史。',rows:[{pid:4242,process_label:'ChatGPT',local:'127.0.0.1:54212',remote:'127.0.0.1:7890',state:'ESTABLISHED'},{pid:4243,process_label:'ChatGPT',local:'192.0.2.10:54214',remote:'203.0.113.8:443',state:'ESTABLISHED'},{pid:5190,process_label:'Codex CLI',local:'127.0.0.1:54301',remote:'127.0.0.1:7890',state:'ESTABLISHED'}]};
  if(command==='get_runtime_memory')return{sampled_at:Date.now(),working_set_bytes:146*1048576,private_bytes:111*1048576,peak_working_set_bytes:179*1048576,scope_note:'助手主进程及其 WebView2 子进程；工作集相加可能重复计算共享页。演示数据。',rows:[{pid:1000,role:'helper',process_label:'CodexProxyTray',working_set_bytes:48.6*1048576,private_bytes:35*1048576},{pid:1001,role:'webview2',process_label:'WebView2 渲染进程',working_set_bytes:78*1048576,private_bytes:62*1048576},{pid:1002,role:'webview2',process_label:'WebView2 GPU 进程',working_set_bytes:19.4*1048576,private_bytes:14*1048576}]};
  if(command==='check_connectivity'){state.connectivity={checking:false,checked_at:Date.now(),status:'available',latency_ms:252,ip:'192.0.2.1',country:'JP',http_status:200,error:null};return structuredClone(state.connectivity);}
  if(command==='install_traffic_service'){state.traffic_service={state:'stopped',version:'0.1.0',error:null};appendLog('流量','演示流量辅助服务安装已完成');return structuredClone(state.traffic_service);}
  if(command==='uninstall_traffic_service'){state.traffic_service={state:'not_installed',version:null,error:null};state.traffic_enabled=false;state.traffic=emptyTraffic();appendLog('流量','演示流量辅助服务卸载已完成');return structuredClone(state.traffic_service);}
  if(command==='enable_traffic'){if(state.traffic_service.state==='not_installed')throw Error('QA 模拟：请先安装辅助服务。');state.traffic_enabled=true;state.traffic_service.state='running';state.traffic={state:'collecting',timestamp_ms:Date.now(),received_bytes:4096,sent_bytes:1024,receive_bps:512,send_bps:128,target_count:2,lost_events:0,lost_buffers:0,message:'QA 模拟，不启动真实采集器或 UAC。'};appendLog('流量','演示目标进程流量统计已开启');return structuredClone(state.traffic);}
  if(command==='disable_traffic'){state.traffic_enabled=false;state.traffic={state:'disabled',timestamp_ms:Date.now(),received_bytes:null,sent_bytes:null,receive_bps:null,send_bps:null,target_count:0,lost_events:null,lost_buffers:null,message:null};appendLog('流量','演示目标进程流量统计已关闭');return structuredClone(state.traffic);}
  if (command === 'check_updates') {state.update={checking:false,checked_at:Date.now(),status:'available',latest:'0.2.0',notes:'QA 模拟发布说明。',error:null};appendLog('更新','演示助手版本检查已完成');return { status: 'available', current: '0.1.0', latest: '0.2.0', url: 'https://github.com/OrangeZh1/CodexProxyTray/releases', notes: 'QA 模拟发布说明。' };}
  if (command === 'get_status' || command === 'refresh_status') {advanceDemo();return structuredClone(state);}
  if (command === 'set_enabled') { const now=Date.now();state.guard_uptime_ms=args.enabled?0:state.guard_uptime_ms+Math.max(0,now-state.updated_at);state.updated_at=now;state.enabled=!!args.enabled;state.guard_ready=state.enabled; }
  if (command === 'save_proxy') { state.proxy_url = String(args.proxyUrl); state.restart_required = state.codex_running; }
  if (command === 'save_system') {for(const [input,field] of Object.entries({autostart:'autostart',launchTarget:'launch_target',cliWorkdir:'cli_workdir',networkEnabled:'network_enabled',connectivityEnabled:'connectivity_enabled',autoCheckUpdates:'auto_check_updates',updateIntervalHours:'update_interval_hours'})){if(args[input]!==undefined&&args[input]!==null)state[field]=args[input];}}
  if (command === 'launch_codex') {state.codex_running=true;state.restart_required=state.enabled;}
  appendLog('演示',({set_enabled:state.enabled?'代理守护已恢复':'代理守护已暂停',save_proxy:'代理设置已应用',save_system:'偏好设置已保存',launch_codex:'已请求打开 ChatGPT',launch_cli:'已请求打开 Codex CLI',open_source:'已模拟打开项目源码',open_release:'已模拟打开发布页面',open_logs:'已模拟打开日志目录'})[command]||'演示操作已完成');
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
  } catch (error) { const failed={manage_desktop:['应用','演示 ChatGPT 管理未完成'],choose_cli_workdir:['CLI','演示工作目录选择未完成'],check_cli_updates:['CLI','演示 Codex CLI 更新检查未完成'],update_cli:['CLI','演示 Codex CLI 更新未完成'],uninstall_cli:['CLI','演示 Codex CLI 卸载未完成'],set_usage_price_sync:['价格','演示自动价格同步设置未完成'],open_usage_prices_dir:['价格','演示价格目录打开请求未完成'],reload_usage_prices:['价格','演示本地价格重新读取未完成'],get_usage_prices:['定价','演示模型参考价同步未完成']}[command];if(generation===epoch&&failed&&(command!=='get_usage_prices'||args.force))appendLog(failed[0],failed[1]+(error.name==='AbortError'?'（模拟取消）':''),'warning');row.error = { name: error.name, message: error.message }; throw error; }
  finally { row.pending = false; row.finishedAt = Date.now(); }
}

const bridge = `window.__TAURI__={core:{invoke:async(command,args={})=>{const response=await fetch('/__qa/invoke',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({command,args})});const reply=await response.json();if(!reply.ok){if(reply.error.name==='AbortError')throw new DOMException(reply.error.message,'AbortError');const error=new Error(reply.error.message);error.name=reply.error.name;throw error;}return reply.value;}}};${process.env.QA_WATCH==='1'?`let assetRevision;setInterval(async()=>{try{const next=await(await fetch('/__qa/assets')).text();if(assetRevision&&assetRevision!==next)location.reload();assetRevision=next;}catch{}},1000);`:''}`;
const demoBadge=publicPriceCatalog?'演示用量与汇率 · 价格来自只读公开缓存 · 原生同步未验证':'演示数据 · 汇率为模拟值 · 不影响真实软件';
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
    if (url.pathname === '/__qa/assets' && process.env.QA_WATCH==='1') return json(res,200,await Promise.all(['app.js','app.css'].map(async file=>(await stat(resolve(root,file))).mtimeMs)));
    if (url.pathname === '/__qa/bridge.js' || url.pathname === '/__qa/control.js') return res.writeHead(200, { 'Content-Type': 'text/javascript; charset=utf-8', 'Cache-Control': 'no-store' }).end(url.pathname.endsWith('bridge.js') ? bridge : controls);
    if (url.pathname === '/__qa' || url.pathname === '/__qa/') return res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' }).end(controlHtml);
    const path = resolve(root, '.' + decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname));
    if (!path.startsWith(root + sep)) return res.writeHead(404).end();
    let data = await readFile(path);
    if (extname(path) === '.html') data = data.toString().replace('<script type="module"', '<script src="/__qa/bridge.js"></script><script type="module"').replace('<body>', `<body><div style="position:fixed;z-index:99999;left:10px;bottom:8px;max-width:calc(100vw - 20px);background:#9f2929;color:white;padding:3px 8px;border-radius:4px;font:11px/1.5 system-ui;pointer-events:none">${demoBadge}</div>`);
    res.writeHead(200, { 'Content-Type': types[extname(path)] || 'application/octet-stream', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' }).end(data);
  } catch (error) { json(res, url.pathname.startsWith('/__qa/') ? 400 : 404, { error: error.message }); }
});
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { epoch++; server.closeAllConnections(); server.close(() => process.exit(0)); });
if(process.env.QA_DEMO==='1'){demo=true;seedDemo();}
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({ pid: process.pid, url: `http://127.0.0.1:${port}`, controls: `http://127.0.0.1:${port}/__qa`, purpose: 'QA simulated IPC only' })));
