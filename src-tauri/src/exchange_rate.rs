//! Display-only USD/CNY reference conversion, pinned to the ECB public feed.
//! No usage values, model IDs or account data are sent with the fixed request.
use serde::{Deserialize,Serialize};
use std::{fs,io::Read,path::Path,sync::{Mutex,OnceLock},time::Duration};

pub const SOURCE:&str="https://api.frankfurter.dev/v2/rate/usd/cny?providers=ecb";
const TTL:u64=12*60*60*1000;
const RETRY:u64=60_000;
const LIMIT:u64=4096;
static CACHE:OnceLock<Mutex<Cache>>=OnceLock::new();
#[derive(Clone,Debug,Serialize,Deserialize)]struct Quote{date:String,base:String,quote:String,rate:f64}
#[derive(Serialize,Deserialize)]struct Saved{schema:u8,source:String,fetched_at:u64,quote:Quote}
#[derive(Default)]struct Cache{loaded:bool,writable:bool,value:Option<Quote>,fetched_at:Option<u64>,last_attempt:Option<u64>,error:Option<String>}
#[derive(Clone,Debug,Serialize)]pub struct Snapshot{pub source:String,pub base:&'static str,pub quote:&'static str,pub rate:Option<f64>,pub date:Option<String>,pub fetched_at:Option<u64>,pub stale:bool,pub error:Option<String>,pub network_enabled:bool}
fn valid_date(value:&str)->bool{
    if value.len()!=10||value.as_bytes()[4]!=b'-'||value.as_bytes()[7]!=b'-'{return false}
    let number=|start,end|value.get(start..end).filter(|part|part.bytes().all(|byte|byte.is_ascii_digit())).and_then(|part|part.parse::<u32>().ok());
    let(Some(year),Some(month),Some(day))=(number(0,4),number(5,7),number(8,10))else{return false};
    if !(1970..=9999).contains(&year)||!(1..=12).contains(&month){return false}let leap=year%4==0&&(year%100!=0||year%400==0);let days=[31,if leap{29}else{28},31,30,31,30,31,31,30,31,30,31];day>0&&day<=days[month as usize-1]
}
fn valid(quote:&Quote)->bool{quote.base=="USD"&&quote.quote=="CNY"&&quote.rate.is_finite()&&quote.rate>0.0&&valid_date(&quote.date)}
fn parse(bytes:&[u8])->Result<Quote,String>{let quote:Quote=serde_json::from_slice(bytes).map_err(|_|"参考汇率响应格式无效。")?;if !valid(&quote){return Err("参考汇率币种、日期或数值无效。".into())}Ok(quote)}
fn load(cache:&mut Cache,path:&Path,now:u64){
    cache.loaded=true;cache.writable=true;
    let bytes=match fs::File::open(path){Ok(file)=>{let mut bytes=vec![];if file.take(LIMIT+1).read_to_end(&mut bytes).is_err()||bytes.len()as u64>LIMIT{cache.error=Some("本机参考汇率缓存无法读取。".into());cache.writable=false;return}bytes},Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return,Err(_)=>{cache.error=Some("本机参考汇率缓存无法读取。".into());cache.writable=false;return}};
    match serde_json::from_slice::<Saved>(&bytes){Ok(saved)if saved.schema==1&&saved.source==SOURCE&&saved.fetched_at>0&&saved.fetched_at<=now&&valid(&saved.quote)=>{cache.value=Some(saved.quote);cache.fetched_at=Some(saved.fetched_at);},_=>{cache.error=Some("本机参考汇率缓存格式无效。".into());cache.writable=false;}}
}
fn fetch(config:&crate::settings::Config)->Result<Quote,String>{
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(12)).redirect(reqwest::redirect::Policy::none()).user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION")))
        .proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"本地代理设置无效。")?).build().map_err(|_|"参考汇率查询暂不可用。")?;
    let response=client.get(SOURCE).send().map_err(|_|"无法获取参考汇率，请检查本地代理。")?;if !response.status().is_success(){return Err("参考汇率服务暂不可用，已保留上次数据。".into())}
    if response.content_length().is_some_and(|length|length>LIMIT){return Err("参考汇率响应超过允许范围。".into())}
    let mut bytes=vec![];response.take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"参考汇率响应读取失败。")?;if bytes.len()as u64>LIMIT{return Err("参考汇率响应超过允许范围。".into())}parse(&bytes)
}
fn due(cache:&Cache,now:u64)->bool{cache.fetched_at.is_none_or(|at|now<at||now-at>=TTL)&&cache.last_attempt.is_none_or(|at|now<at||now-at>=RETRY)}
pub fn read()->Result<Snapshot,String>{
    let root=crate::settings::root();let mut cache=CACHE.get_or_init(||Mutex::new(Cache::default())).lock().map_err(|_|"参考汇率缓存暂不可用。")?;
    let config=crate::settings::load(&root).map_err(|_|"联网配置无法读取，参考汇率查询已暂停。")?;let now=crate::settings::now();if !cache.loaded{load(&mut cache,&root.join("usage-exchange-v1.json"),now);}
    if config.network_enabled&&due(&cache,now){cache.last_attempt=Some(now);match fetch(&config){Ok(value)=>{let fetched_at=crate::settings::now();cache.value=Some(value.clone());cache.fetched_at=Some(fetched_at);cache.error=None;
            if !cache.writable||fs::create_dir_all(&root).is_err()||crate::settings::write_json(&root.join("usage-exchange-v1.json"),&Saved{schema:1,source:SOURCE.into(),fetched_at,quote:value}).is_err(){cache.error=Some("参考汇率已获取，但未能保存本机缓存。".into());}
        },Err(error)=>cache.error=Some(error)}}
    let now=crate::settings::now();let stale=cache.fetched_at.is_none_or(|at|now<at||now-at>=TTL);
    Ok(Snapshot{source:SOURCE.into(),base:"USD",quote:"CNY",rate:cache.value.as_ref().map(|value|value.rate),date:cache.value.as_ref().map(|value|value.date.clone()),fetched_at:cache.fetched_at,stale,error:if config.network_enabled{cache.error.clone()}else{Some("联网已关闭，参考汇率自动更新已暂停。".into())},network_enabled:config.network_enabled})
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn quote_validation_requires_the_exact_pair_positive_rate_and_calendar_date(){
        assert!(parse(br#"{"date":"2026-11-11","base":"USD","quote":"CNY","rate":7.25}"#).is_ok());
        for bytes in [br#"{"date":"2026-11-11","base":"EUR","quote":"CNY","rate":7.25}"#.as_slice(),br#"{"date":"2026-02-29","base":"USD","quote":"CNY","rate":7.25}"#,br#"{"date":"2026-11-11","base":"USD","quote":"CNY","rate":0}"#,br#"{"date":"2026-11-11","base":"USD","quote":"CNY","rate":-1}"#]{assert!(parse(bytes).is_err());}
        assert!(valid_date("2024-02-29"));assert!(!valid_date("2026-13-01"));
    }
    #[test]fn cache_ttl_and_failure_backoff_never_invent_a_rate(){let mut cache=Cache::default();assert!(due(&cache,1_000));cache.last_attempt=Some(1_000);assert!(!due(&cache,2_000));assert!(due(&cache,61_000));cache.fetched_at=Some(61_000);assert!(!due(&cache,61_000+TTL-1));assert!(due(&cache,61_000+TTL));assert!(cache.value.is_none());}
}
