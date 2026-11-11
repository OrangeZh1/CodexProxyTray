//! Opt-in process traffic coordinator and authenticated collector IPC.
//! No listener, collector, privilege request or ETW session is created by new().
use std::{ffi::{OsStr,OsString},mem::size_of,os::windows::ffi::{OsStrExt,OsStringExt},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},thread::{self,JoinHandle},time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
use serde::{Serialize,Deserialize};
use windows::{core::{GUID,PCWSTR,PWSTR},Win32::{Foundation::*,Security::{GetTokenInformation,TOKEN_QUERY,TOKEN_USER,TokenUser,SECURITY_ATTRIBUTES,PSECURITY_DESCRIPTOR},Security::{Authorization::{ConvertSidToStringSidW,ConvertStringSecurityDescriptorToSecurityDescriptorW},Cryptography::{BCryptGenRandom,BCRYPT_USE_SYSTEM_PREFERRED_RNG}},Storage::FileSystem::*,System::{Pipes::*,Threading::*}}};
use crate::{traffic_core::{self,ProcessIdentity},traffic_etw::{EtwSession,EngineError}};

const PROTOCOL:u32=2;const MAX_FRAME:usize=32768;const PIPE_PREFIX:&str=r"\\.\pipe\CodexProxyTray.Traffic.";
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]#[serde(rename_all="snake_case")]
pub enum TrafficState{Disabled,Starting,Collecting,NeedsInstallation,NeedsPermission,Unavailable}
#[derive(Clone,Debug,Serialize,Deserialize)]#[serde(deny_unknown_fields)]
pub struct TrafficSnapshot{
    pub state:TrafficState,pub timestamp_ms:u64,pub received_bytes:Option<u64>,pub sent_bytes:Option<u64>,pub receive_bps:Option<f64>,pub send_bps:Option<f64>,pub target_count:u32,pub lost_events:Option<u64>,pub lost_buffers:Option<u64>,pub message:Option<String>,pub scope:String,
}
impl TrafficSnapshot{
    fn state(state:TrafficState,message:Option<String>)->Self{Self{state,timestamp_ms:now(),received_bytes:None,sent_bytes:None,receive_bps:None,send_bps:None,target_count:0,lost_events:None,lost_buffers:None,message,scope:"current_verified_processes_since_enable".into()}}
    pub fn disabled()->Self{Self::state(TrafficState::Disabled,None)}
}
#[derive(Clone,Copy)]pub enum EnableMode{Normal}
#[derive(Serialize,Deserialize)]#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
enum ToCollector{Targets{protocol:u32,targets:Vec<ProcessIdentity>},Stop{protocol:u32},StoppedReceived{protocol:u32}}
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct FromCollector{protocol:u32,snapshot:TrafficSnapshot,stopped:bool}
struct Shared{snapshot:Mutex<TrafficSnapshot>,targets:Mutex<(u64,Vec<ProcessIdentity>)>,target_error:Mutex<Option<String>>,stop_confirmed:AtomicBool}
struct Running{stop:Arc<AtomicBool>,thread:JoinHandle<()>}
pub struct Coordinator{shared:Arc<Shared>,running:Option<Running>}
impl Coordinator{
    pub fn new()->Self{Self{shared:Arc::new(Shared{snapshot:Mutex::new(TrafficSnapshot::disabled()),targets:Mutex::new((0,vec![])),target_error:Mutex::new(None),stop_confirmed:AtomicBool::new(true)}),running:None}}
    pub fn snapshot(&self)->TrafficSnapshot{let raw=self.shared.snapshot.lock().map(|value|value.clone()).unwrap_or_else(|_|TrafficSnapshot::state(TrafficState::Unavailable,Some("流量统计状态不可用。".into())));if raw.state!=TrafficState::Disabled{if let Some(error)=self.shared.target_error.lock().ok().and_then(|error|error.clone()){return TrafficSnapshot::state(TrafficState::Unavailable,Some(error))}}raw}
    pub fn set_targets(&self,targets:Vec<ProcessIdentity>)->Result<(),String>{traffic_core::validate_targets(&targets)?;{let mut current=self.shared.targets.lock().map_err(|_|"流量统计进程名单不可用。")?;if current.1!=targets{current.0=current.0.wrapping_add(1);current.1=targets;}}*self.shared.target_error.lock().map_err(|_|"流量统计进程名单状态不可用。")?=None;Ok(())}
    pub fn fail_targets(&self,message:String){if let Ok(mut current)=self.shared.targets.lock(){current.0=current.0.wrapping_add(1);current.1.clear();}if let Ok(mut error)=self.shared.target_error.lock(){*error=Some(message.clone());}put(&self.shared,TrafficSnapshot::state(TrafficState::Unavailable,Some(message)));}
    /// Normal uses the previously installed helper service. It never installs or requests UAC.
    pub fn enable(&mut self,mode:EnableMode)->Result<(),String>{
        self.disable()?;put(&self.shared,TrafficSnapshot::state(TrafficState::Starting,Some("正在建立本用户的进程流量采样。".into())));
        let shared=self.shared.clone();let stop=Arc::new(AtomicBool::new(false));let signal=stop.clone();
        let thread=thread::Builder::new().name("traffic-ipc".into()).spawn(move||{if let Err(message)=relay(&shared,&signal,mode){put(&shared,TrafficSnapshot::state(TrafficState::Unavailable,Some(message)));}}).map_err(|_|"流量统计通信线程未能启动。")?;
        self.running=Some(Running{stop,thread});Ok(())
    }
    pub fn disable(&mut self)->Result<(),String>{
        if let Some(running)=&self.running{running.stop.store(true,Ordering::Release);let deadline=Instant::now()+Duration::from_secs(5);while !running.thread.is_finished()&&Instant::now()<deadline{thread::sleep(Duration::from_millis(20));}if !running.thread.is_finished(){put(&self.shared,TrafficSnapshot::state(TrafficState::Unavailable,Some("正在等待本次采样进程协作退出。".into())));return Err("本次采样停止尚未确认，请稍候。".into());}}
        if let Some(running)=self.running.take(){let _=running.thread.join();}if !self.shared.stop_confirmed.load(Ordering::Acquire){put(&self.shared,TrafficSnapshot::state(TrafficState::Unavailable,Some("上次系统采样停止尚未确认。".into())));return Err("上次系统采样停止尚未确认。".into())}put(&self.shared,TrafficSnapshot::disabled());Ok(())
    }
}
impl Default for Coordinator{fn default()->Self{Self::new()}}
impl Drop for Coordinator{fn drop(&mut self){let _=self.disable();}}
fn now()->u64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()as u64}
fn put(shared:&Shared,snapshot:TrafficSnapshot){if let Ok(mut current)=shared.snapshot.lock(){*current=snapshot;}}
fn wide(value:&OsStr)->Vec<u16>{value.encode_wide().chain(Some(0)).collect()}
fn win_code(error:&windows::core::Error)->u32{error.code().0 as u32&0xffff}

