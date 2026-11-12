//! Public standard text-token reference prices. The fixed catalog request never
//! contains local model IDs, usage records, session metadata or credentials.
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,fs,io::Read,path::Path,sync::{Mutex,OnceLock,atomic::{AtomicBool,Ordering},mpsc::Sender},time::{Duration,Instant}};

pub const SOURCE:&str="https://models.dev/api.json";
const TTL:u64=6*60*60*1000;
const FORCE_INTERVAL:u64=5_000;
const RETRY_INTERVAL:u64=60_000;
const MAX_CATALOG:u64=12*1024*1024;
const MAX_CACHE:u64=1024*1024;
const MAX_MODELS:usize=2048;
static CACHE:OnceLock<Mutex<Cache>>=OnceLock::new();
const SYNC_SETTINGS:&str="usage-pricing-settings-v1.json";
static SYNC:OnceLock<Mutex<SyncPreferences>>=OnceLock::new();
static AUTO_ENABLED:AtomicBool=AtomicBool::new(true);
static AUTO_RUNNING:AtomicBool=AtomicBool::new(false);
static AUTO_CHECK:Mutex<Option<Instant>>=Mutex::new(None);
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct SavedSync{schema:u8,enabled:bool}
struct SyncPreferences{enabled:bool,writable:bool,error:Option<String>}
#[derive(Clone,Serialize)]pub struct SyncSetting{pub enabled:bool}
fn load_sync(path:&Path)->SyncPreferences{
    let loaded=(||->Result<Option<SavedSync>,()>{let file=match fs::File::open(path){Ok(file)=>file,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err(())};let mut bytes=vec![];file.take(1025).read_to_end(&mut bytes).map_err(|_|())?;if bytes.len()>1024{return Err(())}let saved:SavedSync=serde_json::from_slice(&bytes).map_err(|_|())?;if saved.schema!=1{return Err(())}Ok(Some(saved))})();
    match loaded{Ok(saved)=>SyncPreferences{enabled:saved.is_none_or(|saved|saved.enabled),writable:true,error:None},Err(())=>SyncPreferences{enabled:false,writable:false,error:Some("参考价自动同步设置无法读取，已有文件已保留。".into())}}
}
fn sync_preferences()->&'static Mutex<SyncPreferences>{SYNC.get_or_init(||{let value=load_sync(&crate::settings::root().join(SYNC_SETTINGS));AUTO_ENABLED.store(value.enabled,Ordering::Release);Mutex::new(value)})}
pub fn set_sync(enabled:bool)->Result<SyncSetting,String>{
    let root=crate::settings::root();let mut preferences=sync_preferences().lock().map_err(|_|"参考价自动同步设置暂不可用。")?;if !preferences.writable{return Err(preferences.error.clone().unwrap_or_else(||"参考价自动同步设置不可写。".into()))}
    fs::create_dir_all(&root).map_err(|_|"自动同步偏好未能保存。")?;crate::settings::write_json(&root.join(SYNC_SETTINGS),&SavedSync{schema:1,enabled}).map_err(|_|"自动同步偏好未能保存。")?;
    preferences.enabled=enabled;preferences.error=None;AUTO_ENABLED.store(enabled,Ordering::Release);if enabled{if let Ok(mut checked)=AUTO_CHECK.lock(){*checked=None;}}Ok(SyncSetting{enabled})
}

