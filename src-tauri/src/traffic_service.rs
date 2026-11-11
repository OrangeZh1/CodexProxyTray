//! Demand-start LocalSystem broker for the optional ETW collector.
//! Only explicit install/uninstall actions request UAC. The broker accepts no paths or commands.
use std::{ffi::{OsStr,OsString},fs::{self,File,OpenOptions},io::{Read,Write},mem::size_of,os::windows::{ffi::{OsStrExt,OsStringExt},fs::MetadataExt,io::AsRawHandle,process::CommandExt},path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{OnceLock,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use serde::{Deserialize,Serialize};
use windows::{core::{PCWSTR,PWSTR},Win32::{Foundation::*,Security::{*,Authorization::*,Cryptography::*},Storage::FileSystem::*,System::{Diagnostics::ToolHelp::*,Pipes::*,Services::*,Threading::*},UI::Shell::{SHGetFolderPathW,ShellExecuteExW,SHELLEXECUTEINFOW,SEE_MASK_NOCLOSEPROCESS}}};
use crate::{traffic::{OwnedHandle,identity_on,open_identity},traffic_core::ProcessIdentity};

const NAME:&str="CodexProxyTrayTraffic";
const DISPLAY:&str="CodexProxyTray Traffic Collector";
const PIPE:&str=r"\\.\pipe\CodexProxyTray.Traffic.Service.v1";
const IMAGE:&str="CodexProxyTrayTrafficService.exe";
const LOADER:&str="WebView2Loader.dll";
const IMAGE_PENDING:&str="CodexProxyTrayTrafficService.exe.pending";
const LOADER_PENDING:&str="WebView2Loader.dll.pending";
const LOADER_HASH:&str="86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5";
const RECEIPT:&str="receipt.json";
const PENDING:&str="receipt.pending.json";
const PRODUCT:&str="CodexProxyTray.TrafficService";
const FILE_SDDL:&str="O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
const PROTOCOL:u32=1;
const MAX_FRAME:usize=4096;
static SELF_HASH:OnceLock<Result<String,String>>=OnceLock::new();
static STOP:AtomicBool=AtomicBool::new(false);

#[derive(Clone,Serialize)]
pub struct ServiceSnapshot {pub state:String,pub version:Option<String>,pub error:Option<String>}
impl ServiceSnapshot {fn new(state:&str,version:Option<String>,error:Option<String>)->Self{Self{state:state.into(),version,error}}}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {schema:u32,product:String,owner_sid:String,version:String,sha256:String,loader_sha256:String,image:String,service_command:String,phase:String,readiness:String,previous:Option<String>,created_parent:bool}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {protocol:u32,token:String,parent:ProcessIdentity}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {protocol:u32,handle:u64,child:Option<ProcessIdentity>,error:Option<String>}
struct Sc(SC_HANDLE);
impl Drop for Sc {fn drop(&mut self){unsafe{let _=CloseServiceHandle(self.0);}}}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {fn drop(&mut self){unsafe{let _=LocalFree(Some(HLOCAL(self.0.0)));}}}
struct InstallLock(OwnedHandle);
impl Drop for InstallLock {fn drop(&mut self){unsafe{let _=ReleaseMutex(self.0.0);}}}
fn wide(value:impl AsRef<OsStr>)->Vec<u16>{value.as_ref().encode_wide().chain(Some(0)).collect()}
fn win(error:windows::core::Error)->String{format!("Windows 操作失败（{}）。",error.code().0 as u32&0xffff)}
fn code(error:&windows::core::Error)->u32{error.code().0 as u32&0xffff}
fn own_image()->Result<PathBuf,String>{std::env::current_exe().and_then(fs::canonicalize).map_err(|_|"无法验证程序路径。".into())}
fn paths()->Result<(PathBuf,PathBuf),String>{unsafe{
    // Resolve the OS folder, never an inherited ProgramFiles environment variable.
    let mut buffer=[0u16;260];SHGetFolderPathW(None,0x26,None,0,&mut buffer).map_err(win)?;
    let end=buffer.iter().position(|&value|value==0).ok_or("系统程序目录无效。")?;
    let base=PathBuf::from(OsString::from_wide(&buffer[..end]));if !base.is_absolute(){return Err("系统程序目录无效。".into())}
    let directory=base.join("CodexProxyTray").join("TrafficService");Ok((directory.clone(),directory.join(IMAGE)))
}}
fn command(image:&Path)->Result<String,String>{let text=image.to_str().ok_or("辅助服务路径无法编码。")?;if text.contains(['"','\0']){return Err("辅助服务路径无效。".into())}Ok(format!("\"{text}\" --traffic-service"))}
fn descriptor(text:&str)->Result<Descriptor,String>{unsafe{let text=wide(text);let mut result=PSECURITY_DESCRIPTOR::default();ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(text.as_ptr()),1,&mut result,None).map_err(win)?;Ok(Descriptor(result))}}
fn sid_string(sid:PSID)->Result<String,String>{unsafe{let mut text=PWSTR::null();ConvertSidToStringSidW(sid,&mut text).map_err(win)?;let result=text.to_string().map_err(|_|"用户身份无效。".into());let _=LocalFree(Some(HLOCAL(text.0.cast())));result}}
fn process_sid(handle:HANDLE)->Result<String,String>{unsafe{
    let mut raw=HANDLE::default();OpenProcessToken(handle,TOKEN_QUERY,&mut raw).map_err(win)?;let token=OwnedHandle(raw);let mut needed=0;
    let _=GetTokenInformation(token.0,TokenUser,None,0,&mut needed);if needed<size_of::<TOKEN_USER>()as u32||needed>65536{return Err("用户身份长度无效。".into())}
    let mut buffer=vec![0u64;(needed as usize).div_ceil(8)];GetTokenInformation(token.0,TokenUser,Some(buffer.as_mut_ptr().cast()),needed,&mut needed).map_err(win)?;
    sid_string((*(buffer.as_ptr()as *const TOKEN_USER)).User.Sid)
}}
fn current_sid()->Result<String,String>{process_sid(unsafe{GetCurrentProcess()})}
fn process_image(handle:HANDLE)->Result<PathBuf,String>{unsafe{let mut path=vec![0u16;32768];let mut length=path.len()as u32;QueryFullProcessImageNameW(handle,PROCESS_NAME_WIN32,PWSTR(path.as_mut_ptr()),&mut length).map_err(win)?;path.truncate(length as usize);PathBuf::from(OsString::from_wide(&path)).canonicalize().map_err(|_|"无法验证进程映像。".into())}}
fn no_reparse(path:&Path)->Result<(),String>{let metadata=fs::symlink_metadata(path).map_err(|_|"辅助服务路径不可读取。")?;if metadata.file_attributes()&FILE_ATTRIBUTE_REPARSE_POINT.0!=0{return Err("辅助服务路径包含重解析点，已保留原文件。".into())}Ok(())}
fn check_chain(directory:&Path)->Result<(),String>{
    let parent=directory.parent().ok_or("辅助服务目录无效。")?;let base=parent.parent().ok_or("辅助服务目录无效。")?;
    no_reparse(base)?;if parent.exists(){no_reparse(parent)?;check_file_acl(parent)?;}if directory.exists(){no_reparse(directory)?;check_file_acl(directory)?;}Ok(())
}
fn protect(path:&Path)->Result<(),String>{unsafe{let security=descriptor(FILE_SDDL)?;let path=wide(path);if !SetFileSecurityW(PCWSTR(path.as_ptr()),OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,security.0).as_bool(){return Err("无法保护辅助服务文件权限。".into())}Ok(())}}
fn check_file_acl(path:&Path)->Result<(),String>{unsafe{
    let name=wide(path);let info=OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION;let mut needed=0;
    let _=GetFileSecurityW(PCWSTR(name.as_ptr()),info.0,None,0,&mut needed);if needed==0||needed>65536{return Err("无法验证辅助服务文件权限。".into())}
    let mut buffer=vec![0u64;(needed as usize).div_ceil(8)];let security=PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast());if !GetFileSecurityW(PCWSTR(name.as_ptr()),info.0,Some(security),needed,&mut needed).as_bool(){return Err("无法读取辅助服务文件权限。".into())}
    check_security(security,true)
}}
fn check_security(security:PSECURITY_DESCRIPTOR,file:bool)->Result<(),String>{unsafe{
    let mut owner=PSID::default();let mut owner_defaulted=windows::core::BOOL::default();GetSecurityDescriptorOwner(security,&mut owner,&mut owner_defaulted).map_err(win)?;if !matches!(sid_string(owner)?.as_str(),"S-1-5-18"|"S-1-5-32-544"){return Err("辅助服务对象所有者不受信，已拒绝操作。".into())}
    let(mut present,mut defaulted)=(windows::core::BOOL::default(),windows::core::BOOL::default());let mut acl=std::ptr::null_mut();GetSecurityDescriptorDacl(security,&mut present,&mut acl,&mut defaulted).map_err(win)?;
    if !present.as_bool()||acl.is_null(){return Err("辅助服务访问控制无效。".into())}
    let mut control=0u16;let mut revision=0;GetSecurityDescriptorControl(security,&mut control,&mut revision).map_err(win)?;if file&&control&SE_DACL_PROTECTED.0==0{return Err("辅助服务文件未保护访问控制。".into())}
    let mut details=ACL_SIZE_INFORMATION::default();GetAclInformation(acl,(&mut details as *mut ACL_SIZE_INFORMATION).cast(),size_of::<ACL_SIZE_INFORMATION>()as u32,AclSizeInformation).map_err(win)?;
    if details.AceCount!=if file{3}else{2}{return Err("辅助服务访问控制含有未知授权。".into())}
    let mut seen=std::collections::HashSet::new();for index in 0..details.AceCount{let mut raw=std::ptr::null_mut();GetAce(acl,index,&mut raw).map_err(win)?;let ace=&*(raw as *const ACCESS_ALLOWED_ACE);if ace.Header.AceType!=0{return Err("辅助服务访问控制含有未知规则。".into())}let sid=sid_string(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
        let expected=match sid.as_str(){"S-1-5-18"|"S-1-5-32-544"=>if file{0x1f01ff}else{0x1f0001},"S-1-5-32-545"if file=>0x1200a9,_=>return Err("辅助服务访问控制含有未知用户。".into())};if ace.Mask!=expected||!seen.insert(sid){return Err("辅助服务访问控制权限不匹配。".into())}}
    Ok(())
}}
fn sha256(path:&Path)->Result<String,String>{unsafe{
    let metadata=fs::metadata(path).map_err(|_|"辅助服务程序不可读取。")?;if !metadata.is_file()||metadata.len()>128*1024*1024{return Err("辅助服务程序大小无效。".into())}
    let mut algorithm=BCRYPT_ALG_HANDLE::default();if BCryptOpenAlgorithmProvider(&mut algorithm,BCRYPT_SHA256_ALGORITHM,PCWSTR::null(),BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).0<0{return Err("无法创建程序校验。".into())}
    let mut hash=BCRYPT_HASH_HANDLE::default();let result=(||{if BCryptCreateHash(algorithm,&mut hash,None,None,0).0<0{return Err("无法初始化程序校验。".into())}let mut file=File::open(path).map_err(|_|"程序校验读取失败。")?;let mut buffer=[0u8;65536];let mut total=0u64;loop{let count=file.read(&mut buffer).map_err(|_|"程序校验读取失败。")?;if count==0{break}total+=count as u64;if total>128*1024*1024{return Err("程序校验读取超过上限。".into())}if BCryptHashData(hash,&buffer[..count],0).0<0{return Err("程序校验计算失败。".into())}}if total!=metadata.len(){return Err("程序在校验期间发生变化。".into())}let mut digest=[0u8;32];if BCryptFinishHash(hash,&mut digest,0).0<0{return Err("程序校验计算失败。".into())}Ok(digest.iter().map(|value|format!("{value:02x}")).collect())})();
    if !hash.0.is_null(){let _=BCryptDestroyHash(hash);}let _=BCryptCloseAlgorithmProvider(algorithm,0);result
}}
fn self_hash()->Result<String,String>{SELF_HASH.get_or_init(||own_image().and_then(|image|sha256(&image))).clone()}
fn inventory(directory:&Path)->Result<(),String>{if !directory.exists(){return Ok(())}for entry in fs::read_dir(directory).map_err(|_|"辅助服务目录不可读取。")?{let entry=entry.map_err(|_|"辅助服务目录不可读取。")?;if !matches!(entry.file_name().to_str(),Some(IMAGE)|Some(LOADER)|Some(RECEIPT)|Some(PENDING)|Some(IMAGE_PENDING)|Some(LOADER_PENDING)){return Err("辅助服务目录有未知文件，已保留并拒绝修改。".into())}no_reparse(&entry.path())?;if !entry.file_type().map_err(|_|"辅助服务文件无效。")?.is_file(){return Err("辅助服务目录有未知子目录。".into())}check_file_acl(&entry.path())?;}Ok(())}
fn load_receipt(directory:&Path,image:&Path,verify_hash:bool)->Result<Receipt,String>{
    check_chain(directory)?;inventory(directory)?;let record=read_record(&directory.join(RECEIPT))?;validate_record(&record,directory,image,verify_hash)?;Ok(record)
}
fn read_record(path:&Path)->Result<Receipt,String>{let metadata=fs::metadata(path).map_err(|_|"辅助服务安装记录缺失。")?;if metadata.len()>16384{return Err("辅助服务安装记录无效。".into())}serde_json::from_slice(&fs::read(path).map_err(|_|"辅助服务安装记录不可读取。")?).map_err(|_|"辅助服务安装记录无效。".into())}
fn validate_record(record:&Receipt,directory:&Path,image:&Path,verify_hash:bool)->Result<(),String>{
    if record.schema!=1||record.product!=PRODUCT||record.image!=image.to_string_lossy()||record.service_command!=command(image)?||!matches!(record.phase.as_str(),"prepared"|"installed"|"removing")||record.sha256.len()!=64||!record.sha256.bytes().all(|v|v.is_ascii_hexdigit())||record.loader_sha256!=LOADER_HASH||record.previous.is_some()||!record.owner_sid.starts_with("S-1-"){return Err("辅助服务安装记录不匹配。".into())}
    if record.phase=="installed"&&[IMAGE_PENDING,LOADER_PENDING].iter().any(|name|directory.join(name).exists()){return Err("辅助服务存在未完成的文件变更，已保留。".into())}
    if verify_hash{for(path,hash)in[(image.to_path_buf(),&record.sha256),(directory.join(LOADER),&record.loader_sha256)]{if matches!(record.phase.as_str(),"prepared"|"removing")&&!path.exists(){continue}if sha256(&path)?!=*hash{return Err("辅助服务程序校验不匹配，已拒绝操作。".into())}}}Ok(())
}
fn recover_pending(directory:&Path,image:&Path,owner:&str,service:Option<&Sc>)->Result<(),String>{
    let pending=directory.join(PENDING);if !pending.exists(){return Ok(())}check_chain(directory)?;inventory(directory)?;let next=read_record(&pending)?;validate_record(&next,directory,image,true)?;if next.owner_sid!=owner{return Err("待恢复记录属于其他用户，已保留。".into())}
    if directory.join(RECEIPT).exists(){let previous=load_receipt(directory,image,true)?;if previous.owner_sid!=next.owner_sid||previous.sha256!=next.sha256||previous.version!=next.version||previous.created_parent!=next.created_parent||!matches!((previous.phase.as_str(),next.phase.as_str()),("prepared","prepared")|("prepared","installed")|("installed","removing")|("removing","removing")){return Err("待恢复安装记录与现有记录不匹配，已保留。".into())}}
    else if service.is_some()||next.phase!="prepared"||fs::read_dir(directory).map_err(|_|"辅助服务目录不可读取。")?.any(|entry|entry.map_or(true,|entry|entry.file_name()!=PENDING)){return Err("待恢复记录缺少可确认的前序状态，已保留。".into())}
    if let Some(service)=service{validate_config_inner(service,image,owner,next.phase!="prepared")?;}else if next.phase=="installed"{return Err("待恢复记录尚未对应已注册服务，已保留。".into())}
    let from=wide(&pending);let to=wide(directory.join(RECEIPT));unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH).map_err(win)}
}
fn write_receipt(directory:&Path,record:&Receipt)->Result<(),String>{
    let pending=directory.join(PENDING);if pending.exists(){return Err("辅助服务有待恢复的安装记录，请保留现场。".into())}
    let bytes=serde_json::to_vec_pretty(record).map_err(|_|"安装记录编码失败。")?;let mut file=OpenOptions::new().write(true).create_new(true).open(&pending).map_err(|_|"安装记录写入失败。")?;protect(&pending)?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"安装记录未能持久保存。")?;drop(file);
    let from=wide(&pending);let to=wide(directory.join(RECEIPT));unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH).map_err(win)}
}
fn manager(access:u32)->Result<Sc,String>{unsafe{OpenSCManagerW(PCWSTR::null(),PCWSTR::null(),access).map(Sc).map_err(win)}}
fn service(manager:&Sc,access:u32)->Result<Option<Sc>,String>{let name=wide(NAME);unsafe{match OpenServiceW(manager.0,PCWSTR(name.as_ptr()),access){Ok(handle)=>Ok(Some(Sc(handle))),Err(error)if code(&error)==1060=>Ok(None),Err(error)=>Err(win(error))}}}
fn status(service:&Sc)->Result<SERVICE_STATUS_PROCESS,String>{unsafe{let mut value=SERVICE_STATUS_PROCESS::default();let bytes=std::slice::from_raw_parts_mut((&mut value as *mut SERVICE_STATUS_PROCESS).cast(),size_of::<SERVICE_STATUS_PROCESS>());let mut needed=0;QueryServiceStatusEx(service.0,SC_STATUS_PROCESS_INFO,Some(bytes),&mut needed).map_err(win)?;Ok(value)}}
fn validate_config(service:&Sc,image:&Path,owner:&str)->Result<(),String>{validate_config_inner(service,image,owner,true)}
fn validate_config_inner(service:&Sc,image:&Path,owner:&str,check_acl:bool)->Result<(),String>{unsafe{
    let mut needed=0;let _=QueryServiceConfigW(service.0,None,0,&mut needed);if needed<size_of::<QUERY_SERVICE_CONFIGW>()as u32||needed>65536{return Err("辅助服务配置长度无效。".into())}let mut bytes=vec![0u64;(needed as usize).div_ceil(8)];let ptr=bytes.as_mut_ptr()as *mut QUERY_SERVICE_CONFIGW;QueryServiceConfigW(service.0,Some(ptr),needed,&mut needed).map_err(win)?;let config=&*ptr;
    if config.dwServiceType!=SERVICE_WIN32_OWN_PROCESS||config.dwStartType!=SERVICE_DEMAND_START||config.dwErrorControl!=SERVICE_ERROR_NORMAL||config.lpBinaryPathName.to_string().map_err(|_|"服务映像路径无效。")?!=command(image)?||!config.lpServiceStartName.to_string().unwrap_or_default().eq_ignore_ascii_case("LocalSystem")||config.lpDisplayName.to_string().unwrap_or_default()!=DISPLAY||(!config.lpDependencies.is_null()&&*config.lpDependencies.0!=0)||(!config.lpLoadOrderGroup.is_null()&&*config.lpLoadOrderGroup.0!=0){return Err("同名辅助服务配置不属于本程序，已拒绝修改。".into())}
    // A durable prepared receipt precedes CreateService. Only that recovery stage may
    // repair the default SCM descriptor after a crash between CreateService and SetSecurity.
    if !check_acl{return Ok(())}
    let info=OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION;let mut needed=0;let _=QueryServiceObjectSecurity(service.0,info.0,None,0,&mut needed);if needed==0||needed>65536{return Err("辅助服务权限长度无效。".into())}let mut buffer=vec![0u64;(needed as usize).div_ceil(8)];let security=PSECURITY_DESCRIPTOR(buffer.as_mut_ptr().cast());QueryServiceObjectSecurity(service.0,info.0,Some(security),needed,&mut needed).map_err(win)?;
    let mut actual_owner=PSID::default();let mut owner_defaulted=windows::core::BOOL::default();GetSecurityDescriptorOwner(security,&mut actual_owner,&mut owner_defaulted).map_err(win)?;if !matches!(sid_string(actual_owner)?.as_str(),"S-1-5-18"|"S-1-5-32-544"){return Err("辅助服务对象所有者不受信。".into())}
    let(mut present,mut defaulted)=(windows::core::BOOL::default(),windows::core::BOOL::default());let mut acl=std::ptr::null_mut();GetSecurityDescriptorDacl(security,&mut present,&mut acl,&mut defaulted).map_err(win)?;if !present.as_bool()||acl.is_null()||(*acl).AceCount!=3{return Err("辅助服务对象访问控制不匹配。".into())}let mut seen=std::collections::HashSet::new();for index in 0..3{let mut raw=std::ptr::null_mut();GetAce(acl,index,&mut raw).map_err(win)?;let ace=&*(raw as *const ACCESS_ALLOWED_ACE);let sid=sid_string(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;let mask=if matches!(sid.as_str(),"S-1-5-18"|"S-1-5-32-544"){SERVICE_ALL_ACCESS}else if sid==owner{SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_START|READ_CONTROL.0}else{return Err("辅助服务对象存在未知授权。".into())};if ace.Header.AceType!=0||ace.Mask!=mask||!seen.insert(sid){return Err("辅助服务对象权限不匹配。".into())}}
    Ok(())
}}
pub fn snapshot()->ServiceSnapshot{match snapshot_checked(){Ok(value)=>value,Err(error)=>ServiceSnapshot::new("error",None,Some(error))}}
fn snapshot_checked()->Result<ServiceSnapshot,String>{
    let(directory,image)=paths()?;let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|READ_CONTROL.0)?;
    let Some(service)=service else{if directory.exists()&&directory.join(RECEIPT).exists(){let record=load_receipt(&directory,&image,false)?;return Ok(ServiceSnapshot::new("error",Some(record.version),Some("安装记录存在，但辅助服务未完成注册。".into())))}return Ok(ServiceSnapshot::new("not_installed",None,None))};
    let record=load_receipt(&directory,&image,false)?;validate_config(&service,&image,&record.owner_sid)?;if record.owner_sid!=current_sid()?{return Err("辅助服务属于其他 Windows 用户。".into())}if record.phase!="installed"{return Err("辅助服务安装尚未完成。".into())}
    if record.sha256!=self_hash()?{return Ok(ServiceSnapshot::new("outdated",Some(record.version),Some("辅助服务与当前版本不同，请先卸载再安装。".into())))}
    let state=if status(&service)?.dwCurrentState==SERVICE_RUNNING{"running"}else{"stopped"};Ok(ServiceSnapshot::new(state,Some(record.version),None))
}
pub fn is_installed()->bool{matches!(snapshot().state.as_str(),"running"|"stopped"|"outdated")}

