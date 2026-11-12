//! Read-only local Codex usage metadata. No network, database, auth/config file or
//! conversation field is retained. Input/cache and output/reasoning overlap.
use serde::{Deserialize,Serialize};
use std::{collections::{BTreeMap,HashMap,HashSet,hash_map::DefaultHasher},fs::{self,File,Metadata},hash::{Hash,Hasher},io::{BufRead,BufReader,Read,Seek,SeekFrom},path::{Path,PathBuf},sync::{Mutex,OnceLock},time::UNIX_EPOCH};

const DAY:u64=86_400_000;
const MAX_LINE:usize=128*1024;
const READ_BUDGET:u64=32*1024*1024;
const FILE_BUDGET:u64=4*1024*1024;
const MAX_FILES:usize=10_000;
// ponytail: keep at most 100k recent metadata events in memory; larger histories
// need a persisted metadata-only index instead of retaining conversation files.
const MAX_EVENTS:usize=100_000;
const MAX_SAFE:u64=9_007_199_254_740_991;
static CACHE:OnceLock<Mutex<Cache>>=OnceLock::new();

#[derive(Clone,Copy,Debug,Default,Hash,PartialEq,Eq,Serialize)]
pub struct Tokens{pub input_tokens:u64,pub cached_input_tokens:u64,pub cache_write_input_tokens:u64,pub output_tokens:u64,pub reasoning_output_tokens:u64,pub total_tokens:u64}
impl Tokens{
    fn valid(self)->bool{self.cached_input_tokens.checked_add(self.cache_write_input_tokens).is_some_and(|cached|cached<=self.input_tokens)&&self.reasoning_output_tokens<=self.output_tokens&&self.input_tokens.checked_add(self.output_tokens)==Some(self.total_tokens)&&self.total_tokens<=MAX_SAFE}
    fn positive(self)->bool{self.total_tokens>0}
    fn difference(self,previous:Self)->Option<Self>{let value=Self{input_tokens:self.input_tokens.checked_sub(previous.input_tokens)?,cached_input_tokens:self.cached_input_tokens.checked_sub(previous.cached_input_tokens)?,cache_write_input_tokens:self.cache_write_input_tokens.checked_sub(previous.cache_write_input_tokens)?,output_tokens:self.output_tokens.checked_sub(previous.output_tokens)?,reasoning_output_tokens:self.reasoning_output_tokens.checked_sub(previous.reasoning_output_tokens)?,total_tokens:self.total_tokens.checked_sub(previous.total_tokens)?};value.valid().then_some(value)}
    fn add(&mut self,value:Self)->bool{let Some(total)=self.total_tokens.checked_add(value.total_tokens).filter(|value|*value<=MAX_SAFE)else{return false};self.input_tokens+=value.input_tokens;self.cached_input_tokens+=value.cached_input_tokens;self.cache_write_input_tokens+=value.cache_write_input_tokens;self.output_tokens+=value.output_tokens;self.reasoning_output_tokens+=value.reasoning_output_tokens;self.total_tokens=total;true}
}
#[derive(Clone,Debug,Default,Serialize)]
pub struct Summary{#[serde(flatten)]pub tokens:Tokens,pub request_count:u64,pub estimated_cost_usd:Option<f64>,pub missing_price:bool}
impl Summary{fn add(&mut self,value:Tokens)->bool{if !self.tokens.add(value){return false}self.request_count+=1;self.missing_price=true;true}}
#[derive(Clone,Debug,Serialize)]pub struct Bucket{pub start_ms:u64,pub end_ms:u64,#[serde(flatten)]pub summary:Summary,pub models:Vec<ModelUsage>}
#[derive(Clone,Debug,Serialize)]pub struct ModelUsage{pub model:String,#[serde(flatten)]pub summary:Summary}
#[derive(Clone,Copy,Debug,Default,Serialize)]pub struct Timing{pub duration_ms:Option<u64>,pub ttft_ms:Option<u64>,pub output_tokens_per_second:Option<f64>,pub timing_estimated:bool,pub timing_source:Option<&'static str>,pub http_status:Option<u16>}
#[derive(Clone,Debug,Serialize)]pub struct UsageRecord{pub at:u64,pub model:String,pub provider:Option<String>,pub source_kind:String,#[serde(flatten)]pub tokens:Tokens,#[serde(flatten)]pub timing:Timing,pub estimated_cost_usd:Option<f64>,pub missing_price:bool}
#[derive(Clone,Debug,Serialize)]pub struct ProviderUsage{pub provider:Option<String>,#[serde(flatten)]pub summary:Summary,pub models:Vec<ModelUsage>}
#[derive(Clone,Debug,Default,Serialize)]pub struct ScanProgress{pub files_total:usize,pub files_complete:usize,pub bytes_total:u64,pub bytes_read:u64,pub total_known:bool}
#[derive(Clone,Debug,Serialize)]pub struct UsageSnapshot{
    pub start_ms:u64,pub end_ms:u64,pub updated_at:u64,pub source_available:bool,pub scan_pending:bool,pub partial:bool,
    pub files_scanned:usize,pub unreadable_files:usize,pub skipped_records:u64,pub duplicate_records:u64,pub partial_tail_files:usize,pub scan_progress:ScanProgress,
    pub summary:Summary,pub series:Vec<Bucket>,pub models:Vec<ModelUsage>,pub providers:Vec<ProviderUsage>,pub records:Vec<UsageRecord>,pub records_truncated:bool,
    pub request_count_kind:String,pub scope:String,pub notes:Vec<String>,
}

// Only these metadata fields deserialize to owned values. Conversation, command,
// filesystem/project and credential fields are unknown fields and are discarded.
#[derive(Default,Deserialize)]#[serde(default)]struct Envelope{timestamp:Option<String>,#[serde(rename="type")]kind:String,ordinal:Option<u64>,payload:Payload}
#[derive(Default,Deserialize)]#[serde(default)]struct Payload{
    #[serde(rename="type")]kind:String,id:Option<String>,timestamp:Option<String>,model:Option<String>,model_name:Option<String>,thread_id:Option<String>,response_id:Option<String>,
    forked_from_id:Option<String>,parent_thread_id:Option<String>,history_base_thread_id:Option<String>,history_mode:Option<String>,subagent_history_start_ordinal:Option<u64>,
    info:Option<UsageInfo>,usage:Option<Counts>,thread_settings:Option<ThreadSettings>,source:Option<Source>,model_provider:Option<String>,turn_id:Option<String>,role:Option<String>,item:Option<ItemMetadata>,started_at_ms:Option<u64>,completed_at_ms:Option<u64>,
}
#[derive(Default,Deserialize)]#[serde(default)]struct ItemMetadata{#[serde(rename="type")]kind:String}
#[derive(Deserialize)]#[serde(untagged)]enum Source{Label(String),Details(SourceDetails)}
#[derive(Default,Deserialize)]#[serde(default)]struct SourceDetails{subagent:Option<Subagent>}
#[derive(Default,Deserialize)]#[serde(default)]struct Subagent{thread_spawn:Option<Spawn>}
#[derive(Default,Deserialize)]#[serde(default)]struct Spawn{parent_thread_id:Option<String>}
impl Source{fn has_parent(&self)->bool{match self{Self::Label(label)=>label=="subagent",Self::Details(details)=>details.subagent.as_ref().and_then(|value|value.thread_spawn.as_ref()).and_then(|value|value.parent_thread_id.as_ref()).is_some()}}}
#[derive(Default,Deserialize)]#[serde(default)]struct ThreadSettings{model:Option<String>,model_provider_id:Option<String>}
#[derive(Default,Deserialize)]#[serde(default)]struct UsageInfo{total_token_usage:Option<Counts>,last_token_usage:Option<Counts>,model:Option<String>,model_name:Option<String>}
#[derive(Default,Deserialize)]#[serde(default)]struct Counts{input_tokens:Option<u64>,cached_input_tokens:Option<u64>,cache_write_input_tokens:Option<u64>,output_tokens:Option<u64>,reasoning_output_tokens:Option<u64>}
impl Counts{fn tokens(&self)->Option<Tokens>{let input_tokens=self.input_tokens?;let output_tokens=self.output_tokens?;let tokens=Tokens{input_tokens,cached_input_tokens:self.cached_input_tokens?,cache_write_input_tokens:self.cache_write_input_tokens.unwrap_or(0),output_tokens,reasoning_output_tokens:self.reasoning_output_tokens.unwrap_or(0),total_tokens:input_tokens.checked_add(output_tokens)?};tokens.valid().then_some(tokens)}}