#[derive(Clone,Debug,Serialize,Deserialize)]pub struct Price{pub input:f64,pub cached:f64,pub output:f64}
impl Price{fn valid(&self)->bool{[self.input,self.cached,self.output].iter().all(|value|value.is_finite()&&*value>=0.0)}}
#[derive(Clone,Debug,Default,Serialize,Deserialize)]pub struct CatalogPrice{pub name:String,pub input:Option<f64>,pub cached:Option<f64>,pub output:Option<f64>,pub status:Option<String>}
impl CatalogPrice{fn valid(&self)->bool{!self.name.is_empty()&&self.name.len()<=160&&!self.name.chars().any(char::is_control)&&self.status.as_ref().is_none_or(|value|value.len()<=32&&!value.chars().any(char::is_control))&&[self.input,self.cached,self.output].iter().flatten().all(|value|value.is_finite()&&*value>=0.0)}}
#[derive(Clone,Debug,Serialize)]pub struct Snapshot{
    pub source:String,pub fetched_at:Option<u64>,pub models:BTreeMap<String,Price>,pub catalog:BTreeMap<String,CatalogPrice>,pub error:Option<String>,pub stale:bool,pub network_enabled:bool,pub auto_sync_enabled:bool,pub next_retry_at:Option<u64>,pub throttled:bool,
}
#[derive(Serialize,Deserialize)]struct Saved{schema:u8,source:String,fetched_at:u64,models:BTreeMap<String,Price>,#[serde(default)]catalog:BTreeMap<String,CatalogPrice>}
#[derive(Default)]struct Cache{loaded:bool,fetched_at:Option<u64>,models:BTreeMap<String,Price>,catalog:BTreeMap<String,CatalogPrice>,last_attempt:Option<u64>,last_completed:Option<u64>,error:Option<String>,writable:bool}
#[derive(Debug,PartialEq,Eq)]enum Decision{Reuse,Fetch,Throttled(u64)}
fn decision(cache:&Cache,force:bool,requested_at:u64,now:u64)->Decision{
    if !force&&!cache.catalog.is_empty()&&cache.fetched_at.is_some_and(|at|at<=now&&now-at<TTL){return Decision::Reuse}
    // The mutex covers the fetch. A caller already waiting for that fetch reuses
    // its outcome rather than issuing a second forced request after the wait.
    if cache.last_completed.is_some_and(|at|at>=requested_at&&at<=now){return Decision::Reuse}
    let previous=if force{cache.last_completed.or(cache.last_attempt)}else{cache.last_attempt};
    if let Some(at)=previous.filter(|at|*at<=now){let ready_at=at.saturating_add(if force{FORCE_INTERVAL}else{RETRY_INTERVAL});if now<ready_at{return if force{Decision::Throttled(ready_at)}else{Decision::Reuse}}}
    Decision::Fetch
}
fn can_fetch(network_enabled:bool,auto_enabled:bool,force:bool)->bool{network_enabled&&(auto_enabled||force)}