/// These functions are invoked only by explicit UI actions; ordinary collection never calls runas.
pub fn install()->Result<ServiceSnapshot,String>{admin_action("--install-traffic-service")}
pub fn uninstall()->Result<ServiceSnapshot,String>{admin_action("--uninstall-traffic-service")}
fn admin_action(action:&str)->Result<ServiceSnapshot,String>{unsafe{
    let exe=own_image()?;let parent=identity_on(GetCurrentProcess()).map_err(win)?;let file=wide(exe);let verb=wide("runas");let args=wide(format!("{action} --parent-pid {} --parent-created {}",parent.pid,parent.created_filetime));let mut request=SHELLEXECUTEINFOW::default();request.cbSize=size_of::<SHELLEXECUTEINFOW>()as u32;request.fMask=SEE_MASK_NOCLOSEPROCESS;request.lpVerb=PCWSTR(verb.as_ptr());request.lpFile=PCWSTR(file.as_ptr());request.lpParameters=PCWSTR(args.as_ptr());request.nShow=0;
    ShellExecuteExW(&mut request).map_err(|error|if code(&error)==1223{"已取消 Windows 授权。".into()}else{win(error)})?;if request.hProcess.is_invalid(){return Err("无法确认辅助服务操作进程。".into())}let process=OwnedHandle(request.hProcess);
    // Runs on Tauri's blocking pool. Keep this action busy until our actual helper exits;
    // a timeout must never release maintenance while an installer is still modifying SCM.
    loop{match WaitForSingleObject(process.0,1000){WAIT_OBJECT_0=>break,WAIT_TIMEOUT=>{},_=>thread::sleep(Duration::from_secs(1))}}
    let mut exit=0;GetExitCodeProcess(process.0,&mut exit).map_err(win)?;if exit==3{return Err("另一个辅助服务操作正在进行。".into())}if exit!=0{return Err(snapshot().error.unwrap_or_else(||"辅助服务操作未完成，原有安装已保留。".into()))}Ok(snapshot())
}}
fn installer_parent(args:&[OsString])->Result<OwnedHandle,String>{
    if args.len()!=5||args[1]!="--parent-pid"||args[3]!="--parent-created"{return Err("辅助服务操作参数无效。".into())}let pid=args[2].to_str().and_then(|v|v.parse().ok()).filter(|&v|v!=0).ok_or("父进程无效。")?;let created_filetime=args[4].to_str().and_then(|v|v.parse().ok()).filter(|&v|v!=0).ok_or("父进程无效。")?;
    let parent=open_identity(ProcessIdentity{pid,created_filetime}).map_err(win)?.ok_or("操作父进程已退出。")?;if process_image(parent.0)?!=own_image()?||sha256(&process_image(parent.0)?)?!=self_hash()?||process_sid(parent.0)?=="S-1-5-18"{return Err("辅助服务操作父程序不匹配。".into())}Ok(parent)
}
fn installer_owner(args:&[OsString])->Result<String,String>{unsafe{
    let mut raw=HANDLE::default();OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut raw).map_err(win)?;let token=OwnedHandle(raw);let mut elevation=TOKEN_ELEVATION::default();let mut returned=0;GetTokenInformation(token.0,TokenElevation,Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),size_of::<TOKEN_ELEVATION>()as u32,&mut returned).map_err(win)?;if elevation.TokenIsElevated==0{return Err("辅助服务安装与卸载需要管理员权限。".into())}
    // Isolated Windows CI can invoke the already-elevated CLI without manufacturing a GUI.
    // This route never elevates itself and cannot assign service access to a different account.
    if args.len()==3&&args[1]=="--owner-sid"{let sid=args[2].to_str().ok_or("辅助服务所有者无效。")?;if sid!=current_sid()?||sid=="S-1-5-18"{return Err("辅助服务所有者不匹配。".into())}return Ok(sid.into())}
    let parent=installer_parent(args)?;process_sid(parent.0)
}}
fn install_lock()->Result<InstallLock,String>{unsafe{
    let security=descriptor("O:BAG:BAD:P(A;;0x1f0001;;;SY)(A;;0x1f0001;;;BA)")?;let attributes=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>()as u32,lpSecurityDescriptor:security.0.0,bInheritHandle:false.into()};let name=wide(r"Global\CodexProxyTray.Traffic.Service.Install.v1");let handle=OwnedHandle(CreateMutexW(Some(&attributes),false,PCWSTR(name.as_ptr())).map_err(win)?);
    let mut raw=PSECURITY_DESCRIPTOR::default();let result=GetSecurityInfo(handle.0,SE_KERNEL_OBJECT,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,None,None,None,None,Some(&mut raw));if result.0!=0{return Err("无法验证辅助服务操作锁。".into())}let owned=Descriptor(raw);check_security(owned.0,false)?;
    match WaitForSingleObject(handle.0,0){WAIT_OBJECT_0|WAIT_ABANDONED=>Ok(InstallLock(handle)),_=>Err("另一个辅助服务操作正在进行。".into())}
}}
fn secure_directory(path:&Path)->Result<(),String>{
    if path.exists(){no_reparse(path)?;return check_file_acl(path)}let security=descriptor(FILE_SDDL)?;let attributes=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>()as u32,lpSecurityDescriptor:security.0.0,bInheritHandle:false.into()};let name=wide(path);unsafe{match CreateDirectoryW(PCWSTR(name.as_ptr()),Some(&attributes)){Ok(())=>{},Err(error)if code(&error)==183=>{},Err(error)=>return Err(win(error))}}no_reparse(path)?;check_file_acl(path)
}
fn stop_service(service:&Sc)->Result<(),String>{
    let deadline=Instant::now()+Duration::from_secs(10);loop{let state=status(service)?.dwCurrentState;if state==SERVICE_STOPPED{return Ok(())}if state!=SERVICE_STOP_PENDING{unsafe{let mut value=SERVICE_STATUS::default();match ControlService(service.0,SERVICE_CONTROL_STOP,&mut value){Ok(())=>{},Err(error)if matches!(code(&error),1061|1062)=>{},Err(error)=>return Err(win(error))}}}if Instant::now()>=deadline{return Err("请先停止流量统计，辅助服务仍在使用或收尾；安装文件已保留。".into())}thread::sleep(Duration::from_millis(50));}
}
fn install_file(source:&Path,target:&Path,pending:&Path,expected:&str)->Result<(),String>{
    if target.exists(){return if sha256(target)?==expected{Ok(())}else{Err("受保护辅助程序校验不匹配。".into())}}
    // A prepared receipt declares these fixed pending names before the first byte is copied.
    if pending.exists(){no_reparse(pending)?;check_file_acl(pending)?;fs::remove_file(pending).map_err(|_|"未完成的辅助程序副本仍被使用。")?;}
    let mut source=File::open(source).map_err(|_|"无法读取辅助服务源文件。")?;let mut file=OpenOptions::new().write(true).create_new(true).open(pending).map_err(|_|"无法创建辅助服务副本。")?;protect(pending)?;std::io::copy(&mut source,&mut file).and_then(|_|file.sync_all()).map_err(|_|"辅助服务副本未能完整保存。")?;drop(file);check_file_acl(pending)?;if sha256(pending)?!=expected{return Err("辅助服务副本校验失败。".into())}
    let from=wide(pending);let to=wide(target);unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_WRITE_THROUGH).map_err(win)}
}
fn install_inner(owner:String)->Result<(),String>{
    let source_image=own_image()?;let source_loader=source_image.parent().ok_or("当前程序目录无效。")?.join(LOADER);if sha256(&source_loader)?!=LOADER_HASH{return Err("WebView2Loader.dll 校验失败，已拒绝安装。".into())}
    let(directory,image)=paths()?;check_chain(&directory)?;let manager=manager(SC_MANAGER_CONNECT|SC_MANAGER_CREATE_SERVICE)?;let existing=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_STOP|WRITE_DAC.0|WRITE_OWNER.0|READ_CONTROL.0)?;
    recover_pending(&directory,&image,&owner,existing.as_ref())?;
    let prior=if directory.join(RECEIPT).exists(){Some(load_receipt(&directory,&image,true)?)}else{None};
    if let Some(existing)=&existing{let record=prior.as_ref().ok_or("同名服务缺少本程序安装记录，已拒绝覆盖。")?;validate_config_inner(existing,&image,&record.owner_sid,record.phase!="prepared")?;if record.phase=="installed"&&record.owner_sid==owner&&record.sha256==self_hash()?{return Ok(())}}
    if let Some(record)=&prior{if record.owner_sid!=owner{return Err("辅助服务属于其他用户，已拒绝覆盖。".into())}if record.phase=="removing"{return Err("辅助服务卸载尚未完成，请继续卸载。".into())}if record.sha256!=self_hash()?{return Err("辅助服务版本不同，请先卸载再安装。".into())}}
    else if directory.exists(){inventory(&directory)?;if fs::read_dir(&directory).map_err(|_|"辅助服务目录不可读取。")?.next().is_some(){return Err("辅助服务目录缺少归属记录，已保留。".into())}}
    let created_parent=prior.as_ref().map_or(!directory.parent().ok_or("辅助服务目录无效。")?.exists(),|record|record.created_parent);
    secure_directory(directory.parent().ok_or("辅助服务目录无效。")?)?;secure_directory(&directory)?;
    let mut record=Receipt{schema:1,product:PRODUCT.into(),owner_sid:owner.clone(),version:env!("CARGO_PKG_VERSION").into(),sha256:self_hash()?,loader_sha256:LOADER_HASH.into(),image:image.to_string_lossy().into(),service_command:command(&image)?,phase:"prepared".into(),readiness:"copy_pending".into(),previous:None,created_parent};if prior.is_none(){write_receipt(&directory,&record)?;}
    install_file(&source_image,&image,&directory.join(IMAGE_PENDING),&record.sha256)?;install_file(&source_loader,&directory.join(LOADER),&directory.join(LOADER_PENDING),LOADER_HASH)?;record.readiness="files_verified".into();write_receipt(&directory,&record)?;
    let installed=if let Some(existing)=existing{existing}else{let name=wide(NAME);let display=wide(DISPLAY);let binary=wide(&record.service_command);unsafe{Sc(CreateServiceW(manager.0,PCWSTR(name.as_ptr()),PCWSTR(display.as_ptr()),SERVICE_ALL_ACCESS,SERVICE_WIN32_OWN_PROCESS,SERVICE_DEMAND_START,SERVICE_ERROR_NORMAL,PCWSTR(binary.as_ptr()),PCWSTR::null(),None,PCWSTR::null(),PCWSTR::null(),PCWSTR::null()).map_err(win)?)} };
    let acl=descriptor(&format!("O:BAG:BAD:P(A;;0xf01ff;;;SY)(A;;0xf01ff;;;BA)(A;;0x20015;;;{owner})"))?;unsafe{SetServiceObjectSecurity(installed.0,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,acl.0).map_err(win)?;}validate_config(&installed,&image,&owner)?;
    record.phase="installed".into();record.readiness="scm_configuration_verified".into();write_receipt(&directory,&record)?;Ok(())
}
fn uninstall_inner(owner:String)->Result<(),String>{
    let(directory,image)=paths()?;let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_STOP|DELETE.0|READ_CONTROL.0)?;
    if service.is_none()&&!directory.exists(){return Ok(())}recover_pending(&directory,&image,&owner,service.as_ref())?;let mut record=load_receipt(&directory,&image,true)?;if record.owner_sid!=owner{return Err("辅助服务属于其他用户，已拒绝删除。".into())}
    if directory.join(PENDING).exists(){return Err("辅助服务有待恢复记录，已保留安装。".into())}if let Some(service)=service{validate_config_inner(&service,&image,&owner,record.phase!="prepared")?;stop_service(&service)?;record.phase="removing".into();record.readiness="service_stopped_for_removal".into();write_receipt(&directory,&record)?;unsafe{DeleteService(service.0).map_err(win)?;}drop(service);
        let deadline=Instant::now()+Duration::from_secs(5);loop{match self::service(&manager,SERVICE_QUERY_STATUS){Ok(None)=>break,Err(_)=>return Err("辅助服务删除尚未确认，文件已保留。".into()),Ok(Some(_))=>{if Instant::now()>=deadline{return Err("辅助服务仍在删除，文件已保留。".into())}thread::sleep(Duration::from_millis(50));}}}}
    // Exact owned allowlist only. Never recursively remove Program Files or the parent product directory.
    if record.phase!="removing"{record.phase="removing".into();record.readiness="service_absent_for_removal".into();write_receipt(&directory,&record)?;}
    inventory(&directory)?;for path in [&image,&directory.join(LOADER),&directory.join(IMAGE_PENDING),&directory.join(LOADER_PENDING)]{if path.exists(){fs::remove_file(path).map_err(|_|"辅助服务文件仍被使用，已保留卸载恢复记录。")?;}}fs::remove_file(directory.join(RECEIPT)).map_err(|_|"辅助服务安装记录清理未完成。")?;
    if fs::remove_dir(&directory).is_err(){
        // Keep an attributable retry path when an otherwise empty directory is still locked.
        // Never overwrite a new file or restore into a replaced/unprotected directory.
        let restored=(||->Result<(),String>{check_chain(&directory)?;no_reparse(&directory)?;check_file_acl(&directory)?;inventory(&directory)?;if fs::read_dir(&directory).map_err(|_|"辅助服务目录不可读取。")?.next().is_some(){return Err("辅助服务目录出现新文件，已保留。".into())}let bytes=serde_json::to_vec_pretty(&record).map_err(|_|"卸载恢复记录编码失败。")?;let path=directory.join(RECEIPT);let mut file=OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_|"卸载恢复记录无法创建。")?;protect(&path)?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"卸载恢复记录未能持久保存。")?;check_file_acl(&path)?;Ok(())})();
        return Err(if restored.is_ok(){"辅助服务目录清理未完成，已恢复卸载记录，可稍后重试。".into()}else{"辅助服务目录清理未完成；恢复记录未能安全保存，原目录已保留。".into()})
    }
    if record.created_parent{let parent=directory.parent().ok_or("辅助服务目录无效。")?;no_reparse(parent)?;check_file_acl(parent)?;if fs::read_dir(parent).map_err(|_|"辅助服务父目录不可读取。")?.next().is_none(){fs::remove_dir(parent).map_err(|_|"辅助服务空目录清理未完成。")?;}}Ok(())
}