#[derive(Clone)]struct RawUsage{at:u64,session:String,model:String,provider:Option<String>,total:Option<Tokens>,last:Option<Tokens>,response:Option<String>,modern:bool,continuation:bool,timing:Timing}
// Only scalar timing metadata is retained. Task durations are turn-wide, and
// token_count gaps include tools/idle time, so neither becomes request timing.
// Schemas: openai/codex codex-rs/protocol/src/protocol.rs (TokenUsageRecord,
// ItemCompletedEvent). Rollout timestamps are serialization times, not HTTP clocks.
#[derive(Default)]struct ResponseTiming{turn:Option<String>,boundary:Option<u64>,start:Option<u64>,first:Option<u64>,last_stamp:Option<u64>,ambiguous:bool}
impl ResponseTiming{
    fn observe(&mut self,envelope:&Envelope,session:Option<&str>){
        let Some(at)=envelope.timestamp.as_deref().and_then(timestamp)else{return};let payload=&envelope.payload;
        if payload.thread_id.as_deref().is_some_and(|thread|Some(thread)!=session){return}
        let reset=envelope.kind=="event_msg"&&matches!(payload.kind.as_str(),"task_started"|"turn_started"|"turn_aborted"|"task_complete"|"turn_complete"|"error"|"stream_error");
        if reset{let turn=if matches!(payload.kind.as_str(),"task_started"|"turn_started"){bounded_label(payload.turn_id.clone())}else{None};let ambiguous=if turn.is_some(){self.turn.is_some()&&(self.ambiguous||turn!=self.turn||self.first.is_some())}else{self.ambiguous};*self=Self{boundary:turn.as_ref().map(|_|at),turn,last_stamp:Some(at),ambiguous,..Self::default()};return}
        if self.last_stamp.is_some_and(|last|at<last){*self=Self::default();return}self.last_stamp=Some(at);
        if envelope.kind=="turn_context"{
            let Some(turn)=bounded_label(payload.turn_id.clone())else{return};
            if self.turn.as_ref()!=Some(&turn){let ambiguous=self.ambiguous||self.turn.is_some();*self=Self{turn:Some(turn),boundary:Some(at),last_stamp:Some(at),ambiguous,..Self::default()};}else{self.boundary=Some(at);}return
        }
        if self.turn.is_none(){return}
        if envelope.kind=="response_item"&&(payload.role.as_deref()==Some("user")||payload.kind.ends_with("_output")){self.boundary=Some(at);return}
        if envelope.kind!="event_msg"||!matches!(payload.kind.as_str(),"item_started"|"item_completed")||payload.turn_id.as_ref()!=self.turn.as_ref()||payload.thread_id.as_deref()!=session{return}
        if payload.kind=="item_completed"&&payload.item.as_ref().is_some_and(|item|matches!(item.kind.as_str(),"UserMessage"|"FunctionCallOutput"|"CommandExecution"|"McpToolCall"|"DynamicToolCall"|"FileChange")){self.boundary=Some(at);return}
        if !payload.item.as_ref().is_some_and(|item|matches!(item.kind.as_str(),"Reasoning"|"AgentMessage")){return}
        let Some(first)=payload.started_at_ms.filter(|first|*first<=at&&payload.completed_at_ms.is_none_or(|end|end==0||*first<=end&&end<=at))else{return};
        if self.first.is_none(){self.start=self.boundary;}self.first=Some(self.first.map_or(first,|old|old.min(first)));
    }
    fn finish(&mut self,at:u64,turn:Option<&str>,output:u64)->Timing{
        let mut timing=Timing::default();
        if !self.ambiguous&&self.turn.as_deref()==turn&&turn.is_some()&&self.last_stamp.is_none_or(|last|last<=at){
            if let Some(start)=self.start.or(self.boundary){if let Some(duration)=at.checked_sub(start).filter(|duration|*duration>0&&*duration<=DAY){
                if let Some(ttft)=self.first.and_then(|first|first.checked_sub(start)).filter(|ttft|*ttft<=duration){
                    timing.duration_ms=Some(duration);timing.ttft_ms=Some(ttft);
                    timing.output_tokens_per_second=duration.checked_sub(ttft).filter(|generation|*generation>=100).map(|generation|output as f64*1000.0/generation as f64).filter(|rate|rate.is_finite());
                    timing.timing_estimated=true;timing.timing_source=Some("codex_rollout_estimate");
                }
            }}
        }
        // A completed response is not itself evidence of the next request start.
        // Require a fresh user/context/tool boundary for the next response.
        self.boundary=self.boundary.filter(|boundary|*boundary>at);self.start=None;self.first=None;self.last_stamp=Some(at);timing
    }
}
#[derive(Default)]struct FileCache{
    offset:u64,len:u64,modified:u128,created:u64,head:u64,opened:bool,skipping:bool,quiet_skip:bool,waiting:bool,
    meta_seen:bool,session:Option<String>,model:String,provider:Option<String>,created_at:Option<u64>,history_start:Option<u64>,continuation:bool,modern_start:Option<u64>,
    events:Vec<RawUsage>,baseline:Option<RawUsage>,skipped:u64,timing:ResponseTiming,
}
#[derive(Default)]struct Cache{root:PathBuf,floor:u64,files:HashMap<PathBuf,FileCache>,limited:bool,evicted_through:Option<u64>}
fn fingerprint(bytes:&[u8])->u64{let mut hash=DefaultHasher::new();bytes.hash(&mut hash);hash.finish()}
fn bounded_label(value:Option<String>)->Option<String>{value.filter(|value|!value.is_empty()&&value.len()<=120&&value.chars().all(|ch|ch.is_ascii_alphanumeric()||"._-:/".contains(ch)))}
fn model(value:Option<String>)->String{bounded_label(value).filter(|value|!value.starts_with('/')&&!value.starts_with("./")&&!value.starts_with("../")&&!value.contains(":/")).unwrap_or_else(||"unknown".into())}
fn stamp(metadata:&Metadata)->(u128,u64){
    #[cfg(windows)]let created={use std::os::windows::fs::MetadataExt;metadata.creation_time()};
    #[cfg(not(windows))]let created=0;
    (metadata.modified().ok().and_then(|time|time.duration_since(UNIX_EPOCH).ok()).map_or(0,|time|time.as_nanos()),created)
}
fn reparse(metadata:&Metadata)->bool{
    #[cfg(windows)]{use std::os::windows::fs::MetadataExt;metadata.file_attributes()&0x400!=0}
    #[cfg(not(windows))]{metadata.file_type().is_symlink()}
}
fn local_home()->Result<PathBuf,String>{
    let root=if let Some(value)=std::env::var_os("CODEX_HOME").filter(|value|!value.is_empty()){PathBuf::from(value)}else{PathBuf::from(std::env::var_os("USERPROFILE").or_else(||std::env::var_os("HOME")).ok_or("无法确定本机 Codex 数据目录。")?).join(".codex")};
    if !root.is_absolute(){return Err("CODEX_HOME 需要是本机绝对路径。".into())}
    let root=match root.canonicalize(){Ok(root)=>root,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>root,Err(_)=>return Err("无法访问本机 Codex 数据目录。".into())};
    let text=root.to_string_lossy().to_ascii_lowercase();if text.starts_with("\\\\?\\unc\\")||(text.starts_with("\\\\")&&!text.starts_with("\\\\?\\")){return Err("用量统计只读取本机目录，不扫描网络共享。".into())}Ok(root)
}
fn list_files(root:&Path)->Result<(Vec<(PathBuf,Metadata)>,usize,bool),String>{
    let mut files=vec![];let mut unreadable=0;let mut limited=false;let mut visited=0usize;
    let mut pending=vec![(root.join("sessions"),0usize),(root.join("archived_sessions"),0usize)];
    while let Some((directory,depth))=pending.pop(){
        if visited>=MAX_FILES*5||files.len()>=MAX_FILES{limited=true;break}visited+=1;
        match fs::symlink_metadata(&directory){Ok(metadata)if reparse(&metadata)=>{limited=true;continue},Err(error)if error.kind()==std::io::ErrorKind::NotFound=>continue,Err(_)=>{unreadable+=1;continue},_=>{}}
        let entries=match fs::read_dir(&directory){Ok(entries)=>entries,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>continue,Err(_)=>{unreadable+=1;continue}};
        for entry in entries{if files.len()>=MAX_FILES||visited>=MAX_FILES*5{limited=true;break}visited+=1;let Ok(entry)=entry else{unreadable+=1;continue};let path=entry.path();let Ok(metadata)=fs::symlink_metadata(&path)else{unreadable+=1;continue};if reparse(&metadata){continue}
            if metadata.is_dir(){if depth<7{pending.push((path,depth+1));}else{limited=true;}}
            else if metadata.is_file()&&path.extension().is_some_and(|extension|extension=="jsonl"){files.push((path,metadata));}
            else if path.extension().is_some_and(|extension|extension=="zst"){limited=true;}
        }
    }
    files.sort_by(|a,b|b.1.modified().ok().cmp(&a.1.modified().ok()).then_with(||a.0.cmp(&b.0)));Ok((files,unreadable,limited))
}
// Early probes stop at the selected type; they do not deserialize text/content fields.
struct TypeProbe<'a>{value:&'a mut Option<String>,payload:bool}
impl<'de>serde::de::DeserializeSeed<'de> for TypeProbe<'_>{type Value=();fn deserialize<D:serde::Deserializer<'de>>(self,decoder:D)->Result<(),D::Error>{decoder.deserialize_map(self)}}
impl<'de>serde::de::Visitor<'de> for TypeProbe<'_>{type Value=();fn expecting(&self,f:&mut std::fmt::Formatter)->std::fmt::Result{f.write_str("Codex type metadata")}
    fn visit_map<M:serde::de::MapAccess<'de>>(self,mut map:M)->Result<(),M::Error>{while let Some(key)=map.next_key::<String>()?{
        if self.payload&&key=="payload"{return map.next_value_seed(TypeProbe{value:self.value,payload:false})}
        if !self.payload&&key=="type"{*self.value=Some(map.next_value::<String>()?);return Err(serde::de::Error::custom("metadata probe complete"))}
        map.next_value::<serde::de::IgnoredAny>()?;
    }Ok(())}
}
fn probe_kind(bytes:&[u8],payload:bool)->Option<String>{let mut value=None;let mut decoder=serde_json::Deserializer::from_slice(bytes);
    // Intentional early error avoids serde's end-of-map check and stops before content.
    let _=serde::de::DeserializeSeed::deserialize(TypeProbe{value:&mut value,payload},&mut decoder);value
}
fn header_kind(bytes:&[u8])->Option<String>{probe_kind(bytes,false)}
fn payload_kind(bytes:&[u8])->Option<String>{probe_kind(bytes,true)}
fn ignored_content(bytes:&[u8])->bool{match header_kind(bytes).as_deref(){Some("event_msg")=>payload_kind(bytes).is_some_and(|kind|!matches!(kind.as_str(),"token_count"|"thread_settings_applied"|"task_started"|"turn_started"|"task_complete"|"turn_complete"|"turn_aborted"|"item_started"|"item_completed"|"error"|"stream_error")),Some(kind)=>!matches!(kind,"session_meta"|"turn_context"|"token_usage_record"),None=>false}}
fn consume(cache:&mut FileCache,bytes:&[u8],floor:u64)->bool{
    if bytes.iter().all(u8::is_ascii_whitespace){return true}
    // Response-item bodies are ignored by serde; only type/role and timestamp
    // can advance a boundary. Oversized content stays skipped by the reader.
    if ignored_content(bytes)&&header_kind(bytes).as_deref()!=Some("response_item"){return true}
    let Ok(envelope)=serde_json::from_slice::<Envelope>(bytes)else{return header_kind(bytes).as_deref()==Some("response_item")};
    if envelope.kind!="session_meta"&&!(cache.history_start.is_some_and(|start|envelope.ordinal.is_some_and(|ordinal|ordinal<start))||cache.continuation&&cache.created_at.is_some_and(|created|envelope.timestamp.as_deref().and_then(timestamp).is_none_or(|at|at<=created))){cache.timing.observe(&envelope,cache.session.as_deref());}
    let payload=envelope.payload;
    if envelope.kind=="session_meta"{
        // Later session_meta entries can be copied parent history. Only the first owns this file.
        if !cache.meta_seen{cache.meta_seen=true;cache.session=bounded_label(payload.id);cache.provider=bounded_label(payload.model_provider).filter(|value|!value.contains('/')&&!value.contains(':'));cache.created_at=payload.timestamp.as_deref().and_then(timestamp).or_else(||envelope.timestamp.as_deref().and_then(timestamp));cache.history_start=payload.subagent_history_start_ordinal;
            cache.continuation=payload.forked_from_id.is_some()||payload.parent_thread_id.is_some()||payload.history_base_thread_id.is_some()||payload.source.as_ref().is_some_and(Source::has_parent)||payload.history_mode.as_deref()==Some("paginated");if payload.model.is_some(){cache.model=model(payload.model);}}
        return true
    }
    if cache.history_start.is_some_and(|start|envelope.ordinal.is_some_and(|ordinal|ordinal<start)){return true}
    if envelope.kind=="turn_context"{cache.model=model(payload.model);return true}
    if envelope.kind=="event_msg"&&payload.kind=="thread_settings_applied"{if payload.thread_id.as_deref().is_none_or(|thread|Some(thread)==cache.session.as_deref()){if let Some(settings)=payload.thread_settings{if settings.model.is_some(){cache.model=model(settings.model);}if settings.model_provider_id.is_some(){cache.provider=bounded_label(settings.model_provider_id).filter(|value|!value.contains('/')&&!value.contains(':'));}}}return true}
    let modern=envelope.kind=="token_usage_record";
    if !modern&&(envelope.kind!="event_msg"||payload.kind!="token_count"){return true}
    let Some(at)=envelope.timestamp.as_deref().and_then(timestamp)else{cache.skipped+=1;return true};
    if cache.continuation&&cache.created_at.is_some_and(|created|at<=created){return true}
    let Some(session)=cache.session.clone()else{cache.skipped+=1;return true};
    if payload.thread_id.as_ref().is_some_and(|thread|thread!=&session){return true}
    let(total,last)=if modern{(None,payload.usage.as_ref().and_then(Counts::tokens))}else{
        let Some(info)=payload.info else{return true};if info.model.is_some()||info.model_name.is_some(){cache.model=model(info.model.or(info.model_name));}(info.total_token_usage.as_ref().and_then(Counts::tokens),info.last_token_usage.as_ref().and_then(Counts::tokens))
    };
    if payload.model.is_some()||payload.model_name.is_some(){cache.model=model(payload.model.or(payload.model_name));}
    if total.is_none()&&last.is_none(){cache.skipped+=1;return true}
    if modern{cache.modern_start=Some(cache.modern_start.map_or(at,|previous|previous.min(at)));}
    let timing=if modern{cache.timing.finish(at,payload.turn_id.as_deref(),last.map_or(0,|tokens|tokens.output_tokens))}else{Timing::default()};
    let event=RawUsage{at,session,model:if cache.model.is_empty(){"unknown".into()}else{cache.model.clone()},provider:cache.provider.clone(),total,last,response:bounded_label(payload.response_id),modern,continuation:cache.continuation,timing};
    if at<floor{if cache.baseline.as_ref().is_none_or(|previous|previous.at<=at){cache.baseline=Some(event);}}else{cache.events.push(event);}true
}
fn scan_file(path:&Path,metadata:&Metadata,cache:&mut FileCache,floor:u64,budget:&mut u64,event_room:usize)->Result<bool,()> {
    let(modified,created)=stamp(metadata);if cache.opened&&cache.len==metadata.len()&&cache.modified==modified&&cache.created==created&&(cache.waiting||cache.offset==metadata.len()){return Ok(false)}
    let mut file=File::open(path).map_err(|_|())?;let mut head=vec![0u8;512];let count=file.read(&mut head).map_err(|_|())?;head.truncate(count);let head=fingerprint(&head);
    if cache.offset>metadata.len()||cache.created!=created||cache.head!=head||(cache.len==metadata.len()&&cache.modified!=modified){*cache=FileCache::default();}
    if cache.len==metadata.len()&&cache.modified==modified&&(cache.waiting||cache.offset==metadata.len()){return Ok(false)}
    cache.created=created;cache.head=head;cache.len=metadata.len();cache.modified=modified;cache.opened=true;cache.waiting=false;
    file.seek(SeekFrom::Start(cache.offset)).map_err(|_|())?;let mut reader=BufReader::with_capacity(64*1024,file);let limit=(*budget).min(FILE_BUDGET);let mut spent=0u64;let original_events=cache.events.len();
    while cache.offset<metadata.len(){if spent>=limit{*budget=budget.saturating_sub(spent);return Ok(true)}
        let start=cache.offset;let mut line=Vec::with_capacity(2048);let mut consumed=0u64;let mut completed=false;
        loop{let chunk=reader.fill_buf().map_err(|_|())?;if chunk.is_empty(){break}let take=chunk.iter().position(|byte|*byte==b'\n').map_or(chunk.len(),|index|index+1).min((limit-spent)as usize);if take==0{break}
            if !cache.skipping{if line.len()+take<=MAX_LINE{line.extend_from_slice(&chunk[..take]);}else{cache.quiet_skip=ignored_content(&line);line.clear();cache.skipping=true;}}
            completed=chunk[take-1]==b'\n';reader.consume(take);consumed+=take as u64;spent+=take as u64;
            if cache.skipping{cache.offset=start+consumed;}
            if completed||spent>=limit{break}
        }
        if cache.skipping{if completed{if !cache.quiet_skip{cache.skipped+=1;}cache.skipping=false;cache.quiet_skip=false;}else{cache.waiting=cache.offset==metadata.len();*budget=budget.saturating_sub(spent);return Ok(cache.offset<metadata.len())}continue}
        if completed{if !consume(cache,&line,floor){cache.skipped+=1;}cache.offset=start+consumed;}
        else if start+consumed==metadata.len(){if ignored_content(&line){cache.offset=start+consumed;cache.skipping=true;cache.quiet_skip=true;cache.waiting=true;}else if consume(cache,&line,floor){cache.offset=start+consumed;}else{cache.waiting=true;}break}
        else{*budget=budget.saturating_sub(spent);return Ok(true)}
        if cache.events.len()-original_events>=event_room{*budget=budget.saturating_sub(spent);return Ok(cache.offset<metadata.len())}
    }*budget=budget.saturating_sub(spent);Ok(false)
}
fn aggregate(cache:&Cache,start_ms:u64,end_ms:u64,pending:bool,unreadable:usize,available:bool)->UsageSnapshot{
    let mut events=vec![];let mut modern_start:HashMap<String,u64>=HashMap::new();let mut skipped=0;let mut partial_tail_files=0;
    for file in cache.files.values(){skipped+=file.skipped;if file.waiting{partial_tail_files+=1;}if let(Some(session),Some(at))=(&file.session,file.modern_start){modern_start.entry(session.clone()).and_modify(|previous|*previous=(*previous).min(at)).or_insert(at);}events.extend(file.baseline.iter().map(|event|(event,true)));events.extend(file.events.iter().map(|event|(event,false)));}
    events.sort_by(|(a,_),(b,_)|a.at.cmp(&b.at).then_with(||a.session.cmp(&b.session)).then_with(||a.modern.cmp(&b.modern)).then_with(||(a.model=="unknown").cmp(&(b.model=="unknown"))).then_with(||a.timing.duration_ms.is_none().cmp(&b.timing.duration_ms.is_none())));
    let chart_start=if start_ms==0{events.iter().filter(|(event,baseline)|!baseline&&event.at<end_ms).map(|(event,_)|event.at).min().unwrap_or(end_ms.saturating_sub(DAY))}else{start_ms};
    // At most about 180 bins, including for all-history/custom ranges. The
    // original requested bounds remain in the snapshot; no history is invented.
    let span=end_ms.saturating_sub(chart_start);let bucket=if span<=2*DAY{3_600_000}else{DAY*(span/(180*DAY)+u64::from(span%(180*DAY)!=0)).max(1)};
    let bucket_start=|at:u64|if bucket>=DAY{(((at+8*3_600_000)/bucket)*bucket).saturating_sub(8*3_600_000)}else{at/bucket*bucket};
    let mut legacy_seen:HashSet<(String,u64,Tokens)>=HashSet::new();let mut modern_seen:HashSet<String>=HashSet::new();let mut totals:HashMap<String,Tokens>=HashMap::new();let mut duplicate=0;let mut model_limited=false;let mut provider_limited=false;let mut summary=Summary{missing_price:true,..Summary::default()};let mut records=vec![];let mut models:BTreeMap<String,Summary>=BTreeMap::new();let mut providers:BTreeMap<Option<String>,(Summary,BTreeMap<String,Summary>)>=BTreeMap::new();let mut bins:BTreeMap<u64,Summary>=BTreeMap::new();let mut bin_models:BTreeMap<u64,BTreeMap<String,Summary>>=BTreeMap::new();
    for (event,baseline) in events{
        if event.at>=end_ms{continue}
        let value=if event.modern{
            let Some(tokens)=event.last else{continue};let key=event.response.as_ref().map(|response|format!("{}:{response}",event.session)).unwrap_or_else(||format!("{}:{}:{tokens:?}",event.session,event.at));
            if !modern_seen.insert(key){duplicate+=1;continue}tokens
        }else{
            if modern_start.get(&event.session).is_some_and(|at|event.at>=*at){continue}
            if let Some(total)=event.total{
                if !legacy_seen.insert((event.session.clone(),event.at,total))||totals.get(&event.session)==Some(&total){duplicate+=1;continue}
                let previous=totals.insert(event.session.clone(),total);
                if let Some(last)=event.last.filter(|last|last.positive()){last}
                else if let Some(previous)=previous{match total.difference(previous){Some(delta)=>delta,None=>{skipped+=1;continue}}}
                else if event.continuation{skipped+=1;continue}else{total}
            }else{let Some(last)=event.last else{continue};let key=format!("legacy:{}:{}:{last:?}",event.session,event.at);if !modern_seen.insert(key){duplicate+=1;continue}last}
        };
        if baseline||!value.positive()||event.at<start_ms{continue}
        if !summary.add(value){skipped+=1;continue}
        let model_key=if models.len()<128||models.contains_key(&event.model){event.model.clone()}else{model_limited=true;"其他模型（合并）".into()};models.entry(model_key.clone()).or_default().add(value);
        let provider_key=if providers.len()<128||providers.contains_key(&event.provider){event.provider.clone()}else{provider_limited=true;None};
        let(provider_summary,provider_models)=providers.entry(provider_key).or_default();provider_summary.add(value);provider_models.entry(model_key.clone()).or_default().add(value);
        let at=bucket_start(event.at);bins.entry(at).or_default().add(value);bin_models.entry(at).or_default().entry(model_key).or_default().add(value);
        records.push(UsageRecord{at:event.at,model:event.model.clone(),provider:event.provider.clone(),source_kind:if event.modern{"token_usage_record"}else{"token_count"}.into(),tokens:value,timing:event.timing,estimated_cost_usd:None,missing_price:true});
    }
    let records_truncated=records.len()>500;if records_truncated{records.drain(..records.len()-500);}records.reverse();
    let mut series=vec![];let mut at=bucket_start(chart_start);while at<end_ms{series.push(Bucket{start_ms:at,end_ms:(at+bucket).min(end_ms),summary:bins.remove(&at).unwrap_or(Summary{missing_price:true,..Summary::default()}),models:bin_models.remove(&at).unwrap_or_default().into_iter().map(|(model,summary)|ModelUsage{model,summary}).collect()});at+=bucket;}
    let mut models=models.into_iter().map(|(model,summary)|ModelUsage{model,summary}).collect::<Vec<_>>();models.sort_by(|a,b|b.summary.tokens.total_tokens.cmp(&a.summary.tokens.total_tokens).then_with(||a.model.cmp(&b.model)));
    let mut providers=providers.into_iter().map(|(provider,(summary,models))|ProviderUsage{provider,summary,models:models.into_iter().map(|(model,summary)|ModelUsage{model,summary}).collect()}).collect::<Vec<_>>();providers.sort_by(|a,b|b.summary.tokens.total_tokens.cmp(&a.summary.tokens.total_tokens).then_with(||a.provider.cmp(&b.provider)));
    let mut notes=vec!["仅统计本机现存 Codex 会话中的用量元数据，不代表官方账单、真实付款或订阅剩余额度。".into(),"输入已包含缓存输入，输出已包含推理输出；合计只计算输入与输出。".into(),"记录数表示去重后的有效用量记录，不包含无用量记录的失败请求。".into(),"费用由界面结合公开参考价或本机自定义单价估算；缺少完整单价时保持未知。".into()];
    let history_limited=cache.evicted_through.is_some_and(|at|at>=start_ms);
    if pending{notes.push("正在增量扫描本机记录，当前结果尚未完整。".into());}if partial_tail_files>0{notes.push("部分会话尾部尚未写入完成，完成后将继续读取。".into());}if history_limited{notes.push(format!("仅保留最近 {MAX_EVENTS} 条用量元数据，更早记录未计入当前结果。"));}if cache.limited{notes.push("部分目录为链接、超出扫描范围，或记录使用了未支持的压缩格式。".into());}if model_limited||provider_limited{notes.push("模型或提供方种类超过显示范围，其余用量已合并展示。".into());}if skipped>0||unreadable>0{notes.push("存在损坏、无法读取或无法可靠归属的记录，已跳过。".into());}if !available{notes.push("未找到可读取的本机 Codex 会话记录。".into());}
    UsageSnapshot{start_ms,end_ms,updated_at:crate::settings::now(),source_available:available,scan_pending:pending,partial:pending||partial_tail_files>0||history_limited||cache.limited||model_limited||provider_limited||skipped>0||unreadable>0,files_scanned:cache.files.values().filter(|file|file.opened).count(),unreadable_files:unreadable,skipped_records:skipped,duplicate_records:duplicate,partial_tail_files,scan_progress:ScanProgress::default(),summary,series,models,providers,records,records_truncated,request_count_kind:"deduplicated_usage_records".into(),scope:"existing_local_codex_session_metadata".into(),notes}
}
fn read_at(root:&Path,start_ms:u64,end_ms:u64,cache:&mut Cache)->Result<UsageSnapshot,String>{
    read_at_bounded(root,start_ms,end_ms,cache,MAX_EVENTS)
}
fn retain_recent(cache:&mut Cache,limit:usize){
    let mut stamps=cache.files.values().flat_map(|file|file.events.iter().map(|event|event.at)).collect::<Vec<_>>();
    if stamps.len()<=limit{return}
    let remove=stamps.len()-limit;let cutoff=*stamps.select_nth_unstable(remove-1).1;let mut equals=remove-stamps.iter().filter(|at|**at<cutoff).count();
    for file in cache.files.values_mut(){file.events.retain(|event|{let drop=event.at<cutoff||(event.at==cutoff&&equals>0);if drop{if event.at==cutoff{equals-=1;}if file.baseline.as_ref().is_none_or(|previous|previous.at<=event.at){file.baseline=Some(event.clone());}}!drop});}
    cache.evicted_through=Some(cache.evicted_through.map_or(cutoff,|previous|previous.max(cutoff)));
}
fn read_at_bounded(root:&Path,start_ms:u64,end_ms:u64,cache:&mut Cache,event_limit:usize)->Result<UsageSnapshot,String>{
    if end_ms<=start_ms||end_ms>8_640_000_000_000_000{return Err("用量统计时间范围无效。".into())}
    let floor=start_ms;if cache.root!=root||start_ms<cache.floor{*cache=Cache{root:root.to_path_buf(),floor,..Cache::default()};}
    // Keep the broadest loaded scope when narrowing dates; expanding beyond it
    // resets once above, while 30d -> 7d -> 24h reuses parsed metadata.
    let(files,mut unreadable,limited)=list_files(root)?;let available=!files.is_empty();cache.limited=limited;let active=files.iter().map(|(path,_)|path.clone()).collect::<HashSet<_>>();cache.files.retain(|path,_|active.contains(path));
    let mut budget=READ_BUDGET;let mut pending=false;let mut count=cache.files.values().map(|file|file.events.len()).sum::<usize>();
    // At most one bounded batch beyond the retained limit exists during a scan.
    for(path,metadata)in &files{let file=cache.files.entry(path.clone()).or_default();let previous=file.events.len();if budget==0{pending|=file.offset<metadata.len()&&!file.waiting;continue}match scan_file(path,metadata,file,cache.floor,&mut budget,event_limit.clamp(1,4096)){Ok(more)=>pending|=more,Err(())=>unreadable+=1}count=count.saturating_sub(previous)+file.events.len();if count>event_limit{retain_recent(cache,event_limit);count=event_limit;}}
    let mut progress=ScanProgress{files_total:files.len(),total_known:!limited&&unreadable==0,..ScanProgress::default()};
    for(path,metadata)in &files{progress.bytes_total=progress.bytes_total.saturating_add(metadata.len());if let Some(file)=cache.files.get(path){progress.bytes_read=progress.bytes_read.saturating_add(file.offset.min(metadata.len()));if file.opened&&file.offset>=metadata.len(){progress.files_complete+=1;}}}
    let mut snapshot=aggregate(cache,start_ms,end_ms,pending,unreadable,available);snapshot.scan_progress=progress;Ok(snapshot)
}
/// Call through spawn_blocking. All caches are in-memory metadata and file cursors.
pub fn read(start_ms:u64,end_ms:u64)->Result<UsageSnapshot,String>{let root=local_home()?;let mut cache=CACHE.get_or_init(||Mutex::new(Cache::default())).lock().map_err(|_|"用量统计缓存暂不可用。")?;read_at(&root,start_ms,end_ms,&mut cache)}

