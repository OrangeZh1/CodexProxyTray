//! Windows-only ETW engine. This module does nothing until EtwSession::start is called.
use std::{collections::HashMap,mem::size_of,sync::{Arc,RwLock,atomic::{AtomicBool,AtomicU32,AtomicU64,Ordering}},thread::{self,JoinHandle}};
use windows::{core::{GUID,PCWSTR,PWSTR},Win32::{Foundation::{GetLastError,ERROR_ACCESS_DENIED,ERROR_PRIVILEGE_NOT_HELD,ERROR_NOT_ALL_ASSIGNED,WAIT_TIMEOUT,WAIT_OBJECT_0,FILETIME},Security::{AdjustTokenPrivileges,LookupPrivilegeValueW,TOKEN_PRIVILEGES,LUID_AND_ATTRIBUTES,TOKEN_QUERY,TOKEN_ADJUST_PRIVILEGES,SE_PRIVILEGE_ENABLED,SE_SYSTEM_PROFILE_NAME},System::{Diagnostics::Etw::*,Threading::{GetCurrentProcess,OpenProcessToken,WaitForSingleObject,GetProcessTimes}}}};
use crate::traffic::{OwnedHandle,open_identity,process_sid};
use crate::traffic_core::{self,ProcessIdentity,Direction,Decode};

pub struct EngineError{pub code:u32,pub message:&'static str}
impl EngineError{pub fn needs_permission(&self)->bool{matches!(self.code,5|1300|1314)}}
struct BoundProcess{identity:ProcessIdentity,handle:OwnedHandle,active:bool,exited:AtomicU64}
struct State{owner_sid:String,bindings:RwLock<HashMap<u32,Vec<BoundProcess>>>,sent:AtomicU64,received:AtomicU64,seen:AtomicBool,had_targets:AtomicBool,partial:AtomicBool,unsupported:AtomicBool,overflow:AtomicBool,consumer_error:AtomicU32,binding_error:AtomicU32}
impl State{fn new(owner_sid:String)->Self{Self{owner_sid,bindings:RwLock::new(HashMap::new()),sent:AtomicU64::new(0),received:AtomicU64::new(0),seen:AtomicBool::new(false),had_targets:AtomicBool::new(false),partial:AtomicBool::new(false),unsupported:AtomicBool::new(false),overflow:AtomicBool::new(false),consumer_error:AtomicU32::new(0),binding_error:AtomicU32::new(0)}}}
pub struct EngineSample{pub sent:u64,pub received:u64,pub seen:bool,pub had_targets:bool,pub partial:bool,pub target_count:u32,pub lost_events:Option<u64>,pub lost_buffers:Option<u64>,pub error:Option<EngineError>}
const NAME_UNITS:usize=1025;
fn output_properties(storage:&mut [u64])->*mut EVENT_TRACE_PROPERTIES{unsafe{
    let guid=(*(storage.as_ptr()as *const EVENT_TRACE_PROPERTIES)).Wnode.Guid;storage.fill(0);let props=storage.as_mut_ptr()as *mut EVENT_TRACE_PROPERTIES;
    (*props).Wnode.BufferSize=(storage.len()*8)as u32;(*props).Wnode.Guid=guid;(*props).Wnode.Flags=WNODE_FLAG_TRACED_GUID;
    (*props).LoggerNameOffset=size_of::<EVENT_TRACE_PROPERTIES>()as u32;(*props).LogFileNameOffset=(size_of::<EVENT_TRACE_PROPERTIES>()+NAME_UNITS*2)as u32;props
}}
unsafe fn stop_owned(control:CONTROLTRACE_HANDLE,properties:&mut [u64]){
    // A controller process exiting does not stop a system logger. Keep the dedicated owner alive
    // until its own STOP succeeds; the GUI reports stopping/unavailable instead of claiming disabled.
    loop{let code=unsafe{ControlTraceW(control,PCWSTR::null(),output_properties(properties),EVENT_TRACE_CONTROL_STOP)};if matches!(code.0,0|4201|234){return}thread::sleep(std::time::Duration::from_secs(1));}
}
fn exited_at(process:&BoundProcess)->Option<u64>{unsafe{
    let cached=process.exited.load(Ordering::Relaxed);if cached!=0{return Some(cached)}
    let(mut created,mut exited,mut kernel,mut user)=(FILETIME::default(),FILETIME::default(),FILETIME::default(),FILETIME::default());GetProcessTimes(process.handle.0,&mut created,&mut exited,&mut kernel,&mut user).ok()?;
    let ended=((exited.dwHighDateTime as u64)<<32)|exited.dwLowDateTime as u64;if ended==0{return None}process.exited.store(ended,Ordering::Relaxed);Some(ended)
}}

/// Holds the session token privilege only if it was already assigned to this process.
/// It never grants rights, changes groups, policies, another process token or any session ACL.
struct ProfilePrivilege{token:OwnedHandle,previous:TOKEN_PRIVILEGES}
impl ProfilePrivilege{
    fn enable()->Option<Self>{unsafe{
        let mut raw=Default::default();OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY|TOKEN_ADJUST_PRIVILEGES,&mut raw).ok()?;let token=OwnedHandle(raw);
        let mut luid=Default::default();LookupPrivilegeValueW(PCWSTR::null(),SE_SYSTEM_PROFILE_NAME,&mut luid).ok()?;
        let requested=TOKEN_PRIVILEGES{PrivilegeCount:1,Privileges:[LUID_AND_ATTRIBUTES{Luid:luid,Attributes:SE_PRIVILEGE_ENABLED}]};let mut previous=TOKEN_PRIVILEGES::default();let mut returned=0;
        AdjustTokenPrivileges(token.0,false,Some(&requested),size_of::<TOKEN_PRIVILEGES>()as u32,Some(&mut previous),Some(&mut returned)).ok()?;
        if GetLastError()==ERROR_NOT_ALL_ASSIGNED{return None}Some(Self{token,previous})
    }}
}
impl Drop for ProfilePrivilege{fn drop(&mut self){unsafe{let _=AdjustTokenPrivileges(self.token.0,false,Some(&self.previous),0,None,None);}}}