/// Main calls this before Tauri, COM, package activation and collector entry handling.
pub fn service_entry(args:&[OsString])->Option<i32>{
    let action=args.first()?.to_str()?;match action{
        "--traffic-service"=>Some(if args.len()==1&&unsafe{dispatch()}.is_ok(){0}else{2}),
        "--install-traffic-service"|"--uninstall-traffic-service"=>Some((||{let owner=installer_owner(args).map_err(|error|{eprintln!("{error}");2})?;let _lock=install_lock().map_err(|error|{eprintln!("{error}");3})?;if action=="--install-traffic-service"{install_inner(owner)}else{uninstall_inner(owner)}.map_err(|error|{eprintln!("{error}");2})?;Ok::<i32,i32>(0)})().unwrap_or_else(|code|code)),_=>None
    }
}
unsafe fn dispatch()->Result<(),String>{let mut name=wide(NAME);let table=[SERVICE_TABLE_ENTRYW{lpServiceName:PWSTR(name.as_mut_ptr()),lpServiceProc:Some(service_main)},SERVICE_TABLE_ENTRYW::default()];StartServiceCtrlDispatcherW(table.as_ptr()).map_err(win)}
unsafe extern "system" fn control(control:u32,_kind:u32,_data:*mut std::ffi::c_void,_context:*mut std::ffi::c_void)->u32{if control==SERVICE_CONTROL_STOP{STOP.store(true,Ordering::Release);}0}
unsafe extern "system" fn service_main(_count:u32,_args:*mut PWSTR){
    let name=wide(NAME);let Ok(handle)=RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()),Some(control),None)else{return};STOP.store(false,Ordering::Release);
    report(handle,SERVICE_START_PENDING,0,0,1);let result=broker(handle);report(handle,SERVICE_STOPPED,0,if result.is_ok(){0}else{1},0);
}
fn report(handle:SERVICE_STATUS_HANDLE,state:SERVICE_STATUS_CURRENT_STATE,controls:u32,error:u32,checkpoint:u32){unsafe{let status=SERVICE_STATUS{dwServiceType:SERVICE_WIN32_OWN_PROCESS,dwCurrentState:state,dwControlsAccepted:controls,dwWin32ExitCode:error,dwServiceSpecificExitCode:0,dwCheckPoint:checkpoint,dwWaitHint:if checkpoint==0{0}else{5000}};let _=SetServiceStatus(handle,&status);}}

