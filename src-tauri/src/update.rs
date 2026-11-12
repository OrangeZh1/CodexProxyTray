use serde::{Deserialize,Serialize};
use std::{fs,io::Read,sync::{Mutex,OnceLock,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use crate::settings::{self,Config};

pub const SOURCE:&str="https://github.com/OrangeZh1/CodexProxyTray";
pub const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const MANIFEST:&str="https://github.com/OrangeZh1/CodexProxyTray/releases/latest/download/CodexProxyTray-update.json";
const VERSION:&str=env!("CARGO_PKG_VERSION");
#[derive(Clone,Serialize)] pub struct Update {pub status:String,pub current:String,pub latest:Option<String>,pub url:String,pub notes:String,pub checked_at:u64}
#[derive(Deserialize)] struct Manifest {schema:u32,product:String,version:String,notes:String}
#[derive(Clone,Serialize,Deserialize)] #[serde(default)] pub struct UpdateState {pub checking:bool,pub checked_at:Option<u64>,pub status:String,pub latest:Option<String>,pub notes:String,pub error:Option<String>}
impl Default for UpdateState {fn default()->Self{Self{checking:false,checked_at:None,status:"unchecked".into(),latest:None,notes:String::new(),error:None}}}
#[derive(Serialize,Deserialize)] struct SavedState {schema:u32,state:UpdateState}
struct State {view:UpdateState,writable:bool}
static STATE:OnceLock<Mutex<State>>=OnceLock::new();
static CACHE:Mutex<Option<(Instant,Result<Update,String>)>>=Mutex::new(None);
static AUTO:Mutex<(bool,Option<Instant>)>=Mutex::new((false,None));
static AUTO_RUNNING:AtomicBool=AtomicBool::new(false);
fn state()->&'static Mutex<State>{STATE.get_or_init(||{
    let path=settings::root().join("update-v1.json");
    let loaded=(||{let metadata=fs::metadata(&path).map_err(|_|())?;if metadata.len()>128*1024{return Err(())}let saved:SavedState=serde_json::from_slice(&fs::read(&path).map_err(|_|())?).map_err(|_|())?;
        if saved.schema!=1||saved.state.checked_at.is_some_and(|at|at>8_640_000_000_000_000)||!matches!(saved.state.status.as_str(),"unchecked"|"available"|"current"|"unpublished"|"error")||saved.state.latest.as_ref().is_some_and(|v|crate::core::version_parts(v).is_none()){return Err(())}Ok(saved.state)})();
    let writable=loaded.is_ok()||!path.exists();let mut view=loaded.unwrap_or_default();view.checking=false;
    if matches!(view.status.as_str(),"available"|"current") {view.status=match &view.latest {Some(latest)=>if crate::core::newer_version(latest,VERSION){"available"}else{"current"},None=>"unchecked"}.into();}
    Mutex::new(State{view,writable})
})}
pub fn snapshot()->UpdateState {state().lock().unwrap_or_else(|e|e.into_inner()).view.clone()}
fn record(result:&Result<Update,String>){
    let mut state=state().lock().unwrap_or_else(|e|e.into_inner());state.view.checking=false;state.view.checked_at=Some(settings::now());
    match result {Ok(update)=>{state.view.status=update.status.clone();state.view.latest=update.latest.clone();state.view.notes=update.notes.clone();state.view.error=None;state.view.checked_at=Some(update.checked_at);},Err(error)=>{state.view.status="error".into();state.view.latest=None;state.view.notes.clear();state.view.error=Some(error.clone());}}
    if state.writable {let root=settings::root();let _=fs::create_dir_all(&root);let _=settings::write_json(&root.join("update-v1.json"),&SavedState{schema:1,state:state.view.clone()});}
}
pub fn check()->Result<Update,String> {
    let config=settings::load(&settings::root()).map_err(|_|"更新配置无法读取，请检查设置。".to_owned())?;
    if !config.network_enabled {return Err("已关闭联网检查更新。".into())}
    let mut cache=CACHE.lock().map_err(|_|"更新检查暂不可用。")?;
    if let Some((at,result))=&*cache {if at.elapsed()<Duration::from_secs(if result.is_ok(){900}else{60}){return result.clone();}}
    state().lock().unwrap_or_else(|e|e.into_inner()).view.checking=true;
    let result=fetch(&config);record(&result);*cache=Some((Instant::now(),result.clone()));result
}
pub fn maybe_automatic(config:&Config){
    let mut schedule=AUTO.lock().unwrap_or_else(|e|e.into_inner());
    if !config.network_enabled||!config.auto_check_updates {schedule.0=false;return;}
    if !schedule.0 {schedule.0=true;schedule.1=None;}
    if schedule.1.is_some_and(|at|at.elapsed()<Duration::from_secs(u64::from(config.update_interval_hours)*3600)){return;}
    if AUTO_RUNNING.swap(true,Ordering::AcqRel){return;}
    schedule.1=Some(Instant::now());thread::spawn(||{let _=check();AUTO_RUNNING.store(false,Ordering::Release);});
}
fn fetch(config:&Config)->Result<Update,String>{
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(12)).user_agent(format!("CodexProxyTray/{VERSION}"))
        .proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"代理设置无效。")?).build().map_err(|_|"更新检查暂不可用。")?;
    let response=client.get(MANIFEST).send().map_err(|_|"无法连接 GitHub，请检查本地代理后重试。")?;
    if response.status()==reqwest::StatusCode::NOT_FOUND {return Ok(Update{status:"unpublished".into(),current:VERSION.into(),latest:None,url:RELEASES.into(),notes:"尚未发布 CodexProxyTray 正式安装包。".into(),checked_at:settings::now()})}
    if response.status()==reqwest::StatusCode::FORBIDDEN||response.status()==reqwest::StatusCode::TOO_MANY_REQUESTS{return Err("GitHub 暂时限制访问，请稍后重试。".into())}
    if !response.status().is_success(){return Err("GitHub 暂不可用，请稍后重试。".into())}
    let mut bytes=Vec::new();response.take(65537).read_to_end(&mut bytes).map_err(|_|"发布信息读取失败。")?;
    if bytes.len()>65536{return Err("发布信息无效，请查看 GitHub 发布页。".into())}
    let data:Manifest=serde_json::from_slice(&bytes).map_err(|_|"发布信息无效，请查看 GitHub 发布页。")?;
    if data.schema!=1||data.product!="CodexProxyTray"||crate::core::version_parts(&data.version).is_none(){return Err("发布信息与当前应用不匹配，请查看 GitHub 发布页。".into())}
    Ok(Update{status:if crate::core::newer_version(&data.version,VERSION){"available"}else{"current"}.into(),current:VERSION.into(),latest:Some(data.version),url:RELEASES.into(),notes:data.notes.chars().take(16000).collect(),checked_at:settings::now()})
}