// RFC3339 timestamps in rollouts; no platform timezone dependency or extra crate.
fn timestamp(value:&str)->Option<u64>{
    let bytes=value.as_bytes();if bytes.len()<20||bytes[4]!=b'-'||bytes[7]!=b'-'||!matches!(bytes[10],b'T'|b't')||bytes[13]!=b':'||bytes[16]!=b':'{return None}
    let number=|a:usize,b:usize|{let text=value.get(a..b)?;if !text.bytes().all(|byte|byte.is_ascii_digit()){return None}text.parse::<i64>().ok()};let(y,m,d,h,min,s)=(number(0,4)?,number(5,7)?,number(8,10)?,number(11,13)?,number(14,16)?,number(17,19)?);
    let leap=y%4==0&&(y%100!=0||y%400==0);let days=[31,if leap{29}else{28},31,30,31,30,31,31,30,31,30,31];if !(1970..=9999).contains(&y)||!(1..=12).contains(&m)||d<1||d>days[(m-1)as usize]||h>23||min>59||s>59{return None}
    let mut position=19;let mut milliseconds=0i64;if bytes.get(position)==Some(&b'.'){position+=1;let start=position;while bytes.get(position).is_some_and(u8::is_ascii_digit){position+=1;}if position==start{return None}for index in 0..3{milliseconds=milliseconds*10+if start+index<position{(bytes[start+index]-b'0')as i64}else{0};}}
    let offset=match bytes.get(position)?{b'Z'|b'z' if position+1==bytes.len()=>0,b'+'|b'-' if position+6==bytes.len()&&bytes[position+3]==b':'=>{let hours=number(position+1,position+3)?;let mins=number(position+4,position+6)?;if hours>23||mins>59{return None}(hours*60+mins)*if bytes[position]==b'+'{1}else{-1}},_=>return None};
    let year=y-i64::from(m<=2);let era=year.div_euclid(400);let yoe=year-era*400;let mp=m+if m>2{-3}else{9};let doy=(153*mp+2)/5+d-1;let day=era*146097+yoe*365+yoe/4-yoe/100+doy-719468;
    (day*DAY as i64+(h*3600+min*60+s-offset*60)*1000+milliseconds).try_into().ok()
}

