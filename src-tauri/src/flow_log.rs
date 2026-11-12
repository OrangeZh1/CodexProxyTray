//! Opt-in local connection history and process traffic samples. No packet data,
//! DNS queries, proxy control API, rules, conversation data or remote uploads.
use std::{collections::{BTreeMap},fs,io::Read,path::Path,net::SocketAddr,sync::{Mutex,OnceLock},time::{Duration,Instant}};
use serde::{Serialize,Deserialize};
use crate::{runtime_details::ConnectionSnapshot,settings,traffic::{TrafficSnapshot,TrafficState}};
const LIMIT:usize=300;
#[derive(Clone,Serialize,Deserialize)]pub struct Entry{pub first_at:u64,pub last_at:u64,pub pid:u32,pub created:String,pub process:String,pub local:String,pub remote:String,pub present:bool}
#[derive(Clone,Serialize,Deserialize)]pub struct Sample{pub at:u64,pub receive_bps:Option<f64>,pub send_bps:Option<f64>,pub received_bytes:Option<u64>,pub sent_bytes:Option<u64>}
#[derive(Clone,Default,Serialize,Deserialize)]#[serde(default)]pub struct Snapshot{pub enabled:bool,pub entries:Vec<Entry>,pub samples:Vec<Sample>,pub sampled_at:Option<u64>,pub error:Option<String>,pub truncated:bool}
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct Preference{schema:u32,enabled:bool}
#[derive(Serialize,Deserialize)]struct Saved{schema:u32,entries:Vec<Entry>,samples:Vec<Sample>}
struct State{view:Snapshot,writable:bool,last:Option<Instant>,last_saved:Option<Instant>}
static STATE:OnceLock<Mutex<State>>=OnceLock::new();
fn state()->&'static Mutex<State>{STATE.get_or_init(||{
    let root=settings::root();let mut view=Snapshot::default();let mut writable=true;
    let preference=(||->Result<bool,String>{let Some(bytes)=read_optional(&root.join("flow-log-settings-v1.json"),1024)?else{return Ok(false)};let value:Preference=serde_json::from_slice(&bytes).map_err(|_|"连接记录设置无法读取，原文件已保留。")?;if value.schema!=1{return Err("连接记录设置版本无效。".into())}Ok(value.enabled)})();
    match preference{Ok(enabled)=>view.enabled=enabled,Err(error)=>{writable=false;view.error=Some(error);}}
    let history=(||->Result<Option<Saved>,String>{let Some(bytes)=read_optional(&root.join("logs/flow-v1.json"),1024*1024)?else{return Ok(None)};let saved:Saved=serde_json::from_slice(&bytes).map_err(|_|"连接历史无法读取，原文件已保留。")?;if saved.schema!=1||saved.entries.len()>LIMIT||saved.samples.len()>LIMIT||!saved.entries.iter().all(valid){return Err("连接历史格式或范围无效，原文件已保留。".into())}Ok(Some(saved))})();
    match history{Ok(Some(saved))=>{view.entries=saved.entries;for row in &mut view.entries{row.present=false;}view.samples=saved.samples;},Ok(None)=>{},Err(error)=>{writable=false;view.enabled=false;view.error=Some(error);}}
    Mutex::new(State{view,writable,last:None,last_saved:None})
})}
fn read_optional(path:&Path,limit:u64)->Result<Option<Vec<u8>>,String>{let file=match fs::File::open(path){Ok(file)=>file,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err("连接记录不可读取，原文件已保留。".into())};let mut bytes=vec![];file.take(limit+1).read_to_end(&mut bytes).map_err(|_|"连接记录读取失败。")?;if bytes.len()as u64>limit{return Err("连接记录超过安全读取范围，原文件已保留。".into())}Ok(Some(bytes))}
fn valid(row:&Entry)->bool{row.pid>0&&row.first_at<=row.last_at&&row.last_at<=8_640_000_000_000_000&&row.process.len()<=100&&row.created.parse::<u64>().is_ok()&&row.local.parse::<SocketAddr>().is_ok()&&row.remote.parse::<SocketAddr>().is_ok()}
pub fn snapshot()->Snapshot{state().lock().unwrap_or_else(|e|e.into_inner()).view.clone()}
pub fn set_enabled(enabled:bool)->Result<Snapshot,String>{let mut state=state().lock().map_err(|_|"连接日志状态暂不可用。")?;if !state.writable{return Err(state.view.error.clone().unwrap_or_else(||"连接日志不可写。".into()))}settings::write_json(&settings::root().join("flow-log-settings-v1.json"),&Preference{schema:1,enabled})?;state.view.enabled=enabled;if !enabled{for row in &mut state.view.entries{row.present=false;}}state.last=None;Ok(state.view.clone())}
fn merge(view:&mut Snapshot,connections:ConnectionSnapshot,traffic:&TrafficSnapshot){
    let now=connections.sampled_at;let mut by_key=view.entries.drain(..).map(|mut row|{row.present=false;((row.pid,row.created.clone(),row.local.clone(),row.remote.clone()),row)}).collect::<BTreeMap<_,_>>();
    for row in connections.rows{let entry=Entry{first_at:now,last_at:now,pid:row.pid,created:row.process_created.clone(),process:row.process_label,local:row.local,remote:row.remote,present:true};if !valid(&entry){continue}let key=(entry.pid,entry.created.clone(),entry.local.clone(),entry.remote.clone());by_key.entry(key).and_modify(|old|{old.last_at=now;old.present=true;}).or_insert(entry);}
    view.entries=by_key.into_values().collect();view.entries.sort_by_key(|row|std::cmp::Reverse(row.last_at));view.truncated=connections.truncated||view.entries.len()>LIMIT;view.entries.truncate(LIMIT);view.sampled_at=Some(now);
    if view.samples.last().is_none_or(|sample|sample.at!=traffic.timestamp_ms)&&traffic.timestamp_ms>0{let collecting=traffic.state==TrafficState::Collecting;let rate=|value:Option<f64>|value.filter(|n|collecting&&n.is_finite()&&*n>=0.0);view.samples.push(Sample{at:traffic.timestamp_ms,receive_bps:rate(traffic.receive_bps),send_bps:rate(traffic.send_bps),received_bytes:traffic.received_bytes.filter(|_|collecting),sent_bytes:traffic.sent_bytes.filter(|_|collecting)});if view.samples.len()>LIMIT{view.samples.remove(0);}}
}
fn persist(state:&mut State){let root=settings::root();let saved=Saved{schema:1,entries:state.view.entries.clone(),samples:state.view.samples.clone()};if fs::create_dir_all(root.join("logs")).is_err()||settings::write_json(&root.join("logs/flow-v1.json"),&saved).is_err(){state.view.error=Some("连接记录尚未写入磁盘，已有历史已保留。".into());}else{state.last_saved=Some(Instant::now());}}
pub fn flush(){if let Ok(mut state)=state().lock(){if state.writable&&state.view.enabled{persist(&mut state);}}}
pub fn maybe_sample(traffic:&TrafficSnapshot){
    let Ok(mut state)=state().try_lock()else{return};if !state.view.enabled||!state.writable||state.last.is_some_and(|last|last.elapsed()<Duration::from_secs(5)){return}state.last=Some(Instant::now());
    match crate::runtime_details::connections(){Ok(connections)=>{merge(&mut state.view,connections,traffic);state.view.error=None;if state.last_saved.is_none_or(|last|last.elapsed()>=Duration::from_secs(30)){persist(&mut state);}},Err(error)=>state.view.error=Some(error)}
}
#[cfg(test)]mod tests{use super::*;#[test]fn endpoint_history_is_validated_and_pid_reuse_is_distinct(){let mut row=Entry{first_at:1,last_at:2,pid:3,created:"100".into(),process:"codex.exe".into(),local:"127.0.0.1:1000".into(),remote:"127.0.0.1:7890".into(),present:true};assert!(valid(&row));row.remote="https://private.example/path".into();assert!(!valid(&row));row.remote="[::1]:7890".into();assert!(valid(&row));row.created="not-a-filetime".into();assert!(!valid(&row));}}
