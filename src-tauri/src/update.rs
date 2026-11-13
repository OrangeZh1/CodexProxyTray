use serde::{Deserialize,Serialize};
use std::{fs,io::Read,sync::{Mutex,OnceLock,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use crate::settings::{self,Config};

pub const SOURCE:&str="https://github.com/OrangeZh1/CodexProxyTray";
pub const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const MANIFEST:&str="https://github.com/OrangeZh1/CodexProxyTray/releases/latest/download/CodexProxyTray-update.json";
const VERSION:&str=env!("CARGO_PKG_VERSION");
const GET_ATTEMPTS:u8=3;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum SendFailure{Connect,Timeout,Other}
fn network_allowed()->Result<(),String>{
    let config=settings::load(&settings::root()).map_err(|_|"更新配置无法读取，请检查设置。".to_owned())?;
    if config.network_enabled{Ok(())}else{Err("联网检测已关闭，更新请求已停止。".into())}
}
fn retry_send<T>(mut allowed:impl FnMut()->Result<(),String>,mut send:impl FnMut()->Result<T,SendFailure>,mut wait:impl FnMut(Duration))->Result<T,String>{
    for attempt in 1..=GET_ATTEMPTS{
        allowed()?;
        match send(){Ok(response)=>{allowed()?;return Ok(response)},Err(kind)=>{
            allowed()?;
            if kind==SendFailure::Other||attempt==GET_ATTEMPTS{return Err(match kind{SendFailure::Connect=>format!("未能建立 GitHub 更新连接（已尝试 {attempt} 次），请稍后重试。"),SendFailure::Timeout=>format!("GitHub 更新请求超时（已尝试 {attempt} 次），请稍后重试。"),SendFailure::Other=>"GitHub 更新请求未能完成，请重试。".into()})}
            wait(Duration::from_millis(if attempt==1{200}else{500}));allowed()?;
        }}
    }unreachable!()
}
pub(crate)fn official_get(client:&reqwest::blocking::Client,url:&str)->Result<reqwest::blocking::Response,String>{
    let parsed=reqwest::Url::parse(url).map_err(|_|"官方更新地址无效。")?;
    if !official_url(&parsed){return Err("官方更新地址不受支持。".into())}
    // Only GET send failures before a response are retried. Status, body, checksum and disk handling stay with the caller.
    retry_send(network_allowed,||client.get(url).send().map_err(|error|if error.is_timeout(){SendFailure::Timeout}else if error.is_connect(){SendFailure::Connect}else{SendFailure::Other}),thread::sleep)
}
fn official_url(url:&reqwest::Url)->bool{url.scheme()=="https"&&url.host_str()==Some("github.com")&&url.username().is_empty()&&url.password().is_none()&&url.port_or_known_default()==Some(443)&&url.query().is_none()&&url.fragment().is_none()&&url.path().starts_with("/OrangeZh1/CodexProxyTray/releases/")}
pub(crate)fn official_redirect()->reqwest::redirect::Policy{reqwest::redirect::Policy::custom(|attempt|{let url=attempt.url();if attempt.previous().len()<5&&url.scheme()=="https"&&url.username().is_empty()&&url.password().is_none()&&url.port_or_known_default()==Some(443)&&matches!(url.host_str(),Some("github.com"|"release-assets.githubusercontent.com")){attempt.follow()}else{attempt.error("Unexpected update redirect")}})}
#[derive(Clone,Serialize)] pub struct Update {pub status:String,pub current:String,pub latest:Option<String>,pub url:String,pub notes:String,pub checked_at:u64}
#[derive(Deserialize)] struct Manifest {schema:u32,product:String,version:String,notes:String}
#[derive(Clone,Serialize,Deserialize)] #[serde(default)] pub struct UpdateState {pub checking:bool,pub checked_at:Option<u64>,pub last_success_at:Option<u64>,pub status:String,pub latest:Option<String>,pub notes:String,pub error:Option<String>}
impl Default for UpdateState {fn default()->Self{Self{checking:false,checked_at:None,last_success_at:None,status:"unchecked".into(),latest:None,notes:String::new(),error:None}}}
#[derive(Serialize,Deserialize)] struct SavedState {schema:u32,state:UpdateState}
struct State {view:UpdateState,writable:bool}
static STATE:OnceLock<Mutex<State>>=OnceLock::new();
static CACHE:Mutex<Option<(Instant,String,Result<Update,String>)>>=Mutex::new(None);
static AUTO:Mutex<(bool,Option<Instant>)>=Mutex::new((false,None));
static AUTO_RUNNING:AtomicBool=AtomicBool::new(false);
fn state()->&'static Mutex<State>{STATE.get_or_init(||{
    let path=settings::root().join("update-v1.json");
    let loaded=(||{let metadata=fs::metadata(&path).map_err(|_|())?;if metadata.len()>128*1024{return Err(())}let saved:SavedState=serde_json::from_slice(&fs::read(&path).map_err(|_|())?).map_err(|_|())?;
        if saved.schema!=1||saved.state.checked_at.is_some_and(|at|at>8_640_000_000_000_000)||saved.state.last_success_at.is_some_and(|at|at>8_640_000_000_000_000)||!matches!(saved.state.status.as_str(),"unchecked"|"available"|"current"|"unpublished"|"error")||saved.state.latest.as_ref().is_some_and(|v|crate::core::version_parts(v).is_none()){return Err(())}Ok(saved.state)})();
    let writable=loaded.is_ok()||!path.exists();let mut view=loaded.unwrap_or_default();view.checking=false;
    if view.last_success_at.is_none()&&matches!(view.status.as_str(),"available"|"current"|"unpublished"){view.last_success_at=view.checked_at;}
    if matches!(view.status.as_str(),"available"|"current") {view.status=match &view.latest {Some(latest)=>if crate::core::newer_version(latest,VERSION){"available"}else{"current"},None=>"unchecked"}.into();}
    Mutex::new(State{view,writable})
})}
pub fn snapshot()->UpdateState {state().lock().unwrap_or_else(|e|e.into_inner()).view.clone()}
fn record(result:&Result<Update,String>){
    let mut state=state().lock().unwrap_or_else(|e|e.into_inner());apply_result(&mut state.view,result,settings::now());
    if state.writable {let root=settings::root();let _=fs::create_dir_all(&root);let _=settings::write_json(&root.join("update-v1.json"),&SavedState{schema:1,state:state.view.clone()});}
}
fn apply_result(view:&mut UpdateState,result:&Result<Update,String>,at:u64){
    view.checking=false;view.checked_at=Some(at);
    match result {Ok(update)=>{view.status=update.status.clone();view.latest=update.latest.clone();view.notes=update.notes.clone();view.error=None;view.checked_at=Some(update.checked_at);view.last_success_at=Some(update.checked_at);},Err(error)=>{view.status="error".into();view.error=Some(error.clone());}}
}
pub fn check()->Result<Update,String>{check_with(true)}
fn check_with(manual:bool)->Result<Update,String> {
    let requested=Instant::now();
    let config=settings::load(&settings::root()).map_err(|_|"更新配置无法读取，请检查设置。".to_owned())?;
    if !config.network_enabled {return Err("已关闭联网检查更新。".into())}
    let mut cache=CACHE.lock().map_err(|_|"更新检查暂不可用。")?;
    if let Some((at,proxy,result))=&*cache {if cache_valid(manual,requested,*at,result.is_ok(),proxy==&config.proxy_url){return result.clone();}}
    state().lock().unwrap_or_else(|e|e.into_inner()).view.checking=true;
    let result=fetch(&config);record(&result);*cache=Some((Instant::now(),config.proxy_url,result.clone()));result
}
fn cache_valid(manual:bool,requested:Instant,finished:Instant,success:bool,same_proxy:bool)->bool{
    // Only callers using the same proxy share the request they waited for; explicit retries bypass older results.
    same_proxy&&(finished>=requested||!manual&&finished.elapsed()<Duration::from_secs(if success{900}else{60}))
}
pub fn maybe_automatic(config:&Config){
    let mut schedule=AUTO.lock().unwrap_or_else(|e|e.into_inner());
    if !config.network_enabled||!config.auto_check_updates {schedule.0=false;return;}
    if !schedule.0 {schedule.0=true;schedule.1=None;}
    if schedule.1.is_some_and(|at|at.elapsed()<Duration::from_secs(u64::from(config.update_interval_hours)*3600)){return;}
    if AUTO_RUNNING.swap(true,Ordering::AcqRel){return;}
    schedule.1=Some(Instant::now());thread::spawn(||{let _=check_with(false);AUTO_RUNNING.store(false,Ordering::Release);});
}
fn fetch(config:&Config)->Result<Update,String>{
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(12)).user_agent(format!("CodexProxyTray/{VERSION}"))
        .proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"代理设置无效。")?).redirect(official_redirect()).build().map_err(|_|"更新检查暂不可用。")?;
    let response=official_get(&client,MANIFEST)?;
    if response.status()==reqwest::StatusCode::NOT_FOUND {return Ok(Update{status:"unpublished".into(),current:VERSION.into(),latest:None,url:RELEASES.into(),notes:"尚未发布 CodexProxyTray 正式安装包。".into(),checked_at:settings::now()})}
    if response.status()==reqwest::StatusCode::FORBIDDEN||response.status()==reqwest::StatusCode::TOO_MANY_REQUESTS{return Err("GitHub 暂时限制访问，请稍后重试。".into())}
    if !response.status().is_success(){return Err("GitHub 暂不可用，请稍后重试。".into())}
    let mut bytes=Vec::new();response.take(65537).read_to_end(&mut bytes).map_err(|_|"发布信息读取失败。")?;
    if bytes.len()>65536{return Err("发布信息无效，请查看 GitHub 发布页。".into())}
    let data:Manifest=serde_json::from_slice(&bytes).map_err(|_|"发布信息无效，请查看 GitHub 发布页。")?;
    if data.schema!=1||data.product!="CodexProxyTray"||crate::core::version_parts(&data.version).is_none(){return Err("发布信息与当前应用不匹配，请查看 GitHub 发布页。".into())}
    Ok(Update{status:if crate::core::newer_version(&data.version,VERSION){"available"}else{"current"}.into(),current:VERSION.into(),latest:Some(data.version),url:RELEASES.into(),notes:data.notes.chars().take(16000).collect(),checked_at:settings::now()})
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn official_get_retries_only_transport_before_response_and_honors_network_disable(){
        use std::cell::Cell;
        let calls=Cell::new(0);let delays=std::cell::RefCell::new(Vec::new());let status=retry_send(||Ok(()),||{calls.set(calls.get()+1);if calls.get()<3{Err(SendFailure::Connect)}else{Ok(403)}},|delay|delays.borrow_mut().push(delay)).unwrap();
        assert_eq!(status,403);assert_eq!(calls.get(),3);assert_eq!(*delays.borrow(),vec![Duration::from_millis(200),Duration::from_millis(500)]);
        for(kind,expected)in[(SendFailure::Other,1),(SendFailure::Timeout,3)]{let calls=Cell::new(0);assert!(retry_send::<()>(||Ok(()),||{calls.set(calls.get()+1);Err(kind)},|_|{}).is_err());assert_eq!(calls.get(),expected);}
        let calls=Cell::new(0);let waits=Cell::new(0);let stopped=retry_send::<()>(||if calls.get()==0{Ok(())}else{Err("network disabled".into())},||{calls.set(calls.get()+1);Err(SendFailure::Connect)},|_|waits.set(waits.get()+1));assert_eq!(stopped.unwrap_err(),"network disabled");assert_eq!(calls.get(),1);assert_eq!(waits.get(),0);
        let enabled=Cell::new(true);let calls=Cell::new(0);assert!(retry_send::<()>(||if enabled.get(){Ok(())}else{Err("network disabled".into())},||{calls.set(calls.get()+1);Err(SendFailure::Connect)},|_|enabled.set(false)).is_err());assert_eq!(calls.get(),1);
        // Once a response exists, even a failing HTTP status/body is returned once, never replayed.
        let calls=Cell::new(0);let response=retry_send(||Ok(()),||{calls.set(calls.get()+1);Ok((503,Err::<(),_>("partial body")))},|_|panic!("response must not retry")).unwrap();assert_eq!(response.0,503);assert!(response.1.is_err());assert_eq!(calls.get(),1);
        assert!(official_url(&reqwest::Url::parse(MANIFEST).unwrap()));
        for url in ["http://github.com/OrangeZh1/CodexProxyTray/releases/latest/download/x","https://github.com.evil.test/OrangeZh1/CodexProxyTray/releases/x","https://name@github.com/OrangeZh1/CodexProxyTray/releases/x","https://github.com/other/repo/releases/x","https://github.com:444/OrangeZh1/CodexProxyTray/releases/x"]{assert!(!official_url(&reqwest::Url::parse(url).unwrap()));}
    }
    #[test]fn failed_check_keeps_last_verified_release_and_manual_retry_bypasses_cache(){
        let mut view=UpdateState::default();
        let success=Update{status:"available".into(),current:VERSION.into(),latest:Some("9.0.0".into()),url:RELEASES.into(),notes:"release notes".into(),checked_at:10};
        apply_result(&mut view,&Ok(success),10);view.checking=true;apply_result(&mut view,&Err("timeout".into()),20);
        assert!(!view.checking);assert_eq!(view.status,"error");assert_eq!(view.latest.as_deref(),Some("9.0.0"));assert_eq!(view.notes,"release notes");assert_eq!(view.last_success_at,Some(10));assert_eq!(view.checked_at,Some(20));
        let at=Instant::now();let before=at-Duration::from_secs(1);
        assert!(!cache_valid(true,at,before,false,true));assert!(!cache_valid(true,at,before,true,true));assert!(cache_valid(false,at,before,false,true));assert!(cache_valid(true,before,at,false,true));
        // A request started after saving another proxy must not inherit the older in-flight result.
        for success in [true,false]{assert!(!cache_valid(true,before,at,success,false));assert!(!cache_valid(false,before,at,success,false));}
        apply_result(&mut view,&Ok(Update{status:"current".into(),current:VERSION.into(),latest:Some(VERSION.into()),url:RELEASES.into(),notes:String::new(),checked_at:30}),30);assert!(view.error.is_none());assert_eq!(view.last_success_at,Some(30));
    }
}