fn label(value:&str)->bool{!value.is_empty()&&value.len()<=120&&value.bytes().all(|byte|byte.is_ascii_alphanumeric()||b"._-/".contains(&byte))&&!value.starts_with('/')&&!value.contains("..")}
fn valid_models(models:&BTreeMap<String,Price>)->bool{models.len()<=MAX_MODELS&&models.iter().all(|(key,value)|label(key)&&value.valid())}
fn valid_catalog(catalog:&BTreeMap<String,CatalogPrice>)->bool{catalog.len()<=MAX_MODELS&&catalog.iter().all(|(key,value)|label(key)&&!key.contains('/')&&value.valid())}
fn load(cache:&mut Cache,path:&Path,now:u64){
    cache.loaded=true;cache.writable=true;
    let bytes=match fs::File::open(path){Ok(file)=>{let mut bytes=vec![];if file.take(MAX_CACHE+1).read_to_end(&mut bytes).is_err()||bytes.len()as u64>MAX_CACHE{cache.error=Some("本机价格缓存无法读取。".into());cache.writable=false;return}bytes},Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return,Err(_)=>{cache.error=Some("本机价格缓存无法读取。".into());cache.writable=false;return}};
    match serde_json::from_slice::<Saved>(&bytes){Ok(saved)if saved.schema==1&&saved.source==SOURCE&&saved.fetched_at<=now&&(!saved.models.is_empty()||!saved.catalog.is_empty())&&valid_models(&saved.models)&&valid_catalog(&saved.catalog)=>{cache.fetched_at=Some(saved.fetched_at);cache.models=saved.models;cache.catalog=saved.catalog;},_=>{cache.error=Some("本机价格缓存格式无效。".into());cache.writable=false;}}
}
#[derive(Default,Deserialize)]#[serde(default)]struct CatalogProvider{models:BTreeMap<String,CatalogModel>}
#[derive(Deserialize)]struct Catalog{openai:CatalogProvider}
#[derive(Default,Deserialize)]#[serde(default)]struct CatalogModel{cost:Option<Cost>,modalities:Modalities,status:Option<String>,name:Option<String>,family:Option<String>,#[serde(rename="type")]kind:Option<String>}
#[derive(Default,Deserialize)]#[serde(default)]struct Cost{input:Option<f64>,output:Option<f64>,cache_read:Option<f64>}
#[derive(Default,Deserialize)]#[serde(default)]struct Modalities{output:Vec<String>}
struct ReferenceData{models:BTreeMap<String,Price>,catalog:BTreeMap<String,CatalogPrice>}
fn parse(bytes:&[u8])->Result<ReferenceData,String>{
    let catalog:Catalog=serde_json::from_slice(bytes).map_err(|_|"models.dev 价格目录格式无效或缺少 OpenAI 参考价格。")?;
    // ponytail: Codex uses the OpenAI reference catalog. Exact IDs avoid choosing
    // a reseller's same-named price or guessing aliases/service-tier multipliers.
    let provider=&catalog.openai;
    let mut models=BTreeMap::new();let mut entries=BTreeMap::new();
    for(id,model)in &provider.models{
        if !label(id)||id.contains('/')||model.kind.as_deref()==Some("embedding")||model.family.as_deref()==Some("text-embedding")||id.starts_with("text-embedding-")||model.modalities.output.len()!=1||model.modalities.output[0]!="text"{continue}
        if entries.len()>=MAX_MODELS{return Err("models.dev 参考模型超过允许范围。".into())}
        let finite=|value:Option<f64>|value.filter(|value|value.is_finite()&&*value>=0.0);let cost=model.cost.as_ref();
        let entry=CatalogPrice{name:model.name.as_ref().filter(|value|!value.is_empty()&&value.len()<=160&&!value.chars().any(char::is_control)).cloned().unwrap_or_else(||id.clone()),input:finite(cost.and_then(|value|value.input)),cached:finite(cost.and_then(|value|value.cache_read)),output:finite(cost.and_then(|value|value.output)),status:model.status.as_ref().filter(|value|value.len()<=32&&!value.chars().any(char::is_control)).cloned()};
        let complete=match(entry.input,entry.cached,entry.output){(Some(input),Some(cached),Some(output))=>Some(Price{input,cached,output}),_=>None};entries.insert(id.clone(),entry);
        let Some(price)=complete else{continue};if models.len()+2>MAX_MODELS{return Err("models.dev 参考价格超过允许范围。".into())}
        models.insert(id.clone(),price.clone());models.insert(format!("openai/{id}"),price);
    }
    if entries.is_empty(){return Err("models.dev 暂无可读取的 OpenAI 文本模型。".into())}Ok(ReferenceData{models,catalog:entries})
}
fn fetch(config:&crate::settings::Config)->Result<ReferenceData,String>{
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION")))
        .proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"本地代理设置无效。")?).build().map_err(|_|"价格同步暂不可用。")?;
    let response=client.get(SOURCE).send().map_err(|_|"无法连接 models.dev，请检查本地代理。")?;
    if !response.status().is_success(){return Err("models.dev 暂不可用，已保留现有价格。".into())}
    if response.content_length().is_some_and(|size|size>MAX_CATALOG){return Err("models.dev 价格目录超过允许范围。".into())}
    let mut bytes=vec![];response.take(MAX_CATALOG+1).read_to_end(&mut bytes).map_err(|_|"models.dev 价格目录读取失败。")?;
    if bytes.len()as u64>MAX_CATALOG{return Err("models.dev 价格目录超过允许范围。".into())}parse(&bytes)
}
fn view(cache:&Cache,network_enabled:bool,force:bool,throttled:Option<u64>)->Snapshot{
    let now=crate::settings::now();let preferences=sync_preferences().lock().unwrap_or_else(|error|error.into_inner());
    let stale=cache.catalog.is_empty()||cache.fetched_at.is_none_or(|at|now<at||now-at>=TTL);
    // This is the manual action's cooldown, including after an automatic fetch.
    // Reusing a valid catalog during the cooldown is not a synchronization error.
    let _=force;
    let next_retry_at=network_enabled.then(||throttled.or_else(||cache.last_completed.or(cache.last_attempt).map(|at|at.saturating_add(FORCE_INTERVAL)))).flatten().filter(|at|*at>now);
    let error=preferences.error.clone().or_else(||if !network_enabled{Some("联网已关闭，价格同步已暂停。".into())}else{cache.error.clone()});
    Snapshot{source:SOURCE.into(),fetched_at:cache.fetched_at,models:cache.models.clone(),catalog:cache.catalog.clone(),error,stale,network_enabled,auto_sync_enabled:preferences.enabled,next_retry_at,throttled:throttled.is_some()}
}
/// Call via spawn_blocking. Failures preserve the last successful public catalog.
pub fn read(force:bool)->Result<Snapshot,String>{
    let requested_at=crate::settings::now();let root=crate::settings::root();
    let mut cache=CACHE.get_or_init(||Mutex::new(Cache::default())).lock().map_err(|_|"价格缓存暂不可用。")?;
    let config=crate::settings::load(&root).map_err(|_|"联网配置无法读取，价格同步已暂停。")?;let now=crate::settings::now();
    if !cache.loaded{load(&mut cache,&root.join("usage-prices-v1.json"),now);}
    let auto_enabled=sync_preferences().lock().map_err(|_|"参考价自动同步设置暂不可用。")?.enabled;
    let mut throttled=None;
    if can_fetch(config.network_enabled,auto_enabled,force){match decision(&cache,force,requested_at,now){Decision::Reuse=>{},Decision::Throttled(at)=>throttled=Some(at),Decision::Fetch=>{
            cache.last_attempt=Some(now);
            match fetch(&config){Ok(reference)=>{let fetched_at=crate::settings::now();cache.models=reference.models;cache.catalog=reference.catalog;cache.fetched_at=Some(fetched_at);cache.error=None;
                let saved=Saved{schema:1,source:SOURCE.into(),fetched_at,models:cache.models.clone(),catalog:cache.catalog.clone()};let fits=serde_json::to_vec_pretty(&saved).is_ok_and(|bytes|bytes.len()as u64<=MAX_CACHE);
                if !cache.writable||!fits||fs::create_dir_all(&root).is_err()||crate::settings::write_json(&root.join("usage-prices-v1.json"),&saved).is_err(){cache.error=Some("价格已获取，但未能保存本机缓存。".into());}
            },Err(error)=>cache.error=Some(error)}
            cache.last_completed=Some(crate::settings::now());
        }}
    }
    Ok(view(&cache,config.network_enabled,force,throttled))
}
/// Reload only the one public price file; no network and no loss of last-good data.
fn apply_reload(cache:&mut Cache,mut replacement:Cache){
    if replacement.fetched_at.is_some()&&replacement.writable{replacement.last_attempt=cache.last_attempt;replacement.last_completed=cache.last_completed;*cache=replacement;}
    else{cache.loaded=true;cache.writable=replacement.writable;cache.error=replacement.error.or_else(||Some("本机参考价文件不存在，已保留内存中的可用价格。".into()));}
}
pub fn reload()->Result<Snapshot,String>{
    let root=crate::settings::root();let network_enabled=crate::settings::load(&root).is_ok_and(|config|config.network_enabled);let mut cache=CACHE.get_or_init(||Mutex::new(Cache::default())).lock().map_err(|_|"价格缓存暂不可用。")?;
    let mut replacement=Cache::default();load(&mut replacement,&root.join("usage-prices-v1.json"),crate::settings::now());
    apply_reload(&mut cache,replacement);
    let mut snapshot=view(&cache,network_enabled,false,None);snapshot.error=cache.error.clone();Ok(snapshot)
}
pub fn open_directory()->Result<(),String>{let root=crate::settings::root();fs::create_dir_all(&root).map_err(|_|"无法准备本机价格目录。")?;crate::native::open(&root.to_string_lossy()).map_err(|_|"无法打开本机价格目录。".into())}
pub fn record_auto_result(tx:&Sender<crate::worker::Message>,result:&Result<Snapshot,String>){let settings_failed=sync_preferences().lock().map_or(true,|preferences|preferences.error.is_some());if settings_failed||!result.as_ref().is_ok_and(|value|!value.network_enabled||!value.auto_sync_enabled){crate::event_log::read_state(tx,crate::event_log::ReadState::Prices,result.as_ref().map_or(true,|value|value.error.is_some()),"价格","模型参考价格自动同步未完成","模型参考价格当前可读取");}}
struct AutoRun;
impl Drop for AutoRun{fn drop(&mut self){AUTO_RUNNING.store(false,Ordering::Release);}}
/// Worker-side scheduling does no disk/network work and never waits for a fetch.
pub fn maybe_automatic(config:&crate::settings::Config,tx:&Sender<crate::worker::Message>){
    if !config.network_enabled||!AUTO_ENABLED.load(Ordering::Acquire){return}let Ok(mut checked)=AUTO_CHECK.try_lock()else{return};if checked.is_some_and(|at|at.elapsed()<Duration::from_secs(60)){return}
    if AUTO_RUNNING.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return}*checked=Some(Instant::now());drop(checked);let sender=tx.clone();
    if std::thread::Builder::new().name("usage-price-sync".into()).spawn(move||{let _run=AutoRun;let result=read(false);record_auto_result(&sender,&result);}).is_err(){AUTO_RUNNING.store(false,Ordering::Release);crate::event_log::read_state(tx,crate::event_log::ReadState::Prices,true,"价格","模型参考价格自动同步未能启动","模型参考价格当前可读取");}
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn public_prices_require_complete_original_text_rates_and_preserve_units(){
        let bytes=br#"{"openai":{"models":{"gpt-fixture":{"modalities":{"output":["text"]},"cost":{"input":1.25,"output":10,"cache_read":0.125}},"missing-cache":{"modalities":{"output":["text"]},"cost":{"input":2,"output":3}},"image":{"modalities":{"output":["image"]},"cost":{"input":1,"output":2,"cache_read":0}},"negative":{"modalities":{"output":["text"]},"cost":{"input":-1,"output":2,"cache_read":0}}}},"reseller":{"models":{"gpt-fixture":{"modalities":{"output":["text"]},"cost":{"input":"unsupported reseller schema","output":99,"cache_read":99}}}}}"#;
        let reference=parse(bytes).unwrap();let prices=reference.models;assert_eq!(prices.len(),2);assert_eq!(prices["gpt-fixture"].input,1.25);assert_eq!(prices["openai/gpt-fixture"].cached,0.125);assert!(!prices.contains_key("missing-cache"));assert_eq!(reference.catalog.len(),3);assert_eq!(reference.catalog["missing-cache"].cached,None);assert_eq!(reference.catalog["negative"].input,None);assert!(parse(br#"{"other":{"models":{}}}"#).is_err());
    }
    #[test]fn partial_and_legacy_models_remain_visible_without_guessing_prices_or_importing_embeddings(){
        let reference=parse(br#"{"openai":{"models":{"partial":{"name":"Incomplete price","modalities":{"output":["text"]},"cost":{"input":2,"output":8}},"legacy":{"status":"deprecated","modalities":{"output":["text"]},"cost":{"input":1,"cache_read":0.1,"output":5}},"embedding":{"type":"embedding","modalities":{"output":["text"]},"cost":{"input":1,"cache_read":0,"output":0}}}}}"#).unwrap();
        assert_eq!(reference.catalog.len(),2);assert!(reference.catalog["partial"].cached.is_none());assert_eq!(reference.catalog["legacy"].status.as_deref(),Some("deprecated"));assert!(reference.models.contains_key("legacy"));assert!(!reference.models.contains_key("partial"));assert!(!reference.catalog.contains_key("embedding"));assert!(valid_catalog(&reference.catalog));
    }
    #[test]fn invalid_cache_is_retained_and_missing_cache_is_writable(){
        let path=std::env::temp_dir().join(format!("codex-price-fixture-{}-{}",std::process::id(),crate::settings::now()));fs::write(&path,b"invalid").unwrap();let mut cache=Cache::default();load(&mut cache,&path,10);assert!(!cache.writable);assert!(cache.models.is_empty());assert_eq!(fs::read(&path).unwrap(),b"invalid");fs::remove_file(&path).unwrap();let mut missing=Cache::default();load(&mut missing,&path,10);assert!(missing.writable);
        fs::write(&path,serde_json::to_vec(&serde_json::json!({"schema":1,"source":SOURCE,"fetched_at":10,"models":{"fixture":{"input":1,"cached":0.1,"output":5}}})).unwrap()).unwrap();let mut previous=Cache::default();load(&mut previous,&path,20);assert!(previous.writable);assert!(previous.models.contains_key("fixture"));assert!(previous.catalog.is_empty());assert_eq!(decision(&previous,false,20,20),Decision::Fetch,"Old complete prices survive while the full catalog upgrades automatically");fs::remove_file(path).unwrap();
    }
    #[test]fn manual_sync_bypasses_ttl_and_automatic_backoff_but_throttling_and_waiters_are_explicit(){
        let mut cache=Cache{fetched_at:Some(1_000),catalog:BTreeMap::from([("fixture".into(),CatalogPrice{name:"fixture".into(),..CatalogPrice::default()})]),..Cache::default()};assert_eq!(decision(&cache,false,2_000,2_000),Decision::Reuse);assert_eq!(decision(&cache,true,2_000,2_000),Decision::Fetch);
        cache.fetched_at=None;cache.last_attempt=Some(1_000);cache.last_completed=Some(1_500);cache.error=Some("synthetic failure".into());
        assert_eq!(decision(&cache,true,3_000,3_000),Decision::Throttled(6_500));assert_eq!(decision(&cache,true,7_000,7_000),Decision::Fetch);assert_eq!(decision(&cache,false,7_000,7_000),Decision::Reuse);
        assert_eq!(decision(&cache,true,1_200,1_600),Decision::Reuse,"A request waiting for the same in-flight fetch reuses its result");assert_eq!(decision(&cache,true,1_600,1_600),Decision::Throttled(6_500));
        assert_eq!(decision(&cache,true,100,100),Decision::Fetch,"Clock correction must not be mistaken for a concurrent completed fetch");
    }
    #[test]fn auto_sync_defaults_on_persists_off_and_preserves_unknown_settings(){
        let path=std::env::temp_dir().join(format!("codex-pricing-pref-fixture-{}-{}",std::process::id(),crate::settings::now()));let initial=load_sync(&path);assert!(initial.enabled&&initial.writable);
        fs::write(&path,serde_json::to_vec(&SavedSync{schema:1,enabled:false}).unwrap()).unwrap();let disabled=load_sync(&path);assert!(!disabled.enabled);assert!(disabled.writable);
        fs::write(&path,b"invalid").unwrap();let invalid=load_sync(&path);assert!(!invalid.enabled&&!invalid.writable);assert_eq!(fs::read(&path).unwrap(),b"invalid");fs::remove_file(path).unwrap();
    }
    #[test]fn auto_off_keeps_manual_sync_available_but_never_overrides_the_global_network_gate(){assert!(!can_fetch(true,false,false));assert!(can_fetch(true,false,true));assert!(!can_fetch(false,true,true));assert!(!can_fetch(false,true,false));}
    #[test]fn malformed_reload_preserves_the_last_good_prices_until_the_file_is_repaired(){
        let mut cache=Cache{fetched_at:Some(10),models:BTreeMap::from([("fixture".into(),Price{input:1.0,cached:0.1,output:5.0})]),writable:true,last_attempt:Some(9),..Cache::default()};
        apply_reload(&mut cache,Cache{writable:false,error:Some("invalid fixture".into()),..Cache::default()});assert_eq!(cache.models["fixture"].input,1.0);assert_eq!(cache.fetched_at,Some(10));assert!(!cache.writable);assert!(cache.error.is_some());
        apply_reload(&mut cache,Cache{fetched_at:Some(20),models:BTreeMap::from([("fixture".into(),Price{input:2.0,cached:0.2,output:8.0})]),writable:true,..Cache::default()});assert_eq!(cache.models["fixture"].input,2.0);assert!(cache.error.is_none());assert!(cache.writable);assert_eq!(cache.last_attempt,Some(9));
    }
}