#[cfg(test)]mod tests{
    use super::*;
    fn token(at:&str,input:u64,cache:u64,output:u64,last:Option<(u64,u64,u64)>)->serde_json::Value{let mut info=serde_json::json!({"total_token_usage":{"input_tokens":input,"cached_input_tokens":cache,"output_tokens":output,"reasoning_output_tokens":0}});if let Some((input,cache,output))=last{info["last_token_usage"]=serde_json::json!({"input_tokens":input,"cached_input_tokens":cache,"output_tokens":output,"reasoning_output_tokens":0});}serde_json::json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":info}})}
    fn meta(id:&str)->serde_json::Value{serde_json::json!({"timestamp":"2026-11-11T23:00:00Z","type":"session_meta","payload":{"id":id,"timestamp":"2026-11-11T23:00:00Z"}})}
    fn add(file:&mut FileCache,value:serde_json::Value){assert!(consume(file,&serde_json::to_vec(&value).unwrap(),0));}
    fn range()->(u64,u64){(timestamp("2026-11-12T00:00:00Z").unwrap(),timestamp("2026-11-13T00:00:00Z").unwrap())}
    #[test]fn bounded_all_history_keeps_accepting_appended_and_new_session_records(){
        use std::io::Write;
        let root=std::env::temp_dir().join(format!("usage-cap-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(root.join("sessions")).unwrap();
        let mut file=File::create(root.join("sessions/a.jsonl")).unwrap();writeln!(file,"{}",meta("capped")).unwrap();
        for hour in 1..=3{writeln!(file,"{}",token(&format!("2026-11-12T{hour:02}:00:00Z"),hour*100,0,0,None)).unwrap();}file.flush().unwrap();
        let mut cache=Cache::default();let end=range().1;let first=read_at_bounded(&root,0,end,&mut cache,3).unwrap();assert_eq!(first.summary.request_count,3);assert!(!first.partial);
        writeln!(file,"{}",token("2026-11-12T04:00:00Z",400,0,0,None)).unwrap();file.flush().unwrap();
        let second=read_at_bounded(&root,0,end,&mut cache,3).unwrap();assert!(second.partial);assert_eq!(second.summary.request_count,3);assert_eq!(second.summary.tokens.total_tokens,300,"Evicted cumulative metadata seeds the baseline but is not counted again");assert_eq!(second.records[0].at,timestamp("2026-11-12T04:00:00Z").unwrap());
        let again=read_at_bounded(&root,0,end,&mut cache,3).unwrap();assert_eq!(again.summary.tokens.total_tokens,300);assert_eq!(again.summary.request_count,3);
        fs::write(root.join("sessions/b.jsonl"),format!("{}\n{}\n",meta("new-session"),token("2026-11-12T05:00:00Z",10,0,0,None))).unwrap();
        let third=read_at_bounded(&root,0,end,&mut cache,3).unwrap();assert_eq!(third.summary.request_count,3);assert_eq!(third.summary.tokens.total_tokens,210);assert_eq!(third.records[0].at,timestamp("2026-11-12T05:00:00Z").unwrap());assert_eq!(cache.files.values().map(|file|file.events.len()).sum::<usize>(),3);
        drop(file);fs::remove_dir_all(root).unwrap();
    }
    #[test]fn cumulative_replays_model_switches_and_cache_subsets_are_accounted_once(){let mut file=FileCache::default();add(&mut file,meta("thread-a"));add(&mut file,serde_json::json!({"type":"turn_context","payload":{"model":"model-a"}}));let first=token("2026-11-12T01:00:00Z",100,80,10,Some((100,80,10)));add(&mut file,first.clone());add(&mut file,first.clone());add(&mut file,serde_json::json!({"type":"turn_context","payload":{"model":"model-b"}}));add(&mut file,token("2026-11-12T02:00:00Z",220,180,30,Some((120,100,20))));let mut cache=Cache::default();cache.files.insert("sessions/a.jsonl".into(),file);let mut archive=FileCache::default();add(&mut archive,meta("thread-a"));add(&mut archive,first);cache.files.insert("archive/a.jsonl".into(),archive);let(start,end)=range();let result=aggregate(&cache,start,end,false,0,true);assert_eq!(result.summary.tokens.total_tokens,250);assert_eq!(result.summary.tokens.cached_input_tokens,180);assert_eq!(result.summary.request_count,2);assert_eq!(result.duplicate_records,2);assert_eq!(result.models.len(),2);assert!(result.summary.estimated_cost_usd.is_none());assert!(result.summary.missing_price);}
    #[test]fn modern_response_ids_win_over_mirrored_legacy_and_fork_history(){let mut file=FileCache::default();add(&mut file,serde_json::json!({"timestamp":"2026-11-12T01:00:00Z","type":"session_meta","payload":{"id":"child","forked_from_id":"parent","timestamp":"2026-11-12T01:00:00Z","subagent_history_start_ordinal":5}}));let mut copied=token("2026-11-12T01:00:00Z",9999,9000,100,None);copied["ordinal"]=2.into();add(&mut file,copied);let modern=serde_json::json!({"timestamp":"2026-11-12T02:00:00Z","ordinal":6,"type":"token_usage_record","payload":{"thread_id":"child","response_id":"resp-a","usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10}}});add(&mut file,modern.clone());add(&mut file,modern);add(&mut file,token("2026-11-12T02:00:00.001Z",100,80,10,Some((100,80,10))));let mut cache=Cache::default();cache.files.insert("child".into(),file);let(start,end)=range();let result=aggregate(&cache,start,end,false,0,true);assert_eq!(result.summary.tokens.total_tokens,110);assert_eq!(result.summary.request_count,1);}
    #[test]fn continuation_without_baseline_and_invalid_counters_never_invent_usage(){let mut file=FileCache::default();add(&mut file,serde_json::json!({"timestamp":"2026-11-11T23:00:00Z","type":"session_meta","payload":{"id":"continued","history_mode":"paginated"}}));add(&mut file,token("2026-11-12T01:00:00Z",10000,8000,100,None));add(&mut file,token("2026-11-12T02:00:00Z",10100,8080,110,None));add(&mut file,token("2026-11-12T03:00:00Z",1,8,1,Some((1,8,1))));let mut cache=Cache::default();cache.files.insert("continued".into(),file);let(start,end)=range();let result=aggregate(&cache,start,end,false,0,true);assert_eq!(result.summary.tokens.total_tokens,110);assert_eq!(result.summary.request_count,1);assert!(result.partial);}
    #[test]fn repeated_totals_are_not_requests_but_verified_counter_resets_can_repeat_values(){let mut file=FileCache::default();add(&mut file,meta("reset"));for(at,input,cache,output,last)in[("2026-11-12T01:00:00Z",100,80,10,(100,80,10)),("2026-11-12T01:00:01Z",100,80,10,(100,80,10)),("2026-11-12T02:00:00Z",250,200,20,(150,120,10)),("2026-11-12T03:00:00Z",100,80,10,(100,80,10)),("2026-11-12T04:00:00Z",200,160,20,(100,80,10))]{add(&mut file,token(at,input,cache,output,Some(last)));}let mut cache=Cache::default();cache.files.insert("reset".into(),file);let(start,end)=range();let result=aggregate(&cache,start,end,false,0,true);assert_eq!(result.summary.tokens.total_tokens,490);assert_eq!(result.summary.request_count,4);assert_eq!(result.duplicate_records,1);}
    #[test]fn conversation_and_paths_are_not_retained_or_returned(){let mut file=FileCache::default();let mut metadata=meta("private-id");metadata["payload"]["cwd"]="PRIVATE_PROJECT_PATH".into();metadata["payload"]["instructions"]="PRIVATE_PROMPT".into();add(&mut file,metadata);add(&mut file,serde_json::json!({"timestamp":"2026-11-12T00:00:00Z","type":"response_item","payload":{"type":"message","content":"PRIVATE_CONVERSATION"}}));add(&mut file,token("2026-11-12T01:00:00Z",100,80,10,Some((100,80,10))));let mut cache=Cache::default();cache.files.insert("PRIVATE_FILE_PATH".into(),file);let(start,end)=range();let snapshot=serde_json::to_string(&aggregate(&cache,start,end,false,0,true)).unwrap();for private in ["PRIVATE_PROJECT_PATH","PRIVATE_PROMPT","PRIVATE_CONVERSATION","PRIVATE_FILE_PATH","private-id"]{assert!(!snapshot.contains(private));}}
    #[test]fn timestamps_accept_fraction_offsets_and_shanghai_day_boundaries(){assert_eq!(timestamp("1970-01-01T00:00:00Z"),Some(0));assert_eq!(timestamp("2026-11-12T08:01:02.123+08:00"),timestamp("2026-11-12T00:01:02.123Z"));assert_eq!(timestamp("2026-11-12T00:01:02.1Z").unwrap()%1000,100);assert_eq!(timestamp("2026-11-12T12:01:02.1+12:00"),timestamp("2026-11-12T00:01:02.100Z"));assert_eq!(timestamp("2026-11-12T00:01:02.12-12:00"),timestamp("2026-11-12T12:01:02.120Z"));for invalid in ["2026-02-29T00:00:00Z","2026-11-12T25:00:00Z","2026-11-12T00:00:00","2026-11-12T00:00:00.+08:00"]{assert!(timestamp(invalid).is_none());}}
    #[test]fn all_history_and_long_custom_ranges_keep_bounded_bins_and_real_provider_metadata(){
        let mut file=FileCache::default();let mut metadata=meta("provider-fixture");metadata["payload"]["model_provider"]="fixture-provider".into();add(&mut file,metadata);add(&mut file,serde_json::json!({"type":"turn_context","payload":{"model":"fixture-model"}}));
        add(&mut file,token("2026-11-12T01:00:00Z",100,80,10,Some((100,80,10))));let mut cache=Cache::default();cache.files.insert("synthetic".into(),file);let end=timestamp("2036-11-13T00:00:00Z").unwrap();
        let all=aggregate(&cache,0,end,false,0,true);assert_eq!(all.start_ms,0);assert!(all.series.len()<=182);assert_eq!(all.summary.request_count,1);assert_eq!(all.records[0].provider.as_deref(),Some("fixture-provider"));assert_eq!(all.records[0].source_kind,"token_count");assert_eq!(all.providers[0].models[0].model,"fixture-model");assert_eq!(all.providers[0].summary.tokens.total_tokens,110);
        let long=aggregate(&cache,1,end,false,0,true);assert!(long.series.len()<=182);assert_eq!(long.summary.tokens.total_tokens,110);
        let absent=std::env::temp_dir().join(format!("codex-usage-absent-{}-{}",std::process::id(),crate::settings::now()));let mut empty=Cache::default();let snapshot=read_at(&absent,1,end,&mut empty).unwrap();assert!(!snapshot.source_available);assert!(snapshot.series.len()<=182);assert!(read_at(&absent,end,end,&mut empty).is_err());
    }
    #[test]fn modern_timing_uses_correlated_turn_output_metadata_and_never_token_count_gaps(){
        let mut file=FileCache::default();add(&mut file,meta("timing-thread"));let base=timestamp("2026-11-12T00:00:00Z").unwrap();
        let event=|at:&str,kind:&str,payload:serde_json::Value|serde_json::json!({"timestamp":at,"type":kind,"payload":payload});
        add(&mut file,event("2026-11-12T00:00:00Z","event_msg",serde_json::json!({"type":"task_started","turn_id":"turn-a"})));
        add(&mut file,event("2026-11-12T00:00:00.100Z","turn_context",serde_json::json!({"turn_id":"turn-a","model":"fixture"})));
        add(&mut file,event("2026-11-12T00:00:00.200Z","response_item",serde_json::json!({"type":"message","role":"user","content":"PRIVATE_CONVERSATION"})));
        add(&mut file,event("2026-11-12T00:00:02.001Z","event_msg",serde_json::json!({"type":"item_completed","thread_id":"timing-thread","turn_id":"turn-a","item":{"type":"Reasoning","content":"PRIVATE_REASONING"},"started_at_ms":base+1_200,"completed_at_ms":base+2_000})));
        add(&mut file,event("2026-11-12T00:00:04.001Z","event_msg",serde_json::json!({"type":"item_completed","thread_id":"timing-thread","turn_id":"turn-a","item":{"type":"AgentMessage","text":"PRIVATE_OUTPUT"},"started_at_ms":base+2_100,"completed_at_ms":base+4_000})));
        let usage=|at:&str,response:&str,turn:&str|event(at,"token_usage_record",serde_json::json!({"thread_id":"timing-thread","turn_id":turn,"response_id":response,"usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":400,"reasoning_output_tokens":200},"status":"completed"}));
        add(&mut file,usage("2026-11-12T00:00:05Z","response-a","turn-a"));
        add(&mut file,token("2026-11-12T00:00:05.001Z",100,80,400,Some((100,80,400))));
        add(&mut file,usage("2026-11-12T00:00:10Z","response-b","turn-a"));
        add(&mut file,event("2026-11-12T00:00:11Z","response_item",serde_json::json!({"type":"function_call_output","output":"PRIVATE_TOOL_OUTPUT"})));
        add(&mut file,event("2026-11-12T00:00:14Z","event_msg",serde_json::json!({"type":"item_completed","thread_id":"timing-thread","turn_id":"turn-a","item":{"type":"AgentMessage"},"started_at_ms":base+12_000})));
        add(&mut file,usage("2026-11-12T00:00:15Z","response-c","turn-a"));
        add(&mut file,event("2026-11-12T00:00:16Z","event_msg",serde_json::json!({"type":"item_completed","thread_id":"timing-thread","turn_id":"turn-a","item":{"type":"CommandExecution","output":"PRIVATE_COMMAND_OUTPUT"},"started_at_ms":base+15_200,"completed_at_ms":base+16_000})));
        add(&mut file,event("2026-11-12T00:00:18Z","event_msg",serde_json::json!({"type":"item_completed","thread_id":"timing-thread","turn_id":"turn-a","item":{"type":"Reasoning"},"started_at_ms":base+17_000})));
        add(&mut file,usage("2026-11-12T00:00:20Z","response-d","turn-a"));
        let mut cache=Cache::default();cache.files.insert("synthetic".into(),file);let(start,end)=range();let result=aggregate(&cache,start,end,false,0,true);assert_eq!(result.records.len(),4);
        let first=result.records.iter().find(|row|row.at==base+5_000).unwrap();assert_eq!(first.timing.duration_ms,Some(4_800));assert_eq!(first.timing.ttft_ms,Some(1_000));assert!((first.timing.output_tokens_per_second.unwrap()-400.0/3.8).abs()<0.0001);assert!(first.timing.timing_estimated);assert_eq!(first.timing.timing_source,Some("codex_rollout_estimate"));
        let second=result.records.iter().find(|row|row.at==base+10_000).unwrap();assert!(second.timing.duration_ms.is_none(),"A token usage gap alone is not request duration");assert!(second.timing.output_tokens_per_second.is_none());
        let third=result.records.iter().find(|row|row.at==base+15_000).unwrap();assert_eq!(third.timing.duration_ms,Some(4_000));assert_eq!(third.timing.ttft_ms,Some(1_000));assert!(result.records.iter().all(|row|row.timing.http_status.is_none()));
        let fourth=result.records.iter().find(|row|row.at==base+20_000).unwrap();assert_eq!(fourth.timing.duration_ms,Some(4_000));assert_eq!(fourth.timing.ttft_ms,Some(1_000));
        let json=serde_json::to_string(&result).unwrap();for private in ["PRIVATE_CONVERSATION","PRIVATE_REASONING","PRIVATE_OUTPUT","PRIVATE_TOOL_OUTPUT","PRIVATE_COMMAND_OUTPUT","timing-thread","turn-a","response-a"]{assert!(!json.contains(private));}
    }
    #[test]fn unsupported_legacy_aborted_forked_and_mismatched_turns_keep_timing_unknown(){
        let mut file=FileCache::default();add(&mut file,meta("safe-timing"));let base=timestamp("2026-11-12T00:00:00Z").unwrap();
        add(&mut file,serde_json::json!({"timestamp":"2026-11-12T00:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"old-turn"}}));
        add(&mut file,token("2026-11-12T00:00:02Z",100,80,10,Some((100,80,10))));assert!(file.events.last().unwrap().timing.duration_ms.is_none());
        add(&mut file,serde_json::json!({"timestamp":"2026-11-12T00:00:03Z","type":"event_msg","payload":{"type":"turn_aborted","turn_id":"old-turn"}}));
        add(&mut file,serde_json::json!({"timestamp":"2026-11-12T00:02:00Z","type":"token_usage_record","payload":{"thread_id":"safe-timing","turn_id":"new-turn","response_id":"resumed","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}}));assert!(file.events.last().unwrap().timing.duration_ms.is_none());
        let mut timer=ResponseTiming{turn:Some("turn-a".into()),boundary:Some(base),start:Some(base),first:Some(base+1_000),last_stamp:Some(base+2_000),..ResponseTiming::default()};assert!(timer.finish(base+3_000,Some("parallel-turn"),10).duration_ms.is_none());
        let mut missing_first=ResponseTiming{turn:Some("turn-a".into()),boundary:Some(base),..ResponseTiming::default()};assert!(missing_first.finish(base+3_000,Some("turn-a"),10).duration_ms.is_none(),"A task boundary alone does not authenticate a per-response timing interval");
        let mut overlapping=ResponseTiming{turn:Some("still-active".into()),boundary:Some(base),..ResponseTiming::default()};let parallel:Envelope=serde_json::from_value(serde_json::json!({"timestamp":"2026-11-12T00:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"parallel"}})).unwrap();overlapping.observe(&parallel,Some("safe-timing"));assert!(overlapping.finish(base+3_000,Some("parallel"),10).duration_ms.is_none(),"Overlapping turns are not silently treated as sequential calls");
        let mut fork=FileCache::default();add(&mut fork,serde_json::json!({"timestamp":"2026-11-12T01:00:00Z","type":"session_meta","payload":{"id":"child-timing","forked_from_id":"parent","timestamp":"2026-11-12T01:00:00Z"}}));
        add(&mut fork,serde_json::json!({"timestamp":"2026-11-12T00:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"copied-turn"}}));assert!(fork.timing.turn.is_none(),"Copied history must not seed the child's timer");
        add(&mut fork,serde_json::json!({"timestamp":"2026-11-12T01:00:01Z","type":"token_usage_record","payload":{"thread_id":"child-timing","turn_id":"copied-turn","response_id":"fresh","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}}));assert!(fork.events.last().unwrap().timing.duration_ms.is_none());
    }
    #[test]fn narrower_dates_keep_the_broad_cache_and_report_real_scan_progress(){
        let root=std::env::temp_dir().join(format!("codex-usage-date-cache-{}-{}",std::process::id(),crate::settings::now()));fs::create_dir_all(root.join("sessions")).unwrap();let path=root.join("sessions/a.jsonl");
        fs::write(&path,format!("{}\n{}\n{}\n",meta("range-cache"),token("2026-11-11T23:30:00Z",100,20,10,Some((100,20,10))),token("2026-11-12T01:00:00Z",150,30,15,Some((50,10,5))))).unwrap();
        let(start,end)=range();let mut cache=Cache::default();let broad=read_at(&root,start-DAY,end,&mut cache).unwrap();let bytes=broad.scan_progress.bytes_read;
        assert_eq!(broad.summary.request_count,2);assert_eq!(broad.scan_progress.files_complete,1);assert_eq!(bytes,broad.scan_progress.bytes_total);
        let narrow=read_at(&root,start,end,&mut cache).unwrap();assert_eq!(narrow.summary.request_count,1);assert_eq!(cache.floor,start-DAY);assert_eq!(cache.files[&path].events.len(),2);assert_eq!(narrow.scan_progress.bytes_read,bytes);
        assert_eq!(read_at(&root,start-DAY,end,&mut cache).unwrap().summary.request_count,2);assert!(!narrow.scan_pending);fs::remove_dir_all(root).unwrap();
    }
    #[test]fn incremental_reader_handles_partial_tail_truncation_and_oversized_content(){use std::io::Write;let root=std::env::temp_dir().join(format!("codex-usage-fixture-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(root.join("sessions")).unwrap();let path=root.join("sessions/a.jsonl");let mut file=File::create(&path).unwrap();writeln!(file,"{}",meta("reader")).unwrap();writeln!(file,"{{\"type\":\"response_item\",\"payload\":{{\"content\":\"{}\"}}}}","x".repeat(MAX_LINE*2)).unwrap();let usage=serde_json::to_vec(&token("2026-11-12T01:00:00Z",100,80,10,Some((100,80,10)))).unwrap();file.write_all(&usage[..usage.len()/2]).unwrap();file.flush().unwrap();let(start,end)=range();let mut cache=Cache::default();let first=read_at(&root,start,end,&mut cache).unwrap();assert_eq!(first.summary.request_count,0);assert_eq!(first.partial_tail_files,1);assert!(first.partial);assert!(first.notes.iter().any(|note|note.contains("尚未写入完成")));file.write_all(&usage[usage.len()/2..]).unwrap();file.write_all(b"\n").unwrap();file.flush().unwrap();let second=read_at(&root,start,end,&mut cache).unwrap();assert_eq!(second.summary.tokens.total_tokens,110);assert_eq!(second.skipped_records,0);assert!(!second.partial);assert_eq!(read_at(&root,start,end,&mut cache).unwrap().summary.request_count,1);drop(file);fs::write(&path,format!("{}\n{}\n",meta("reader"),token("2026-11-12T02:00:00Z",20,10,2,Some((20,10,2))))).unwrap();assert_eq!(read_at(&root,start,end,&mut cache).unwrap().summary.tokens.total_tokens,22);assert!(root.starts_with(std::env::temp_dir()));fs::remove_dir_all(root).unwrap();}
}