unsafe extern "system" fn record_callback(record:*mut EVENT_RECORD){
    if record.is_null(){return}let record=unsafe{&*record};if record.UserContext.is_null(){return}
    let state=unsafe{&*(record.UserContext as *const State)};
    let provider=record.EventHeader.ProviderId.to_u128();
    // Do not create a slice over addresses, ports, connection IDs or packet contents.
    let fields=if record.UserDataLength>=8&&!record.UserData.is_null(){unsafe{std::slice::from_raw_parts(record.UserData as *const u8,8)}}else{&[]};
    match traffic_core::decode(provider,record.EventHeader.EventDescriptor.Version,record.EventHeader.EventDescriptor.Opcode,fields){
        Decode::Ignore=>{},Decode::Unsupported=>{state.unsupported.store(true,Ordering::Relaxed);},
        Decode::Packet(packet)=>{
            let Ok(bindings)=state.bindings.read()else{return};let Some(lifetimes)=bindings.get(&packet.pid)else{return};
            // ProcessTrace converts QPC to FILETIME because RAW_TIMESTAMP is deliberately absent.
            // Retained handles anchor identities, including exited processes whose ETW buffers arrive later.
            if record.EventHeader.TimeStamp<0{return}let timestamp=record.EventHeader.TimeStamp as u64;
            let owned=lifetimes.iter().any(|process|{
                let wait=unsafe{WaitForSingleObject(process.handle.0,0)};
                if wait==WAIT_TIMEOUT{return process.active&&traffic_core::within_lifetime(process.identity.created_filetime,None,timestamp)}
                if wait!=WAIT_OBJECT_0{state.binding_error.store(6,Ordering::Relaxed);return false}
                match exited_at(process){Some(exited)=>traffic_core::within_lifetime(process.identity.created_filetime,Some(exited),timestamp),None=>{state.binding_error.store(6,Ordering::Relaxed);false}}
            });if !owned{return}
            let counter=match packet.direction{Direction::Send=>&state.sent,Direction::Receive=>&state.received};
            if counter.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|value|value.checked_add(packet.bytes as u64)).is_err(){state.overflow.store(true,Ordering::Relaxed);}
            state.seen.store(true,Ordering::Relaxed);
        }
    }
}

