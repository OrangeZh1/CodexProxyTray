use serde::Serialize;
use std::{io::Read,sync::{Mutex,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use crate::settings::{self,Config};

#[derive(Clone,Serialize)]pub struct Connectivity {pub checking:bool,pub checked_at:Option<u64>,pub status:String,pub latency_ms:Option<f64>,pub ip:Option<String>,pub country:Option<String>,pub http_status:Option<u16>,pub error:Option<String>}
impl Default for Connectivity{fn default()->Self{Self{checking:false,checked_at:None,status:"unchecked".into(),latency_ms:None,ip:None,country:None,http_status:None,error:None}}}
struct Store{view:Option<Connectivity>,last:Option<Instant>,scheduled:Option<Instant>,proxy:String}
static STORE:Mutex<Store>=Mutex::new(Store{view:None,last:None,scheduled:None,proxy:String::new()});
static CHECK:Mutex<()>=Mutex::new(());
static RUNNING:AtomicBool=AtomicBool::new(false);
pub fn snapshot()->Connectivity{STORE.lock().unwrap_or_else(|e|e.into_inner()).view.clone().unwrap_or_default()}
pub fn maybe_automatic(config:&Config){
    let mut store=STORE.lock().unwrap_or_else(|e|e.into_inner());
    if !config.network_enabled||!config.connectivity_enabled {let mut view=Connectivity::default();view.status="disabled".into();store.view=Some(view);store.last=None;store.scheduled=None;return;}
    if store.proxy==config.proxy_url&&store.scheduled.is_some_and(|at|at.elapsed()<Duration::from_secs(120)){return;}
    if RUNNING.swap(true,Ordering::AcqRel){return;}
    store.scheduled=Some(Instant::now());drop(store);thread::spawn(||{let _=check();RUNNING.store(false,Ordering::Release);});
}
pub fn check()->Result<Connectivity,String>{
    let _lock=CHECK.lock().map_err(|_|"连通检测暂不可用。")?;
    let config=settings::load(&settings::root()).map_err(|_|"连通检测配置无法读取。")?;
    if !config.network_enabled||!config.connectivity_enabled{return Err("联网连通检测已关闭。".into())}
    {let mut store=STORE.lock().unwrap_or_else(|e|e.into_inner());if store.proxy==config.proxy_url&&store.last.is_some_and(|at|at.elapsed()<Duration::from_secs(15))&&store.view.as_ref().is_some_and(|v|v.checked_at.is_some()&&!v.checking){return Ok(store.view.clone().unwrap())}let mut view=store.view.clone().unwrap_or_default();view.checking=true;store.view=Some(view);}
    let start=Instant::now();let result=probe(&config,start);let mut view=match result{Ok(view)=>view,Err(error)=>{let mut view=Connectivity::default();view.checked_at=Some(settings::now());view.status="error".into();view.error=Some(error);view}};
    view.checking=false;
    let current=settings::load(&settings::root()).ok();
    if current.as_ref().is_some_and(|c|c.network_enabled&&c.connectivity_enabled&&c.proxy_url==config.proxy_url){let mut store=STORE.lock().unwrap_or_else(|e|e.into_inner());store.proxy=config.proxy_url;store.last=Some(Instant::now());store.view=Some(view.clone());}
    Ok(view)
}
fn probe(config:&Config,start:Instant)->Result<Connectivity,String>{
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none()).user_agent(format!("CodexProxyTray/{} connectivity-check",env!("CARGO_PKG_VERSION"))).proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"代理地址无效。")?).build().map_err(|_|"无法创建连通检测客户端。")?;
    let response=client.get("https://chatgpt.com/cdn-cgi/trace").send().map_err(|_|"经当前代理入口连接 ChatGPT 失败或超时。")?;
    let code=response.status().as_u16();let mut bytes=Vec::new();response.take(8193).read_to_end(&mut bytes).map_err(|_|"ChatGPT 探测响应读取失败。")?;
    let mut view=Connectivity::default();view.checked_at=Some(settings::now());view.http_status=Some(code);view.latency_ms=Some(start.elapsed().as_secs_f64()*1000.0);
    if code==200&&bytes.len()<=8192{
        let(ip,country)=crate::core::network_trace(&String::from_utf8_lossy(&bytes));
        if ip.is_some(){view.status="available".into();view.ip=ip;view.country=country;}else{view.status="error".into();view.error=Some("探测响应格式无法识别，请稍后重试。".into());}
    }else{view.status=if matches!(code,401|403|429){"blocked"}else{"error"}.into();view.error=Some(if code==200{"探测响应超过读取上限。".into()}else{format!("网页探测返回 HTTP {code}；不代表账号或对话可用性。")});}
    Ok(view)
}