struct BrokerPipe(OwnedHandle);
impl BrokerPipe{
    fn server(owner:&str)->Result<Self,String>{unsafe{let security=descriptor(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{owner})"))?;let attributes=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>()as u32,lpSecurityDescriptor:security.0.0,bInheritHandle:false.into()};let name=wide(PIPE);let raw=CreateNamedPipeW(PCWSTR(name.as_ptr()),PIPE_ACCESS_DUPLEX|FILE_FLAG_FIRST_PIPE_INSTANCE,PIPE_TYPE_MESSAGE|PIPE_READMODE_MESSAGE|PIPE_NOWAIT|PIPE_REJECT_REMOTE_CLIENTS,1,8192,8192,1000,Some(&attributes));if raw.is_invalid(){return Err("无法创建辅助服务通信管道。".into())}Ok(Self(OwnedHandle(raw)))}}
    fn client()->Result<Self,String>{unsafe{let name=wide(PIPE);let raw=CreateFileW(PCWSTR(name.as_ptr()),(GENERIC_READ|GENERIC_WRITE).0,FILE_SHARE_MODE(0),None,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,None).map_err(win)?;let pipe=Self(OwnedHandle(raw));let mode=PIPE_READMODE_MESSAGE|PIPE_NOWAIT;SetNamedPipeHandleState(pipe.0.0,Some(&mode),None,None).map_err(win)?;Ok(pipe)}}
    fn read<T:for<'de>Deserialize<'de>>(&self)->Result<Option<T>,String>{unsafe{let(mut total,mut remaining)=(0,0);PeekNamedPipe(self.0.0,None,0,None,Some(&mut total),Some(&mut remaining)).map_err(win)?;if total==0{return Ok(None)}if remaining==0||remaining as usize>MAX_FRAME{return Err("辅助服务消息大小无效。".into())}let mut bytes=vec![0u8;remaining as usize];let mut count=0;ReadFile(self.0.0,Some(&mut bytes),Some(&mut count),None).map_err(win)?;bytes.truncate(count as usize);serde_json::from_slice(&bytes).map(Some).map_err(|_|"辅助服务消息格式无效。".into())}}
    fn write<T:Serialize>(&self,value:&T)->Result<(),String>{unsafe{let bytes=serde_json::to_vec(value).map_err(|_|"辅助服务消息编码失败。")?;if bytes.len()>MAX_FRAME{return Err("辅助服务消息大小无效。".into())}let mut count=0;WriteFile(self.0.0,Some(&bytes),Some(&mut count),None).map_err(win)?;if count as usize!=bytes.len(){return Err("辅助服务消息写入不完整。".into())}Ok(())}}
}
fn valid_token(token:&str)->bool{token.len()==32&&token.bytes().all(|v|v.is_ascii_hexdigit())}
fn trusted_gui(parent:HANDLE,record:&Receipt)->Result<(),String>{if process_sid(parent)?!=record.owner_sid||sha256(&process_image(parent)?)?!=record.sha256{return Err("辅助服务客户端身份或版本不匹配。".into())}Ok(())}
fn installed_self()->Result<Receipt,String>{let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;if record.phase!="installed"||own_image()!=Ok(image.canonicalize().map_err(|_|"辅助服务映像不存在。")?)||current_sid()?!="S-1-5-18"||record.sha256!=self_hash()?{return Err("当前进程不是受保护的系统采集辅助程序。".into())}Ok(record)}
pub(crate) fn validate_service_collector(parent:HANDLE)->Result<(),String>{let record=installed_self()?;trusted_gui(parent,&record)}
pub(crate) fn validate_collector(handle:HANDLE)->Result<(),String>{let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;if record.owner_sid!=current_sid()?||record.sha256!=self_hash()?||process_image(handle)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?{return Err("采集辅助程序身份不匹配。".into())}Ok(())}
pub(crate) fn collector_broker()->Result<OwnedHandle,String>{unsafe{
    let record=installed_self()?;let(_,image)=paths()?;let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|READ_CONTROL.0)?.ok_or("采集辅助服务已退出。")?;validate_config(&service,&image,&record.owner_sid)?;let state=status(&service)?;
    if state.dwCurrentState!=SERVICE_RUNNING||state.dwProcessId==0||state.dwProcessId==GetCurrentProcessId(){return Err("采集辅助服务身份无效。".into())}
    let snapshot=OwnedHandle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0).map_err(win)?);let mut entry=PROCESSENTRY32W::default();entry.dwSize=size_of::<PROCESSENTRY32W>()as u32;let mut parent=None;let mut more=Process32FirstW(snapshot.0,&mut entry).is_ok();while more{if entry.th32ProcessID==GetCurrentProcessId(){parent=Some(entry.th32ParentProcessID);break}more=Process32NextW(snapshot.0,&mut entry).is_ok();}
    if parent!=Some(state.dwProcessId){return Err("采集器不属于当前辅助服务。".into())}let process=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE,false,state.dwProcessId).map_err(win)?);
    if process_sid(process.0)?!="S-1-5-18"||process_image(process.0)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?||identity_on(process.0).map_err(win)?.created_filetime>identity_on(GetCurrentProcess()).map_err(win)?.created_filetime||status(&service)?.dwProcessId!=state.dwProcessId||WaitForSingleObject(process.0,0)!=WAIT_TIMEOUT{return Err("采集辅助服务身份已变化。".into())}Ok(process)
}}
fn broker(handle:SERVICE_STATUS_HANDLE)->Result<(),String>{
    let record=installed_self()?;
    // Only this broker process is changed: the GUI may inspect/wait, never mutate,
    // terminate, duplicate handles, change security, or inherit a broader token DACL.
    let query=(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE).0;let all=PROCESS_ALL_ACCESS.0;
    let security=descriptor(&format!("D:P(A;;0x{all:x};;;SY)(A;;0x{all:x};;;BA)(A;;0x{query:x};;;{})",record.owner_sid))?;
    unsafe{SetKernelObjectSecurity(GetCurrentProcess(),DACL_SECURITY_INFORMATION,security.0).map_err(win)?;}
    let pipe=BrokerPipe::server(&record.owner_sid)?;let mut children=Vec::<Child>::new();let mut idle=Instant::now();let mut checkpoint=1;
    report(handle,SERVICE_RUNNING,SERVICE_ACCEPT_STOP,0,0);
    loop{
        children.retain_mut(|child|child.try_wait().ok().flatten().is_none());if !children.is_empty(){idle=Instant::now();}
        if STOP.load(Ordering::Acquire){if children.is_empty(){break}checkpoint+=1;report(handle,SERVICE_STOP_PENDING,0,0,checkpoint);thread::sleep(Duration::from_millis(100));continue}
        if children.is_empty()&&idle.elapsed()>=Duration::from_secs(30){break}
        report(handle,SERVICE_RUNNING,if children.is_empty(){SERVICE_ACCEPT_STOP}else{0},0,0);
        let connected=unsafe{match ConnectNamedPipe(pipe.0.0,None){Ok(())=>true,Err(error)if code(&error)==535=>true,Err(error)if code(&error)==536=>false,Err(error)=>return Err(win(error))}};
        if connected{
            let result=serve_request(&pipe,&record,&mut children);if let Err(error)=result{let _=pipe.write(&Reply{protocol:PROTOCOL,handle:0,child:None,error:Some(error)});}unsafe{let _=DisconnectNamedPipe(pipe.0.0);}idle=Instant::now();
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
fn serve_request(pipe:&BrokerPipe,record:&Receipt,children:&mut Vec<Child>)->Result<(),String>{unsafe{
    let mut client=0;GetNamedPipeClientProcessId(pipe.0.0,&mut client).map_err(win)?;let peer=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE|PROCESS_DUP_HANDLE,false,client).map_err(win)?);trusted_gui(peer.0,record)?;let identity=identity_on(peer.0).map_err(win)?;
    let deadline=Instant::now()+Duration::from_secs(3);let request:Request=loop{if STOP.load(Ordering::Acquire)||WaitForSingleObject(peer.0,0)!=WAIT_TIMEOUT{return Err("辅助服务请求已取消。".into())}if let Some(request)=pipe.read()?{break request}if Instant::now()>=deadline{return Err("辅助服务请求超时。".into())}thread::sleep(Duration::from_millis(10));};
    if request.protocol!=PROTOCOL||request.parent!=identity||!valid_token(&request.token){return Err("辅助服务请求身份不匹配。".into())}if children.len()>=4{return Err("辅助服务采集器数量已达上限。".into())}
    // The only launch target and arguments are fixed; no request-supplied executable or directory is accepted.
    let(_,image)=paths()?;let child=Command::new(&image).args(["--traffic-collector","--pipe-token",&request.token,"--parent-pid",&identity.pid.to_string(),"--parent-created",&identity.created_filetime.to_string()]).current_dir(image.parent().ok_or("辅助服务目录无效。")?).creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"系统采集器未能启动。")?;
    let child_handle=HANDLE(child.as_raw_handle());children.push(child);let child_identity=identity_on(child_handle).map_err(win)?;let mut remote=HANDLE::default();DuplicateHandle(GetCurrentProcess(),child_handle,peer.0,&mut remote,(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE).0,false,DUPLICATE_HANDLE_OPTIONS(0)).map_err(win)?;
    let response=Reply{protocol:PROTOCOL,handle:remote.0 as usize as u64,child:Some(child_identity),error:None};if let Err(error)=pipe.write(&response){let mut local=HANDLE::default();let _=DuplicateHandle(peer.0,remote,GetCurrentProcess(),&mut local,0,false,DUPLICATE_CLOSE_SOURCE);if !local.is_invalid(){let _=CloseHandle(local);}return Err(error)}Ok(())
}}
pub(crate) fn start_collector(token:&str,parent:ProcessIdentity)->Result<OwnedHandle,String>{unsafe{
    if !valid_token(token)||identity_on(GetCurrentProcess()).map_err(win)?!=parent{return Err("采集请求身份无效。".into())}let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;if record.phase!="installed"||record.owner_sid!=current_sid()?||record.sha256!=self_hash()?{return Err("辅助服务未安装或版本不同。".into())}
    let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_START|READ_CONTROL.0)?.ok_or("辅助服务未安装。")?;validate_config(&service,&image,&record.owner_sid)?;
    match StartServiceW(service.0,None){Ok(())=>{},Err(error)if code(&error)==1056=>{},Err(error)=>return Err(win(error))}
    let deadline=Instant::now()+Duration::from_secs(8);let pipe=loop{let state=status(&service)?.dwCurrentState;if state==SERVICE_RUNNING{if let Ok(pipe)=BrokerPipe::client(){break pipe}}else if state==SERVICE_STOPPED{match StartServiceW(service.0,None){Ok(())=>{},Err(error)if code(&error)==1056=>{},Err(error)=>return Err(win(error))}}if Instant::now()>=deadline{return Err("辅助服务启动尚未完成。".into())}thread::sleep(Duration::from_millis(25));};
    let mut server=0;GetNamedPipeServerProcessId(pipe.0.0,&mut server).map_err(win)?;let state=status(&service)?;if state.dwCurrentState!=SERVICE_RUNNING||state.dwProcessId!=server{return Err("辅助服务管道身份不匹配。".into())}let service_process=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,server).map_err(win)?);if process_image(service_process.0)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?{return Err("辅助服务进程映像不匹配。".into())}
    pipe.write(&Request{protocol:PROTOCOL,token:token.into(),parent})?;let deadline=Instant::now()+Duration::from_secs(8);let reply:Reply=loop{if let Some(reply)=pipe.read()?{break reply}if Instant::now()>=deadline{return Err("辅助服务采集器响应超时。".into())}thread::sleep(Duration::from_millis(10));};
    if reply.protocol!=PROTOCOL||reply.handle==0||reply.error.is_some(){return Err(reply.error.unwrap_or_else(||"辅助服务采集器启动失败。".into()))}let handle=OwnedHandle(HANDLE(reply.handle as usize as *mut std::ffi::c_void));if identity_on(handle.0).map_err(win)?!=reply.child.ok_or("辅助服务进程身份缺失。")?{return Err("辅助服务返回的进程身份不匹配。".into())}validate_collector(handle.0)?;Ok(handle)
}}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn broker_protocol_rejects_commands_and_unbounded_tokens(){
        assert!(valid_token("0123456789abcdef0123456789abcdef"));assert!(!valid_token("../codex.exe"));assert!(!valid_token(&"a".repeat(33)));
        assert!(serde_json::from_str::<Request>(r#"{"protocol":1,"token":"0123456789abcdef0123456789abcdef","parent":{"pid":1,"created_filetime":2},"command":"cmd.exe"}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"protocol":1,"token":"0123456789abcdef0123456789abcdef","parent":{"pid":1,"created_filetime":2,"path":"C:\\evil.exe"}}"#).is_err());
    }
}