pub struct EtwSession{
    control:CONTROLTRACE_HANDLE,consumer:PROCESSTRACE_HANDLE,properties:Vec<u64>,_name:Vec<u16>,_logfile:Box<EVENT_TRACE_LOGFILEW>,state:Arc<State>,context:*const State,thread:Option<JoinHandle<()>>,_privilege:Option<ProfilePrivilege>,
}
impl EtwSession{
    pub fn start(name:&str,guid:GUID,targets:&[ProcessIdentity],owner_sid:String)->Result<Self,EngineError>{unsafe{
        let mut wide=name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        if wide.len()>NAME_UNITS{return Err(EngineError{code:87,message:"采样会话名称过长。"})}
        // ControlTrace returns both the logger name and log filename, even for a realtime logger.
        let bytes=size_of::<EVENT_TRACE_PROPERTIES>()+NAME_UNITS*4;
        // u64 alignment is sufficient for the generated SDK struct on supported Windows x64.
        let mut properties=vec![0u64;bytes.div_ceil(8)];let props=properties.as_mut_ptr()as *mut EVENT_TRACE_PROPERTIES;
        *props=EVENT_TRACE_PROPERTIES::default();(*props).Wnode.BufferSize=(properties.len()*8)as u32;(*props).Wnode.Flags=WNODE_FLAG_TRACED_GUID;(*props).Wnode.Guid=guid;(*props).Wnode.ClientContext=1;
        (*props).BufferSize=64;(*props).MinimumBuffers=8;(*props).MaximumBuffers=64;(*props).FlushTimer=1;
        (*props).LogFileMode=EVENT_TRACE_REAL_TIME_MODE|EVENT_TRACE_SYSTEM_LOGGER_MODE;(*props).EnableFlags=EVENT_TRACE_FLAG_NETWORK_TCPIP;(*props).LoggerNameOffset=size_of::<EVENT_TRACE_PROPERTIES>()as u32;(*props).LogFileNameOffset=0;
        std::ptr::copy_nonoverlapping(wide.as_ptr(),(properties.as_mut_ptr()as *mut u8).add(size_of::<EVENT_TRACE_PROPERTIES>())as *mut u16,wide.len());
        let mut control=CONTROLTRACE_HANDLE::default();let mut code=StartTraceW(&mut control,PCWSTR(wide.as_ptr()),props);let mut privilege=None;
        if code==ERROR_ACCESS_DENIED||code==ERROR_PRIVILEGE_NOT_HELD{privilege=ProfilePrivilege::enable();if privilege.is_some(){code=StartTraceW(&mut control,PCWSTR(wide.as_ptr()),props);}}
        // In particular, ERROR_ALREADY_EXISTS is not ours and must never trigger ControlTrace STOP.
        if code.0!=0{return Err(EngineError{code:code.0,message:"Windows 系统网络采样未能启动。"})}
        let state=Arc::new(State::new(owner_sid));let context=Arc::into_raw(state.clone());let mut logfile=Box::new(EVENT_TRACE_LOGFILEW::default());
        logfile.LoggerName=PWSTR(wide.as_mut_ptr());logfile.Context=context as *mut _;logfile.Anonymous1.ProcessTraceMode=PROCESS_TRACE_MODE_EVENT_RECORD|PROCESS_TRACE_MODE_REAL_TIME;logfile.Anonymous2.EventRecordCallback=Some(record_callback);
        let consumer=OpenTraceW(&mut *logfile);
        if consumer.Value==0||consumer.Value==u64::MAX{let error=GetLastError().0;stop_owned(control,&mut properties);drop(Arc::from_raw(context));return Err(EngineError{code:error,message:"Windows 网络事件消费者未能启动。"})}
        let mut session=Self{control,consumer,properties,_name:wide,_logfile:logfile,state,context,thread:None,_privilege:privilege};
        // Install the initial whitelist before ProcessTrace can deliver buffered transfer events.
        if session.set_targets(targets).is_err(){let _=session.stop();return Err(EngineError{code:87,message:"网络统计目标身份无效。"})}
        let callback_state=session.state.clone();let thread=match thread::Builder::new().name("traffic-etw".into()).spawn(move||{
            let result=ProcessTrace(&[consumer],None,None);if result.0!=0&&result.0!=1223{callback_state.consumer_error.store(result.0,Ordering::Relaxed);}
        }){Ok(thread)=>thread,Err(_)=>{let _=session.stop();return Err(EngineError{code:8,message:"网络事件消费线程未能启动。"})}};
        session.thread=Some(thread);Ok(session)
    }}
    pub fn set_targets(&mut self,targets:&[ProcessIdentity])->Result<(),String>{
        traffic_core::validate_targets(targets)?;let mut bindings=self.state.bindings.write().map_err(|_|"网络统计进程名单不可用。")?;let mut fault=0;
        for list in bindings.values_mut(){for process in list{process.active=false;}}
        for &identity in targets{
            if let Some(existing)=bindings.get_mut(&identity.pid).and_then(|list|list.iter_mut().find(|process|process.identity==identity)){existing.active=true;continue}
            // Bound lifetime history is kept for delayed ETW buffers. Fail closed at a finite session limit.
            if bindings.values().map(Vec::len).sum::<usize>()>=4096{fault=8;continue}
            match open_identity(identity){Ok(Some(handle))=>{if process_sid(handle.0).ok().as_deref()!=Some(self.state.owner_sid.as_str()){fault=1307;continue}bindings.entry(identity.pid).or_default().push(BoundProcess{identity,handle,active:true,exited:AtomicU64::new(0)});self.state.had_targets.store(true,Ordering::Relaxed);},Ok(None)=>{
                // Process lists cross an authenticated pipe: normal process churn can
                // invalidate an identity before it reaches this consumer. Never bind
                // its reused PID, but keep counting other verified handles. The
                // cumulative result stays explicitly partial until the next session.
                self.state.partial.store(true,Ordering::Relaxed);
            },Err(error)=>{fault=error.code().0 as u32&0xffff;}}
        }
        // Discard still-live processes removed from the verified scope; keep only exited tombstones.
        for list in bindings.values_mut(){list.retain(|process|process.active||unsafe{WaitForSingleObject(process.handle.0,0)}==WAIT_OBJECT_0);}bindings.retain(|_,list|!list.is_empty());
        // True identity/permission/history failures remain fatal for this session;
        // a later good target list cannot make the missed cumulative bytes complete.
        if fault!=0{self.state.binding_error.store(fault,Ordering::Relaxed);}Ok(())
    }
    pub fn sample(&mut self)->EngineSample{unsafe{
        let props=output_properties(&mut self.properties);let code=ControlTraceW(self.control,PCWSTR::null(),props,EVENT_TRACE_CONTROL_QUERY);
        let(lost_events,lost_buffers)=if code.0==0{(Some((*props).EventsLost as u64),Some((*props).LogBuffersLost as u64+(*props).RealTimeBuffersLost as u64))}else{(None,None)};
        let error=sample_error(&self.state,code.0,lost_events,lost_buffers);
        let count=self.state.bindings.read().map(|bindings|bindings.values().flatten().filter(|process|process.active&&WaitForSingleObject(process.handle.0,0)==WAIT_TIMEOUT).count()as u32).unwrap_or(0);
        EngineSample{sent:self.state.sent.load(Ordering::Relaxed),received:self.state.received.load(Ordering::Relaxed),seen:self.state.seen.load(Ordering::Relaxed),had_targets:self.state.had_targets.load(Ordering::Relaxed),partial:self.state.partial.load(Ordering::Relaxed),target_count:count,lost_events,lost_buffers,error}
    }}
    pub fn stop(&mut self)->Result<(),EngineError>{unsafe{
        if self.control.Value!=0{stop_owned(self.control,&mut self.properties);self.control.Value=0;}
        if self.consumer.Value!=0&&self.consumer.Value!=u64::MAX{let _=CloseTrace(self.consumer);self.consumer.Value=u64::MAX;}
        // CloseTrace can return ERROR_CTX_CLOSE_PENDING: do not free names/Context until the consumer really returns.
        if let Some(thread)=self.thread.take(){let _=thread.join();}
        if !self.context.is_null(){drop(Arc::from_raw(self.context));self.context=std::ptr::null();}
        Ok(())
    }}
}
impl Drop for EtwSession{fn drop(&mut self){let _=self.stop();}}