pub(crate) struct OwnedHandle(pub(crate) HANDLE);
// Kernel handles are thread-safe; this wrapper alone owns CloseHandle and never exposes a borrowed handle past a lock.
unsafe impl Send for OwnedHandle{}unsafe impl Sync for OwnedHandle{}
impl Drop for OwnedHandle{fn drop(&mut self){if !self.0.is_invalid(){unsafe{let _=CloseHandle(self.0);}}}}
pub(crate) fn identity_on(handle:HANDLE)->windows::core::Result<ProcessIdentity>{unsafe{
    let(mut created,mut exited,mut kernel,mut user)=(FILETIME::default(),FILETIME::default(),FILETIME::default(),FILETIME::default());GetProcessTimes(handle,&mut created,&mut exited,&mut kernel,&mut user)?;
    Ok(ProcessIdentity{pid:GetProcessId(handle),created_filetime:((created.dwHighDateTime as u64)<<32)|created.dwLowDateTime as u64})
}}
pub(crate) fn open_identity(identity:ProcessIdentity)->windows::core::Result<Option<OwnedHandle>>{unsafe{
    let raw=match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE,false,identity.pid){Ok(raw)=>raw,Err(error)if win_code(&error)==87=>return Ok(None),Err(error)=>return Err(error)};let process=OwnedHandle(raw);
    if identity_on(process.0)?!=identity||WaitForSingleObject(process.0,0)!=WAIT_TIMEOUT{return Ok(None)}Ok(Some(process))
}}
fn own_image()->Result<PathBuf,String>{std::env::current_exe().and_then(std::fs::canonicalize).map_err(|_|"无法验证采样程序路径。".into())}
fn same_image(handle:HANDLE,expected:&Path)->bool{unsafe{let mut path=vec![0u16;32768];let mut length=path.len()as u32;if QueryFullProcessImageNameW(handle,PROCESS_NAME_WIN32,PWSTR(path.as_mut_ptr()),&mut length).is_err(){return false}path.truncate(length as usize);PathBuf::from(OsString::from_wide(&path)).canonicalize().ok().is_some_and(|actual|actual.as_os_str().to_string_lossy().eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy()))}}
pub(crate) fn process_sid(process:HANDLE)->Result<String,String>{unsafe{
    let mut raw=HANDLE::default();OpenProcessToken(process,TOKEN_QUERY,&mut raw).map_err(|_|"无法读取本用户采样身份。")?;let token=OwnedHandle(raw);let mut length=0;let _=GetTokenInformation(token.0,TokenUser,None,0,&mut length);
    if length<size_of::<TOKEN_USER>()as u32||length>65536{return Err("本用户身份长度无效。".into())}
    let mut data=vec![0u64;(length as usize).div_ceil(8)];GetTokenInformation(token.0,TokenUser,Some(data.as_mut_ptr().cast()),length,&mut length).map_err(|_|"无法读取本用户采样身份。")?;
    let user=&*(data.as_ptr()as *const TOKEN_USER);let mut sid=PWSTR::null();ConvertSidToStringSidW(user.User.Sid,&mut sid).map_err(|_|"无法创建本用户采样访问控制。")?;
    let value=sid.to_string().map_err(|_|"用户采样身份不可用。".to_owned());let _=LocalFree(Some(HLOCAL(sid.0.cast())));value
}}
fn random_token()->Result<(String,GUID),String>{unsafe{let mut bytes=[0u8;16];if BCryptGenRandom(None,&mut bytes,BCRYPT_USE_SYSTEM_PREFERRED_RNG).0<0{return Err("无法生成独立采样会话标识。".into())}let token=bytes.iter().map(|value|format!("{value:02x}")).collect::<String>();bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;Ok((token,GUID::from_u128(u128::from_be_bytes(bytes))))}}
struct Pipe{handle:OwnedHandle}
impl Pipe{
    fn server(name:&str)->Result<Self,String>{unsafe{
        let sid=process_sid(GetCurrentProcess())?;let sddl=wide(OsStr::new(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")));let mut descriptor=PSECURITY_DESCRIPTOR::default();ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()),1,&mut descriptor,None).map_err(|_|"无法设置仅本用户与受信服务可访问的采样管道。")?;
        let attributes=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>()as u32,lpSecurityDescriptor:descriptor.0,bInheritHandle:false.into()};let name=wide(OsStr::new(name));
        let raw=CreateNamedPipeW(PCWSTR(name.as_ptr()),PIPE_ACCESS_DUPLEX|FILE_FLAG_FIRST_PIPE_INSTANCE,PIPE_TYPE_MESSAGE|PIPE_READMODE_MESSAGE|PIPE_NOWAIT|PIPE_REJECT_REMOTE_CLIENTS,1,65536,65536,1000,Some(&attributes));let code=GetLastError();let _=LocalFree(Some(HLOCAL(descriptor.0)));
        if raw.is_invalid(){return Err(format!("无法创建本用户采样管道（{}）。",code.0))}Ok(Self{handle:OwnedHandle(raw)})
    }}
    fn client(name:&str,parent:&OwnedHandle)->Result<Self,String>{unsafe{
        let name=wide(OsStr::new(name));let deadline=Instant::now()+Duration::from_secs(10);
        loop{if WaitForSingleObject(parent.0,0)!=WAIT_TIMEOUT{return Err("采样父进程已退出。".into())}
            match CreateFileW(PCWSTR(name.as_ptr()),(GENERIC_READ|GENERIC_WRITE).0,FILE_SHARE_MODE(0),None,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,None){Ok(raw)=>{let pipe=Self{handle:OwnedHandle(raw)};let mode=PIPE_READMODE_MESSAGE|PIPE_NOWAIT;SetNamedPipeHandleState(pipe.handle.0,Some(&mode),None,None).map_err(|_|"无法设置采样管道模式。")?;return Ok(pipe)},Err(error)if matches!(win_code(&error),2|231)&&Instant::now()<deadline=>thread::sleep(Duration::from_millis(20)),Err(_)=>return Err("无法连接受信的采样父进程。".into())}
        }
    }}
    fn connect(&self)->Result<bool,String>{unsafe{match ConnectNamedPipe(self.handle.0,None){Ok(())=>Ok(true),Err(error)=>match win_code(&error){535=>Ok(true),536=>Ok(false),_=>Err("采样管道连接失败。".into())}}}}
    fn read<T:for<'de>Deserialize<'de>>(&self)->Result<Option<T>,String>{unsafe{
        let(mut available,mut remaining)=(0,0);PeekNamedPipe(self.handle.0,None,0,None,Some(&mut available),Some(&mut remaining)).map_err(|_|"采样连接已关闭。")?;
        if available==0{return Ok(None)}if remaining==0||remaining as usize>MAX_FRAME{return Err("采样消息超过允许范围。".into())}
        let mut bytes=vec![0u8;remaining as usize];let mut received=0;ReadFile(self.handle.0,Some(&mut bytes),Some(&mut received),None).map_err(|_|"采样消息读取失败。")?;bytes.truncate(received as usize);
        serde_json::from_slice(&bytes).map(Some).map_err(|_|"采样消息格式无效。".into())
    }}
    fn write<T:Serialize>(&self,value:&T,alive:impl Fn()->bool)->Result<(),String>{unsafe{
        let bytes=serde_json::to_vec(value).map_err(|_|"采样消息编码失败。")?;if bytes.len()>MAX_FRAME{return Err("采样消息超过允许范围。".into())}let deadline=Instant::now()+Duration::from_secs(3);
        loop{if !alive(){return Err("采样连接已停止。".into())}let mut written=0;match WriteFile(self.handle.0,Some(&bytes),Some(&mut written),None){Ok(())if written as usize==bytes.len()=>return Ok(()),Ok(())if written==0=>{},_=>return Err("采样消息写入失败。".into())}if Instant::now()>=deadline{return Err("采样通信超时。".into())}thread::sleep(Duration::from_millis(10));}
    }}
    fn finish(&self,value:&FromCollector,alive:impl Fn()->bool)->Result<(),String>{
        self.write(value,&alive)?;let deadline=Instant::now()+Duration::from_secs(3);
        while alive()&&Instant::now()<deadline{match self.read::<ToCollector>()?{Some(ToCollector::StoppedReceived{protocol})if protocol==PROTOCOL=>return Ok(()),Some(ToCollector::Targets{protocol,..}|ToCollector::Stop{protocol})if protocol==PROTOCOL=>{},Some(_)=>return Err("采样停止回执确认无效。".into()),None=>thread::sleep(Duration::from_millis(10))}}
        Err("助手未确认采样停止回执。".into())
    }
}

struct ChildProcess(OwnedHandle);
impl ChildProcess{fn handle(&self)->HANDLE{self.0.0}fn pid(&self)->u32{unsafe{GetProcessId(self.handle())}}fn alive(&self)->bool{unsafe{WaitForSingleObject(self.handle(),0)==WAIT_TIMEOUT}}}
fn relay(shared:&Arc<Shared>,stop:&Arc<AtomicBool>,mode:EnableMode)->Result<(),String>{
    let _=mode;let(token,_)=random_token()?;let pipe=Pipe::server(&format!("{PIPE_PREFIX}{token}"))?;let parent=unsafe{identity_on(GetCurrentProcess())}.map_err(|_|"无法验证采样父进程身份。")?;
    if stop.load(Ordering::Acquire){put(shared,TrafficSnapshot::disabled());return Ok(())}
    if crate::traffic_service::snapshot().state=="not_installed"{put(shared,TrafficSnapshot::state(TrafficState::NeedsInstallation,Some("请先安装进程流量统计辅助服务。".into())));return Ok(())}
    let child=match crate::traffic_service::start_collector(&token,parent){Ok(handle)=>ChildProcess(handle),Err(message)=>{put(shared,TrafficSnapshot::state(TrafficState::Unavailable,Some(message)));return Ok(())}};
    let deadline=Instant::now()+Duration::from_secs(15);
    while !pipe.connect()?{if !child.alive(){return Err("采样进程在连接前退出。".into())}if Instant::now()>=deadline{return Err("采样进程连接超时。".into())}thread::sleep(Duration::from_millis(20));}
    let mut client=0;unsafe{GetNamedPipeClientProcessId(pipe.handle.0,&mut client).map_err(|_|"无法验证采样客户端。")?;}
    if client!=child.pid(){return Err("采样客户端身份不匹配。".into())}crate::traffic_service::validate_collector(child.handle())?;
    let(mut version,targets)=shared.targets.lock().map_err(|_|"采样进程名单不可用。")?.clone();
    // Once a target message might have arrived, only a verified collector STOP acknowledgement
    // can prove that the system logger is gone; a child exiting is not that proof.
    shared.stop_confirmed.store(false,Ordering::Release);
    let result=(||->Result<(),String>{
        pipe.write(&ToCollector::Targets{protocol:PROTOCOL,targets},||child.alive())?;let mut last_message=Instant::now();let mut terminal=false;
        loop{
            if stop.load(Ordering::Acquire){return Ok(())}
            match accept_frame(shared,&pipe){Ok(Some(state))=>{last_message=Instant::now();terminal=matches!(state,TrafficState::NeedsPermission|TrafficState::Unavailable);},Err(_)if terminal&&shared.stop_confirmed.load(Ordering::Acquire)=>return Ok(()),Err(error)=>return Err(error),Ok(None)=>{}}
            if !child.alive(){if shared.stop_confirmed.load(Ordering::Acquire){return Ok(())}return Err("采样进程退出，但系统会话停止未确认。".into())}
            let(next,targets)=shared.targets.lock().map_err(|_|"采样进程名单不可用。")?.clone();if next!=version{pipe.write(&ToCollector::Targets{protocol:PROTOCOL,targets},||child.alive())?;version=next;}
            if last_message.elapsed()>Duration::from_secs(6){return Err("采样进程响应超时。".into())}thread::sleep(Duration::from_millis(20));
        }
    })();
    // All post-handshake errors take the same cooperative cleanup path. Never kill a trace/process.
    if !shared.stop_confirmed.load(Ordering::Acquire){
        let _=pipe.write(&ToCollector::Stop{protocol:PROTOCOL},||child.alive());let until=Instant::now()+Duration::from_secs(5);
        while child.alive()&&!shared.stop_confirmed.load(Ordering::Acquire)&&Instant::now()<until{if accept_frame(shared,&pipe).is_err(){break}thread::sleep(Duration::from_millis(20));}
    }
    if !shared.stop_confirmed.load(Ordering::Acquire){return Err("系统采样停止未确认，未开始新的采样会话。".into())}
    if stop.load(Ordering::Acquire){put(shared,TrafficSnapshot::disabled());}result
}
fn accept_frame(shared:&Shared,pipe:&Pipe)->Result<Option<TrafficState>,String>{
    let Some(frame)=pipe.read::<FromCollector>()?else{return Ok(None)};if frame.protocol!=PROTOCOL{return Err("采样协议版本无效。".into())}
    if frame.stopped{shared.stop_confirmed.store(true,Ordering::Release);let _=pipe.write(&ToCollector::StoppedReceived{protocol:PROTOCOL},||true);}let state=frame.snapshot.state;put(shared,frame.snapshot);Ok(Some(state))
}

/// Main must call this before Tauri, COM and package-debug resume handling.
/// The only accepted arguments identify a local authenticated pipe and the original GUI process.
pub fn collector_entry(args:&[OsString])->Option<i32>{
    if !args.iter().any(|arg|arg=="--traffic-collector"){return None}
    Some(match parse_collector_args(args).and_then(|(token,parent)|collect(&token,parent)){Ok(())=>0,Err(_)=>2})
}
fn parse_collector_args(args:&[OsString])->Result<(String,ProcessIdentity),String>{
    if args.len()!=7||args[0]!="--traffic-collector"||args[1]!="--pipe-token"||args[3]!="--parent-pid"||args[5]!="--parent-created"{return Err("采样参数无效。".into())}
    let token=args[2].to_str().ok_or("采样参数无效。")?;if token.len()!=32||!token.bytes().all(|value|value.is_ascii_hexdigit()){return Err("采样管道标识无效。".into())}
    let pid=args[4].to_str().and_then(|value|value.parse::<u32>().ok()).filter(|&value|value!=0).ok_or("采样父进程无效。")?;let created_filetime=args[6].to_str().and_then(|value|value.parse::<u64>().ok()).filter(|&value|value!=0).ok_or("采样父进程无效。")?;Ok((token.into(),ProcessIdentity{pid,created_filetime}))
}
fn engine_failure(error:EngineError)->TrafficSnapshot{TrafficSnapshot::state(if error.needs_permission(){TrafficState::NeedsPermission}else{TrafficState::Unavailable},Some(format!("{}（{}）",error.message,error.code)))}
fn collect(token:&str,parent_identity:ProcessIdentity)->Result<(),String>{
    let parent=open_identity(parent_identity).map_err(|_|"采样父进程不可访问。")?.ok_or("采样父进程已退出。")?;let exe=own_image()?;
    let broker=if process_sid(unsafe{GetCurrentProcess()})?=="S-1-5-18"{crate::traffic_service::validate_service_collector(parent.0)?;Some(crate::traffic_service::collector_broker()?)}else{if !same_image(parent.0,&exe){return Err("采样父程序路径不匹配。".into())}None};
    let pipe=Pipe::client(&format!("{PIPE_PREFIX}{token}"),&parent)?;let mut server=0;unsafe{GetNamedPipeServerProcessId(pipe.handle.0,&mut server).map_err(|_|"无法验证采样服务端。")?;}if server!=parent_identity.pid||identity_on(parent.0).map_err(|_|"父进程身份无效。")?!=parent_identity{return Err("采样服务端身份不匹配。".into())}
    // Writes depend only on the GUI. A dead broker must not prevent the final STOP acknowledgement.
    let alive=||unsafe{WaitForSingleObject(parent.0,0)==WAIT_TIMEOUT};let broker_alive=||broker.as_ref().is_none_or(|process|unsafe{WaitForSingleObject(process.0,0)==WAIT_TIMEOUT});let deadline=Instant::now()+Duration::from_secs(10);
    let targets=loop{if !alive(){return Ok(())}if !broker_alive(){let message="流量辅助服务已退出，未开始采样。";let _=pipe.finish(&FromCollector{protocol:PROTOCOL,snapshot:TrafficSnapshot::state(TrafficState::Unavailable,Some(message.into())),stopped:true},alive);return Err(message.into())}match pipe.read::<ToCollector>()?{Some(ToCollector::Targets{protocol,targets})if protocol==PROTOCOL=>{traffic_core::validate_targets(&targets)?;break targets},Some(ToCollector::Stop{protocol})if protocol==PROTOCOL=>{let _=pipe.finish(&FromCollector{protocol:PROTOCOL,snapshot:TrafficSnapshot::disabled(),stopped:true},alive);return Ok(())},Some(_)=>return Err("采样协议无效。".into()),None=>{if Instant::now()>=deadline{return Err("采样名单等待超时。".into())}thread::sleep(Duration::from_millis(20));}}};
    let owner_sid=process_sid(parent.0)?;let(_,guid)=random_token()?;let mut session=match EtwSession::start(&format!("CodexProxyTray.Network.{token}"),guid,&targets,owner_sid){Ok(session)=>session,Err(error)=>{let _=pipe.finish(&FromCollector{protocol:PROTOCOL,snapshot:engine_failure(error),stopped:true},alive);return Ok(())}};
    let mut sampled=Instant::now()-Duration::from_secs(1);let mut previous:Option<(Instant,u64,u64)>=None;
    let result=(||->Result<(),String>{loop{
        if !alive(){return Ok(())}
        if !broker_alive(){return Err("流量辅助服务已退出，本次采样已停止。".into())}
        match pipe.read::<ToCollector>()?{Some(ToCollector::Targets{protocol,targets})if protocol==PROTOCOL=>session.set_targets(&targets)?,Some(ToCollector::Stop{protocol})if protocol==PROTOCOL=>return Ok(()),Some(_)=>return Err("采样协议无效。".into()),None=>{}}
        if sampled.elapsed()>=Duration::from_secs(1){let at=Instant::now();let sample=session.sample();let mut snapshot=if let Some(error)=sample.error{engine_failure(error)}else if sample.had_targets&&!sample.seen{TrafficSnapshot::state(TrafficState::Starting,Some("等待首个已验证目标进程的网络事件。".into()))}else{let mut value=TrafficSnapshot::state(TrafficState::Collecting,None);value.sent_bytes=Some(sample.sent);value.received_bytes=Some(sample.received);if let Some((before,sent,received))=previous{let seconds=at.duration_since(before).as_secs_f64();if seconds>0.0{value.send_bps=Some(sample.sent.saturating_sub(sent)as f64/seconds);value.receive_bps=Some(sample.received.saturating_sub(received)as f64/seconds);}}previous=Some((at,sample.sent,sample.received));value};snapshot.target_count=sample.target_count;snapshot.lost_events=sample.lost_events;snapshot.lost_buffers=sample.lost_buffers;pipe.write(&FromCollector{protocol:PROTOCOL,snapshot,stopped:false},alive)?;sampled=at;}
        thread::sleep(Duration::from_millis(20));
    }})();
    // Parent death and IPC errors take this same cleanup path; no global logger name is ever stopped.
    if let Err(error)=session.stop(){let _=pipe.write(&FromCollector{protocol:PROTOCOL,snapshot:engine_failure(error),stopped:false},alive);return Err("本次采样停止未确认。".into())}
    let snapshot=match &result{Ok(())=>TrafficSnapshot::disabled(),Err(message)=>TrafficSnapshot::state(TrafficState::Unavailable,Some(message.clone()))};
    let _=pipe.finish(&FromCollector{protocol:PROTOCOL,snapshot,stopped:true},alive);result
}