fn sample_error(state:&State,query_code:u32,lost_events:Option<u64>,lost_buffers:Option<u64>)->Option<EngineError>{
    if query_code!=0{return Some(EngineError{code:query_code,message:"无法读取本次采样的丢失事件统计。"})}
    if lost_events!=Some(0)||lost_buffers!=Some(0){return Some(EngineError{code:234,message:"系统网络事件发生丢失，当前流量不完整。"})}
    let binding_error=state.binding_error.load(Ordering::Relaxed);
    if binding_error!=0{return Some(EngineError{code:binding_error,message:"部分目标进程身份无法验证，未显示不完整流量。"})}
    if state.unsupported.load(Ordering::Relaxed){return Some(EngineError{code:50,message:"当前 Windows 网络事件格式不受支持。"})}
    if state.overflow.load(Ordering::Relaxed){return Some(EngineError{code:534,message:"网络统计计数超出范围。"})}
    let code=state.consumer_error.load(Ordering::Relaxed);
    (code!=0).then_some(EngineError{code,message:"网络事件消费者已退出。"})
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::{io::{Read,Write},net::{TcpListener,TcpStream},time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
    use std::os::windows::{io::AsRawHandle,process::CommandExt};
    fn without_trace()->EtwSession{
        EtwSession{control:CONTROLTRACE_HANDLE::default(),consumer:PROCESSTRACE_HANDLE::default(),properties:vec![],_name:vec![],_logfile:Box::new(EVENT_TRACE_LOGFILEW::default()),state:Arc::new(State::new(crate::traffic::process_sid(unsafe{GetCurrentProcess()}).unwrap())),context:std::ptr::null(),thread:None,_privilege:None}
    }
    #[test]fn short_lived_binding_fixture(){
        if std::env::var_os("CODEX_TRAFFIC_BINDING_FIXTURE").is_some(){let _=std::io::stdin().read_exact(&mut [0u8;1]);}
    }
    fn child_fixture()->std::process::Child{
        std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","traffic_etw::tests::short_lived_binding_fixture"]).env("CODEX_TRAFFIC_BINDING_FIXTURE","1").creation_flags(0x08000000).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap()
    }
    fn release_child(child:&mut std::process::Child){
        child.stdin.take().unwrap().write_all(b"x").unwrap();assert!(child.wait().unwrap().success());
    }
    #[test]fn expired_bindings_leave_live_targets_usable_and_explicitly_partial(){
        let mut child=child_fixture();let expired=crate::traffic::identity_on(windows::Win32::Foundation::HANDLE(child.as_raw_handle())).unwrap();release_child(&mut child);
        let own=crate::traffic::identity_on(unsafe{GetCurrentProcess()}).unwrap();let mut session=without_trace();
        session.set_targets(&[own,expired]).unwrap();assert!(session.state.partial.load(Ordering::Relaxed));assert_eq!(session.state.binding_error.load(Ordering::Relaxed),0);
        let bindings=session.state.bindings.read().unwrap();assert!(bindings.contains_key(&own.pid));assert!(!bindings.contains_key(&expired.pid));drop(bindings);
        session.set_targets(&[own]).unwrap();assert!(session.state.partial.load(Ordering::Relaxed),"Earlier skipped bytes cannot become complete when a later list succeeds");
        assert!(sample_error(&session.state,0,Some(0),Some(0)).is_none());
        let mut clean=without_trace();clean.set_targets(&[own]).unwrap();assert!(!clean.state.partial.load(Ordering::Relaxed));
    }
    #[test]fn stale_pid_identity_is_never_bound_to_the_replacement(){
        let own=crate::traffic::identity_on(unsafe{GetCurrentProcess()}).unwrap();let stale=ProcessIdentity{created_filetime:own.created_filetime-1,..own};let mut session=without_trace();
        session.set_targets(&[stale]).unwrap();assert!(session.state.bindings.read().unwrap().is_empty());assert!(session.state.partial.load(Ordering::Relaxed));
        session.set_targets(&[own]).unwrap();let bindings=session.state.bindings.read().unwrap();assert_eq!(bindings[&own.pid].len(),1);assert_eq!(bindings[&own.pid][0].identity,own);
    }
    #[test]fn late_events_use_retained_exit_times_and_cannot_cross_pid_lifetimes(){
        let mut child=child_fixture();let identity=crate::traffic::identity_on(windows::Win32::Foundation::HANDLE(child.as_raw_handle())).unwrap();let mut session=without_trace();session.set_targets(&[identity]).unwrap();release_child(&mut child);session.set_targets(&[]).unwrap();
        let exited={let bindings=session.state.bindings.read().unwrap();exited_at(&bindings[&identity.pid][0]).unwrap()};
        let mut fields=[identity.pid.to_le_bytes(),128u32.to_le_bytes()].concat();let mut record=EVENT_RECORD::default();record.UserContext=Arc::as_ptr(&session.state)as *mut _;record.UserData=fields.as_mut_ptr().cast();record.UserDataLength=fields.len()as u16;record.EventHeader.ProviderId=GUID::from_u128(traffic_core::TCP_PROVIDER);record.EventHeader.EventDescriptor.Version=2;record.EventHeader.EventDescriptor.Opcode=10;
        record.EventHeader.TimeStamp=(identity.created_filetime+(exited-identity.created_filetime)/2)as i64;unsafe{record_callback(&mut record)};assert_eq!(session.state.sent.load(Ordering::Relaxed),128);
        record.EventHeader.TimeStamp=(exited+1)as i64;unsafe{record_callback(&mut record)};record.EventHeader.TimeStamp=(identity.created_filetime-1)as i64;unsafe{record_callback(&mut record)};assert_eq!(session.state.sent.load(Ordering::Relaxed),128);
        assert!(!session.state.partial.load(Ordering::Relaxed),"A retained exact lifetime handles delayed buffers without losing scope");
    }
    #[test]fn partial_scope_never_hides_permission_loss_or_real_engine_failure(){
        let state=State::new("fixture".into());state.partial.store(true,Ordering::Relaxed);assert!(sample_error(&state,0,Some(0),Some(0)).is_none());
        assert_eq!(sample_error(&state,0,Some(1),Some(0)).unwrap().code,234);assert_eq!(sample_error(&state,5,None,None).unwrap().code,5);
        state.binding_error.store(5,Ordering::Relaxed);assert_eq!(sample_error(&state,0,Some(0),Some(0)).unwrap().code,5);state.binding_error.store(0,Ordering::Relaxed);
        state.unsupported.store(true,Ordering::Relaxed);assert_eq!(sample_error(&state,0,Some(0),Some(0)).unwrap().code,50);state.unsupported.store(false,Ordering::Relaxed);
        state.overflow.store(true,Ordering::Relaxed);assert_eq!(sample_error(&state,0,Some(0),Some(0)).unwrap().code,534);state.overflow.store(false,Ordering::Relaxed);
        state.consumer_error.store(6,Ordering::Relaxed);assert_eq!(sample_error(&state,0,Some(0),Some(0)).unwrap().code,6);
    }
    fn transfer_loopback(bytes:usize){
        let timeout=Duration::from_secs(2);let listener=TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).expect("bind isolated CI loopback socket");
        listener.set_nonblocking(true).unwrap();
        // Queue the completed connection before accept: the fixture needs no extra thread.
        let mut client=TcpStream::connect_timeout(&listener.local_addr().unwrap(),timeout).expect("connect isolated CI loopback socket");
        let deadline=Instant::now()+timeout;
        let mut server=loop{match listener.accept(){Ok((socket,_))=>break socket,Err(error)if error.kind()==std::io::ErrorKind::WouldBlock&&Instant::now()<deadline=>thread::sleep(Duration::from_millis(5)),Err(error)=>panic!("accept isolated CI loopback socket: {error}")}};
        server.set_nonblocking(false).unwrap();
        for socket in [&client,&server]{socket.set_read_timeout(Some(timeout)).unwrap();socket.set_write_timeout(Some(timeout)).unwrap();}
        let block=[0x5au8;32*1024];let mut received=[0u8;32*1024];
        let mut left=bytes;
        while left!=0{let count=left.min(block.len());client.write_all(&block[..count]).expect("CI loopback client write");server.read_exact(&mut received[..count]).expect("CI loopback server read");assert_eq!(&received[..count],&block[..count]);server.write_all(&block[..count]).expect("CI loopback server write");client.read_exact(&mut received[..count]).expect("CI loopback client read");assert_eq!(&received[..count],&block[..count]);left-=count;}
    }
    #[test]
    #[ignore="Real ETW smoke: only the explicit isolated GitHub Windows job may run it"]
    fn etw_loopback_counts_only_verified_targets_and_stops(){
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),"Real ETW smoke is restricted to the explicitly authorized GitHub runner");
        let identity=crate::traffic::identity_on(unsafe{GetCurrentProcess()}).expect("read CI process identity");
        let owner=crate::traffic::process_sid(unsafe{GetCurrentProcess()}).expect("read CI process SID");
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();let guid=GUID::from_u128(nonce^((identity.pid as u128)<<96)^0xb402d5b6_612f_4a5c_916e_997e203fead8u128);
        let mut session=EtwSession::start(&format!("CodexProxyTray.CI.{}.{}",identity.pid,nonce),guid,&[],owner).unwrap_or_else(|error|panic!("ETW start failed: {} ({})",error.message,error.code));
        transfer_loopback(32*1024);thread::sleep(Duration::from_secs(2));
        let unowned=session.sample();assert!(unowned.error.is_none(),"Unowned ETW sample failed: {}",unowned.error.as_ref().map(|error|format!("{} ({})",error.message,error.code)).unwrap_or_default());assert_eq!((unowned.sent,unowned.received),(0,0));assert!(!unowned.seen);assert_eq!(unowned.lost_events,Some(0));assert_eq!(unowned.lost_buffers,Some(0));
        session.set_targets(&[identity]).expect("bind CI process as the sole verified target");transfer_loopback(512*1024);
        let deadline=Instant::now()+Duration::from_secs(8);
        loop{
            let sample=session.sample();assert!(sample.error.is_none(),"Owned ETW sample failed: {}",sample.error.as_ref().map(|error|format!("{} ({})",error.message,error.code)).unwrap_or_default());assert_eq!(sample.lost_events,Some(0));assert_eq!(sample.lost_buffers,Some(0));
            if sample.seen&&sample.sent>0&&sample.received>0{break}
            assert!(Instant::now()<deadline,"The real Windows ETW session did not observe both loopback send and receive bytes for the verified process");thread::sleep(Duration::from_millis(100));
        }
        session.stop().unwrap_or_else(|error|panic!("ETW STOP was not confirmed: {} ({})",error.message,error.code));assert_eq!(session.control.Value,0);assert_eq!(session.consumer.Value,u64::MAX);assert!(session.thread.is_none());assert!(session.context.is_null());
    }
}
