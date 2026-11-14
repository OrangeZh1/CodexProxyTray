//! Demand-start LocalSystem broker for the optional ETW collector.
//! Only explicit install/uninstall actions request UAC. The broker accepts no paths or commands.
use std::{collections::{HashMap,HashSet},ffi::{OsStr,OsString},fs::{self,File,OpenOptions},io::{Read,Write},mem::size_of,os::windows::{ffi::{OsStrExt,OsStringExt},fs::MetadataExt,io::AsRawHandle,process::CommandExt},path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{OnceLock,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
use serde::{Deserialize,Serialize};
use windows::{core::{PCWSTR,PWSTR},Win32::{Foundation::*,Security::{*,Authorization::*,Cryptography::*},Storage::FileSystem::*,System::{Diagnostics::ToolHelp::*,Pipes::*,Services::*,Threading::*},UI::Shell::{SHGetFolderPathW,ShellExecuteExW,SHELLEXECUTEINFOW,SEE_MASK_NOCLOSEPROCESS}}};
use crate::{traffic::{OwnedHandle,identity_on,open_identity},traffic_core::ProcessIdentity};

const NAME:&str="CodexProxyTrayTraffic";
const DISPLAY:&str="CodexProxyTray Traffic Collector";
const PIPE:&str=r"\\.\pipe\CodexProxyTray.Traffic.Service.v2";
const IMAGE:&str="CodexProxyTrayTrafficService.exe";
const LOADER:&str="WebView2Loader.dll";
const IMAGE_PENDING:&str="CodexProxyTrayTrafficService.exe.pending";
const LOADER_PENDING:&str="WebView2Loader.dll.pending";
const LOADER_HASH:&str="86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5";
const RECEIPT:&str="receipt.json";
const PENDING:&str="receipt.pending.json";
const RECOVERY:&str="TrafficServiceRecovery";
const JOURNAL:&str="journal.json";
const JOURNAL_PENDING:&str="journal.pending.json";
const CANDIDATE:&str="candidate.exe";
const PREVIOUS:&str="previous.exe";
const RETIRED:&str="retired.exe";
const PRODUCT:&str="CodexProxyTray.TrafficService";
const FILE_SDDL:&str="O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
const PROTOCOL:u32=2;
// Independent of the GUI release. Raise the minimum only for required helper fixes.
const SERVICE_VERSION:&str="1.0.1";
const MIN_SERVICE_VERSION:&str="1.0.1";
const MAX_FRAME:usize=4096;
static SELF_HASH:OnceLock<Result<String,String>>=OnceLock::new();
static STOP:AtomicBool=AtomicBool::new(false);

#[derive(Clone,Serialize)]
pub struct ServiceSnapshot {pub state:String,pub version:Option<String>,pub error:Option<String>}
impl ServiceSnapshot {fn new(state:&str,version:Option<String>,error:Option<String>)->Self{Self{state:state.into(),version,error}}}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {schema:u32,product:String,owner_sid:String,version:String,#[serde(default)]protocol:u32,sha256:String,loader_sha256:String,image:String,service_command:String,phase:String,readiness:String,previous:Option<String>,created_parent:bool}
#[derive(Clone,Serialize,Deserialize)]#[serde(deny_unknown_fields)]
struct UpdateJournal{schema:u32,owner_sid:String,phase:String,old:Receipt,new:Receipt,previous:Option<Receipt>,was_running:bool}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {protocol:u32,token:String,parent:ProcessIdentity}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {protocol:u32,handle:u64,child:Option<ProcessIdentity>,error:Option<String>}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyReceived {protocol:u32}
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
        let expected=match sid.as_str(){"S-1-5-18"|"S-1-5-32-544"=>if file{0x1f01ff}else{0x1f0001},"S-1-5-32-545" if file=>0x1200a9,_=>return Err("辅助服务访问控制含有未知用户。".into())};if ace.Mask!=expected||!seen.insert(sid){return Err("辅助服务访问控制权限不匹配。".into())}}
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
    if directory.join(RECEIPT).exists(){let previous=load_receipt(directory,image,true)?;if previous.owner_sid!=next.owner_sid||previous.sha256!=next.sha256||previous.version!=next.version||previous.protocol!=next.protocol||previous.created_parent!=next.created_parent||!matches!((previous.phase.as_str(),next.phase.as_str()),("prepared","prepared")|("prepared","installed")|("installed","removing")|("removing","removing")){return Err("待恢复安装记录与现有记录不匹配，已保留。".into())}}
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
    if let Some(journal)=read_update(&directory,&image)?{if journal.owner_sid!=record.owner_sid{return Err("辅助服务更新记录属于其他用户。".into())}let recovery=recovery_directory(&directory)?;if journal.phase!="committed"||journal.previous.is_some()||[JOURNAL_PENDING,CANDIDATE,RETIRED].iter().any(|name|recovery.join(name).exists()){validate_update_layout(&directory,&image,&journal)?;return Ok(ServiceSnapshot::new("outdated",Some(record.version.clone()),Some("辅助服务更新尚待恢复或收尾，请直接更新辅助服务重试；原偏好已保留。".into())))}}
    if let Err(error)=compatible(&record){return Ok(ServiceSnapshot::new("outdated",Some(record.version),Some(error)))}
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
fn recovery_directory(directory:&Path)->Result<PathBuf,String>{Ok(directory.parent().ok_or("辅助服务目录无效。")?.join(RECOVERY))}
fn receipt_equal(a:&Receipt,b:&Receipt)->bool{serde_json::to_vec(a).ok().is_some_and(|bytes|serde_json::to_vec(b).ok()==Some(bytes))}
fn previous_equal(a:Option<&Receipt>,b:Option<&Receipt>)->bool{match(a,b){(None,None)=>true,(Some(a),Some(b))=>receipt_equal(a,b),_=>false}}
fn update_inventory(directory:&Path)->Result<(),String>{
    if !directory.exists(){return Ok(())}no_reparse(directory)?;check_file_acl(directory)?;
    for entry in fs::read_dir(directory).map_err(|_|"辅助服务恢复目录不可读取。")?{let entry=entry.map_err(|_|"辅助服务恢复目录不可读取。")?;if !matches!(entry.file_name().to_str(),Some(JOURNAL)|Some(JOURNAL_PENDING)|Some(CANDIDATE)|Some(PREVIOUS)|Some(RETIRED))||!entry.file_type().map_err(|_|"辅助服务恢复文件无效。")?.is_file(){return Err("辅助服务恢复目录存在未知文件，已保留。".into())}no_reparse(&entry.path())?;check_file_acl(&entry.path())?;}Ok(())
}
fn update_policy(record:&Receipt)->Result<bool,String>{
    if record.protocol!=PROTOCOL||record.phase!="installed"||record.readiness!="scm_configuration_verified"||record.version.starts_with('v')||crate::core::version_parts(&record.version).is_none(){return Err("辅助服务旧版本、协议或就绪记录无法确认，未覆盖。".into())}
    // GUI rebuilds do not require replacing a compatible helper of the same/newer version.
    Ok(crate::core::newer_version(SERVICE_VERSION,&record.version))
}
fn validate_update(journal:&UpdateJournal,directory:&Path,image:&Path)->Result<(),String>{
    if journal.schema!=1||!matches!(journal.phase.as_str(),"prepared"|"committed")||journal.owner_sid!=journal.old.owner_sid||journal.owner_sid!=journal.new.owner_sid||journal.old.phase!="installed"||journal.new.phase!="installed"||journal.old.readiness!="scm_configuration_verified"||journal.new.readiness!="scm_configuration_verified"||journal.old.protocol!=PROTOCOL||journal.new.protocol!=PROTOCOL||journal.old.sha256==journal.new.sha256||journal.old.created_parent!=journal.new.created_parent||!crate::core::newer_version(&journal.new.version,&journal.old.version){return Err("辅助服务更新记录无效，恢复材料已保留。".into())}
    for record in [&journal.old,&journal.new]{validate_record(record,directory,image,false)?;if record.version.starts_with('v')||crate::core::version_parts(&record.version).is_none(){return Err("辅助服务更新版本无效。".into())}}
    if let Some(previous)=&journal.previous{validate_record(previous,directory,image,false)?;if previous.owner_sid!=journal.owner_sid||previous.protocol!=PROTOCOL||previous.phase!="installed"||previous.created_parent!=journal.old.created_parent||!crate::core::newer_version(&journal.old.version,&previous.version){return Err("辅助服务上一版本恢复记录无效。".into())}}Ok(())
}
fn read_update_bytes(path:&Path)->Result<Vec<u8>,String>{let file=File::open(path).map_err(|_|"辅助服务更新记录不可读取。")?;let mut bytes=vec![];file.take(32769).read_to_end(&mut bytes).map_err(|_|"辅助服务更新记录不可读取。")?;if bytes.len()>32768{return Err("辅助服务更新记录过大。".into())}Ok(bytes)}
fn read_update_file(path:&Path)->Result<UpdateJournal,String>{serde_json::from_slice(&read_update_bytes(path)?).map_err(|_|"辅助服务更新记录损坏，已保留。".into())}
fn update_pending_prefix(journal:&UpdateJournal,bytes:&[u8])->bool{
    let mut next=journal.clone();if next.phase=="prepared"{next.phase="committed".into();}else{next.previous=None;}
    if serde_json::to_vec_pretty(&next).is_ok_and(|expected|bytes.len()<expected.len()&&expected.starts_with(bytes)){return true}
    if journal.phase=="prepared"{if let Some(previous)=&journal.previous{let restored=UpdateJournal{schema:1,owner_sid:journal.owner_sid.clone(),phase:"committed".into(),old:previous.clone(),new:journal.old.clone(),previous:None,was_running:journal.was_running};return serde_json::to_vec_pretty(&restored).is_ok_and(|expected|bytes.len()<expected.len()&&expected.starts_with(bytes))}}false
}
fn update_transition(old:&UpdateJournal,next:&UpdateJournal)->bool{
    if old.owner_sid!=next.owner_sid{return false}
    if receipt_equal(&old.old,&next.old)&&receipt_equal(&old.new,&next.new)&&old.was_running==next.was_running{return match(old.phase.as_str(),next.phase.as_str()){
        ("prepared","prepared"|"committed")=>previous_equal(old.previous.as_ref(),next.previous.as_ref()),
        ("committed","committed")=>previous_equal(old.previous.as_ref(),next.previous.as_ref())||next.previous.is_none(),_=>false}}
    if old.phase=="committed"&&next.phase=="prepared"{return receipt_equal(&old.new,&next.old)&&next.previous.as_ref().is_some_and(|previous|receipt_equal(previous,&old.old))}
    old.phase=="prepared"&&next.phase=="committed"&&old.was_running==next.was_running&&receipt_equal(&old.old,&next.new)&&old.previous.as_ref().is_some_and(|previous|receipt_equal(previous,&next.old))&&next.previous.is_none()
}
fn read_update(directory:&Path,image:&Path)->Result<Option<UpdateJournal>,String>{
    check_chain(directory)?;let recovery=recovery_directory(directory)?;update_inventory(&recovery)?;if !recovery.exists(){return Ok(None)}
    let committed=recovery.join(JOURNAL);let pending=recovery.join(JOURNAL_PENDING);let old=if committed.exists(){let value=read_update_file(&committed)?;validate_update(&value,directory,image)?;Some(value)}else{None};
    if pending.exists(){let bytes=read_update_bytes(&pending)?;let next=match serde_json::from_slice::<UpdateJournal>(&bytes){Ok(next)=>next,Err(_)=>{if let Some(old)=&old{if update_pending_prefix(old,&bytes){return Ok(Some(old.clone()))}}return Err("辅助服务更新暂存记录损坏，未知内容已保留。".into())}};validate_update(&next,directory,image)?;if let Some(old)=&old{if !update_transition(old,&next){return Err("辅助服务更新前后记录不匹配，已保留。".into())}}else if next.phase!="prepared"||next.previous.is_some()||!receipt_equal(&read_record(&directory.join(RECEIPT))?,&next.old)||sha256(image)?!=next.old.sha256||fs::read_dir(&recovery).map_err(|_|"辅助服务恢复目录不可读取。")?.any(|entry|entry.map_or(true,|entry|entry.file_name()!=JOURNAL_PENDING)){return Err("辅助服务更新缺少已验证的前序状态。".into())}return Ok(Some(next))}
    if old.is_none()&&fs::read_dir(&recovery).map_err(|_|"辅助服务恢复目录不可读取。")?.next().is_some(){return Err("辅助服务恢复文件缺少归属记录。".into())}Ok(old)
}
fn write_update(directory:&Path,journal:&UpdateJournal)->Result<(),String>{
    update_inventory(directory)?;let pending=directory.join(JOURNAL_PENDING);if pending.exists(){return Err("辅助服务更新记录尚未恢复。".into())}let bytes=serde_json::to_vec_pretty(journal).map_err(|_|"辅助服务更新记录编码失败。")?;if bytes.len()>32768{return Err("辅助服务更新记录过大。".into())}let mut file=OpenOptions::new().create_new(true).write(true).open(&pending).map_err(|_|"辅助服务更新记录不可写入。")?;protect(&pending)?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"辅助服务更新记录未能持久保存。".to_owned())?;drop(file);let from=wide(&pending);let to=wide(directory.join(JOURNAL));unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH).map_err(win)}
}
fn update_hash(path:&Path)->Result<Option<String>,String>{if path.exists(){no_reparse(path)?;check_file_acl(path)?;sha256(path).map(Some)}else{Ok(None)}}
fn move_update_file(from:&Path,to:&Path,hash:&str)->Result<(),String>{if update_hash(from)?.as_deref()!=Some(hash)||to.exists(){return Err("辅助服务更新文件已变化，原文件已保留。".into())}let from=wide(from);let to=wide(to);unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_WRITE_THROUGH).map_err(win)}}
fn remove_update_file(path:&Path,hash:&str)->Result<(),String>{if let Some(actual)=update_hash(path)?{if actual!=hash{return Err("辅助服务恢复文件校验不匹配，已保留。".into())}fs::remove_file(path).map_err(|_|"辅助服务恢复文件仍在使用，已保留。")?}Ok(())}
fn validate_update_layout(directory:&Path,image:&Path,journal:&UpdateJournal)->Result<(),String>{
    let recovery=recovery_directory(directory)?;let current=update_hash(image)?;let previous=update_hash(&recovery.join(PREVIOUS))?;let retired=update_hash(&recovery.join(RETIRED))?;
    let prior=journal.previous.as_ref().map(|record|record.sha256.as_str());
    if retired.as_deref().is_some_and(|hash|Some(hash)!=prior){return Err("辅助服务历史恢复副本无法确认。".into())}
    if journal.phase=="committed"{if current.as_deref()!=Some(journal.new.sha256.as_str())||previous.as_deref()!=Some(journal.old.sha256.as_str())||recovery.join(CANDIDATE).exists()||!receipt_equal(&read_record(&directory.join(RECEIPT))?,&journal.new){return Err("辅助服务已提交布局与更新记录不匹配。".into())}return Ok(())}
    if current.as_deref()==Some(journal.old.sha256.as_str()){
        if previous.as_deref()!=prior&&!(previous.is_none()&&retired.as_deref()==prior){return Err("辅助服务更新前的恢复副本不匹配。".into())}
    }else if current.as_deref()==Some(journal.new.sha256.as_str())||current.is_none(){
        if previous.as_deref()!=Some(journal.old.sha256.as_str())||retired.as_deref()!=prior{return Err("辅助服务上一版本或历史副本缺失。".into())}
        if let Some(candidate)=update_hash(&recovery.join(CANDIDATE))?{if current.is_some()||candidate!=journal.new.sha256{return Err("辅助服务更新暂存副本不匹配。".into())}}
    }else{return Err("辅助服务映像不属于本次更新。".into())}
    let record=read_record(&directory.join(RECEIPT))?;if !receipt_equal(&record,&journal.old)&&!receipt_equal(&record,&journal.new){return Err("辅助服务安装记录不属于本次更新。".into())}Ok(())
}
fn service_process(service:&Sc,image:&Path,hash:&str)->Result<Option<OwnedHandle>,String>{unsafe{
    let state=status(service)?;if state.dwCurrentState==SERVICE_STOPPED{return Ok(None)}if state.dwCurrentState!=SERVICE_RUNNING||state.dwProcessId==0{return Err("辅助服务仍在启停，尚不能核验运行实例，请稍后重试。".into())}let process=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE,false,state.dwProcessId).map_err(win)?);let identity=identity_on(process.0).map_err(win)?;let confirmed=status(service)?;
    if process_sid(process.0)?!="S-1-5-18"||process_image(process.0)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?||sha256(image)?!=hash||confirmed.dwCurrentState!=SERVICE_RUNNING||confirmed.dwProcessId!=identity.pid{return Err("辅助服务运行实例身份无法确认。".into())}Ok(Some(process))
}}
fn stop_update_service(service:&Sc,image:&Path,hash:&str)->Result<(),String>{let process=service_process(service,image,hash)?;let identity=process.as_ref().map(|process|identity_on(process.0).map_err(win)).transpose()?;stop_service(service)?;if let Some(process)=process{if unsafe{WaitForSingleObject(process.0,5000)}!=WAIT_OBJECT_0||identity_on(process.0).map_err(win)?!=identity.unwrap(){return Err("辅助服务进程尚未退出，未替换程序。".into())}}Ok(())}
fn check_update_health(service:&Sc,image:&Path,record:&Receipt)->Result<(),String>{unsafe{
    validate_config(service,image,&record.owner_sid)?;match StartServiceW(service.0,None){Ok(())=>{},Err(error)if code(&error)==1056=>{},Err(error)=>return Err(win(error))}
    let deadline=Instant::now()+Duration::from_secs(10);let mut ready=None;loop{let state=status(service)?;if state.dwCurrentState==SERVICE_RUNNING{let process=service_process(service,image,&record.sha256)?.ok_or("辅助服务运行标识缺失。")?;let identity=identity_on(process.0).map_err(win)?;if let Some((before,at))=ready{if before==identity&&Instant::now().duration_since(at)>=Duration::from_millis(300){return Ok(())}}else{ready=Some((identity,Instant::now()));}}else{ready=None;}if Instant::now()>=deadline{return Err("新辅助服务未通过真实 SCM 就绪检查。".into())}thread::sleep(Duration::from_millis(50));}
}}
fn discard_update_receipt(directory:&Path,journal:&UpdateJournal)->Result<(),String>{let path=directory.join(PENDING);if !path.exists(){return Ok(())}no_reparse(&path)?;check_file_acl(&path)?;let mut bytes=Vec::new();File::open(&path).and_then(|file|file.take(16385).read_to_end(&mut bytes)).map_err(|_|"辅助服务待恢复记录不可读取。")?;if bytes.len()>16384||![&journal.old,&journal.new].iter().any(|record|serde_json::to_vec_pretty(record).is_ok_and(|expected|expected.starts_with(&bytes))){return Err("辅助服务待恢复记录不属于本次更新，已保留。".into())}fs::remove_file(path).map_err(|_|"辅助服务待恢复记录仍在使用。".into())}
fn finish_update(directory:&Path,image:&Path,journal:&mut UpdateJournal)->Result<(),String>{
    let recovery=recovery_directory(directory)?;update_inventory(&recovery)?;if update_hash(image)?.as_deref()!=Some(journal.new.sha256.as_str())||!receipt_equal(&read_record(&directory.join(RECEIPT))?,&journal.new)||update_hash(&recovery.join(PREVIOUS))?.as_deref()!=Some(journal.old.sha256.as_str())||recovery.join(CANDIDATE).exists(){return Err("已提交辅助服务与恢复记录不匹配。".into())}
    if let Some(previous)=&journal.previous{remove_update_file(&recovery.join(RETIRED),&previous.sha256)?;}else if recovery.join(RETIRED).exists(){return Err("辅助服务有未知的历史副本，已保留。".into())}
    if journal.previous.take().is_some(){write_update(&recovery,journal)?;}Ok(())
}
fn rollback_update(directory:&Path,image:&Path,service:&Sc,journal:&UpdateJournal)->Result<(),String>{
    let recovery=recovery_directory(directory)?;update_inventory(&recovery)?;let previous=recovery.join(PREVIOUS);let retired=recovery.join(RETIRED);let candidate=recovery.join(CANDIDATE);let actual=update_hash(image)?;
    if actual.as_ref().is_some_and(|hash|hash!=&journal.old.sha256&&hash!=&journal.new.sha256){return Err("辅助服务现有映像无法归属到本次更新。".into())}
    if let Some(hash)=&actual{stop_update_service(service,image,hash)?;}else if status(service)?.dwCurrentState!=SERVICE_STOPPED{return Err("辅助服务映像缺失且系统实例仍在运行。".into())}
    if actual.as_deref()!=Some(journal.old.sha256.as_str()){if update_hash(&previous)?.as_deref()!=Some(journal.old.sha256.as_str()){return Err("辅助服务上一版本副本缺失，恢复材料已保留。".into())}if actual.is_some(){move_update_file(image,&candidate,&journal.new.sha256)?;}move_update_file(&previous,image,&journal.old.sha256)?;}
    discard_update_receipt(directory,journal)?;write_receipt(directory,&journal.old)?;
    if let Some(old_previous)=&journal.previous{match(update_hash(&previous)?,update_hash(&retired)?){(Some(hash),None)if hash==old_previous.sha256=>{},(None,Some(hash))if hash==old_previous.sha256=>move_update_file(&retired,&previous,&old_previous.sha256)?,_=>return Err("辅助服务较早恢复副本无法确认。".into())}}else if previous.exists()||retired.exists(){return Err("辅助服务恢复目录出现未知历史副本。".into())}
    // The journal was durable before candidate creation; an incomplete candidate is owned staging only.
    if candidate.exists(){no_reparse(&candidate)?;check_file_acl(&candidate)?;fs::remove_file(&candidate).map_err(|_|"辅助服务暂存副本仍在使用。")?;}
    check_update_health(service,image,&journal.old)?;if !journal.was_running{stop_update_service(service,image,&journal.old.sha256)?;}
    if let Some(previous)=&journal.previous{write_update(&recovery,&UpdateJournal{schema:1,owner_sid:journal.owner_sid.clone(),phase:"committed".into(),old:previous.clone(),new:journal.old.clone(),previous:None,was_running:journal.was_running})?;}else{fs::remove_file(recovery.join(JOURNAL)).map_err(|_|"辅助服务回退记录清理未完成。")?;fs::remove_dir(&recovery).map_err(|_|"辅助服务空恢复目录清理未完成。")?;}Ok(())
}
fn recover_update(directory:&Path,image:&Path,owner:&str,service:Option<&Sc>)->Result<(),String>{
    let Some(mut journal)=read_update(directory,image)?else{return Ok(())};if journal.owner_sid!=owner{return Err("辅助服务更新记录属于其他用户。".into())}let recovery=recovery_directory(directory)?;
    if journal.phase=="committed"&&service.is_none()&&!directory.exists(){remove_update_recovery(directory,image,&journal.new)?;return Ok(())}
    validate_update_layout(directory,image,&journal)?;
    if recovery.join(JOURNAL_PENDING).exists(){let pending=recovery.join(JOURNAL_PENDING);if read_update_file(&pending).is_err(){if !update_pending_prefix(&journal,&read_update_bytes(&pending)?){return Err("辅助服务更新暂存记录无法归属。".into())}fs::remove_file(pending).map_err(|_|"辅助服务更新暂存记录仍在使用。")?;}else{let from=wide(pending);let to=wide(recovery.join(JOURNAL));unsafe{MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH).map_err(win)?;}}}
    if journal.phase=="committed"{return finish_update(directory,image,&mut journal)}if crate::core::newer_version(&journal.new.version,SERVICE_VERSION){return Err("请使用较新的助手继续辅助服务更新恢复。".into())}
    let service=service.ok_or("辅助服务更新恢复缺少已验证的 SCM 注册，已保留。")?;validate_config(service,image,owner)?;rollback_update(directory,image,service,&journal)
}
fn check_update_removal(directory:&Path,image:&Path,record:&Receipt)->Result<(),String>{
    let Some(journal)=read_update(directory,image)?else{return Ok(())};let recovery=recovery_directory(directory)?;
    if journal.phase!="committed"||journal.owner_sid!=record.owner_sid||journal.new.sha256!=record.sha256||journal.new.version!=record.version||recovery.join(CANDIDATE).exists()||recovery.join(JOURNAL_PENDING).exists(){return Err("辅助服务更新恢复尚未完成，已保留。".into())}
    if let Some(hash)=update_hash(&recovery.join(PREVIOUS))?{if hash!=journal.old.sha256{return Err("辅助服务上一版本校验不匹配。".into())}}else if record.phase!="removing"{return Err("辅助服务上一版本恢复副本缺失。".into())}
    if let Some(hash)=update_hash(&recovery.join(RETIRED))?{if journal.previous.as_ref().is_none_or(|previous|previous.sha256!=hash){return Err("辅助服务存在未知历史恢复副本。".into())}}Ok(())
}
fn remove_update_recovery(directory:&Path,image:&Path,record:&Receipt)->Result<(),String>{
    check_update_removal(directory,image,record)?;
    let recovery=recovery_directory(directory)?;let Some(journal)=read_update(directory,image)?else{if recovery.exists(){fs::remove_dir(recovery).map_err(|_|"辅助服务空恢复目录仍在使用。")?;}return Ok(())};
    if journal.phase!="committed"||journal.owner_sid!=record.owner_sid||journal.new.sha256!=record.sha256||journal.new.version!=record.version||recovery.join(CANDIDATE).exists()||recovery.join(JOURNAL_PENDING).exists(){return Err("辅助服务恢复资料尚未归档，已保留。".into())}
    if let Some(previous)=&journal.previous{remove_update_file(&recovery.join(RETIRED),&previous.sha256)?;}else if recovery.join(RETIRED).exists(){return Err("辅助服务恢复目录存在未知历史副本。".into())}remove_update_file(&recovery.join(PREVIOUS),&journal.old.sha256)?;fs::remove_file(recovery.join(JOURNAL)).map_err(|_|"辅助服务恢复记录清理失败。")?;fs::remove_dir(recovery).map_err(|_|"辅助服务恢复目录仍在使用。".to_owned())?;Ok(())
}
fn upgrade_inner(directory:&Path,image:&Path,service:&Sc,old:Receipt,source:&Path)->Result<(),String>{
    let recovery=recovery_directory(directory)?;let prior=read_update(directory,image)?;if let Some(prior)=&prior{if prior.phase!="committed"||!receipt_equal(&prior.new,&old){return Err("辅助服务已有未结束更新，已保留。".into())}}
    secure_directory(&recovery)?;let mut next=old.clone();next.version=SERVICE_VERSION.into();next.protocol=PROTOCOL;next.sha256=self_hash()?;next.phase="installed".into();next.readiness="scm_configuration_verified".into();
    let mut journal=UpdateJournal{schema:1,owner_sid:old.owner_sid.clone(),phase:"prepared".into(),old,new:next,previous:prior.map(|value|value.old),was_running:status(service)?.dwCurrentState!=SERVICE_STOPPED};validate_update(&journal,directory,image)?;write_update(&recovery,&journal)?;
    let result=(||->Result<(),String>{
        let candidate=recovery.join(CANDIDATE);if candidate.exists(){return Err("辅助服务暂存文件已经存在，已保留。".into())}let mut input=File::open(source).map_err(|_|"辅助服务源文件不可读取。")?;let mut output=OpenOptions::new().create_new(true).write(true).open(&candidate).map_err(|_|"辅助服务暂存文件不可创建。")?;protect(&candidate)?;std::io::copy(&mut input,&mut output).and_then(|_|output.sync_all()).map_err(|_|"辅助服务新副本未能持久保存。")?;drop(output);if update_hash(&candidate)?.as_deref()!=Some(journal.new.sha256.as_str()){return Err("辅助服务新副本校验不匹配。".into())}
        stop_update_service(service,image,&journal.old.sha256)?;if let Some(previous)=&journal.previous{move_update_file(&recovery.join(PREVIOUS),&recovery.join(RETIRED),&previous.sha256)?;}move_update_file(image,&recovery.join(PREVIOUS),&journal.old.sha256)?;move_update_file(&candidate,image,&journal.new.sha256)?;
        write_receipt(directory,&journal.new)?;check_update_health(service,image,&journal.new)?;if !journal.was_running{stop_update_service(service,image,&journal.new.sha256)?;}journal.phase="committed".into();write_update(&recovery,&journal)?;finish_update(directory,image,&mut journal)
    })();if let Err(error)=result{if journal.phase=="committed"{return Err(format!("新辅助服务已就绪，但恢复副本收尾未完成：{error}"))}return match recover_update(directory,image,&journal.owner_sid,Some(service)){Ok(())=>Err(format!("辅助服务更新未完成，已恢复原版本：{error}")),Err(recovery)=>Err(format!("辅助服务更新未完成；恢复材料已保留：{error} {recovery}"))}}Ok(())
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
    let(directory,image)=paths()?;check_chain(&directory)?;let manager=manager(SC_MANAGER_CONNECT|SC_MANAGER_CREATE_SERVICE)?;let existing=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_START|SERVICE_STOP|WRITE_DAC.0|WRITE_OWNER.0|READ_CONTROL.0)?;
    recover_update(&directory,&image,&owner,existing.as_ref())?;
    recover_pending(&directory,&image,&owner,existing.as_ref())?;
    let prior=if directory.join(RECEIPT).exists(){Some(load_receipt(&directory,&image,true)?)}else{None};
    if let Some(existing)=&existing{let record=prior.as_ref().ok_or("同名服务缺少本程序安装记录，已拒绝覆盖。")?;validate_config_inner(existing,&image,&record.owner_sid,record.phase!="prepared")?;if record.owner_sid!=owner{return Err("辅助服务属于其他用户，已拒绝覆盖。".into())}if record.phase=="installed"{if update_policy(record)?{return upgrade_inner(&directory,&image,existing,record.clone(),&source_image)}compatible(record)?;return Ok(())}}
    if let Some(record)=&prior{if record.owner_sid!=owner{return Err("辅助服务属于其他用户，已拒绝覆盖。".into())}if record.phase=="removing"{return Err("辅助服务卸载尚未完成，请继续卸载。".into())}if record.sha256!=self_hash()?{return Err("辅助服务存在其他版本的未完成安装，原文件已保留。".into())}}
    else if directory.exists(){inventory(&directory)?;if fs::read_dir(&directory).map_err(|_|"辅助服务目录不可读取。")?.next().is_some(){return Err("辅助服务目录缺少归属记录，已保留。".into())}}
    let created_parent=prior.as_ref().map_or(!directory.parent().ok_or("辅助服务目录无效。")?.exists(),|record|record.created_parent);
    secure_directory(directory.parent().ok_or("辅助服务目录无效。")?)?;secure_directory(&directory)?;
    let mut record=Receipt{schema:1,product:PRODUCT.into(),owner_sid:owner.clone(),version:SERVICE_VERSION.into(),protocol:PROTOCOL,sha256:self_hash()?,loader_sha256:LOADER_HASH.into(),image:image.to_string_lossy().into(),service_command:command(&image)?,phase:"prepared".into(),readiness:"copy_pending".into(),previous:None,created_parent};if prior.is_none(){write_receipt(&directory,&record)?;}
    install_file(&source_image,&image,&directory.join(IMAGE_PENDING),&record.sha256)?;install_file(&source_loader,&directory.join(LOADER),&directory.join(LOADER_PENDING),LOADER_HASH)?;record.readiness="files_verified".into();write_receipt(&directory,&record)?;
    let installed=if let Some(existing)=existing{existing}else{let name=wide(NAME);let display=wide(DISPLAY);let binary=wide(&record.service_command);unsafe{Sc(CreateServiceW(manager.0,PCWSTR(name.as_ptr()),PCWSTR(display.as_ptr()),SERVICE_ALL_ACCESS,SERVICE_WIN32_OWN_PROCESS,SERVICE_DEMAND_START,SERVICE_ERROR_NORMAL,PCWSTR(binary.as_ptr()),PCWSTR::null(),None,PCWSTR::null(),PCWSTR::null(),PCWSTR::null()).map_err(win)?)} };
    let acl=descriptor(&format!("O:BAG:BAD:P(A;;0xf01ff;;;SY)(A;;0xf01ff;;;BA)(A;;0x20015;;;{owner})"))?;unsafe{SetServiceObjectSecurity(installed.0,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,acl.0).map_err(win)?;}validate_config(&installed,&image,&owner)?;
    record.phase="installed".into();record.readiness="scm_configuration_verified".into();write_receipt(&directory,&record)?;Ok(())
}
fn uninstall_inner(owner:String)->Result<(),String>{
    let(directory,image)=paths()?;let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_START|SERVICE_STOP|DELETE.0|READ_CONTROL.0)?;
    if let Some(journal)=read_update(&directory,&image)?{if journal.owner_sid!=owner{return Err("辅助服务恢复记录属于其他用户。".into())}if journal.phase=="prepared"{recover_update(&directory,&image,&owner,service.as_ref())?;}else if service.is_none()&&!directory.exists(){remove_update_recovery(&directory,&image,&journal.new)?;return Ok(())}}
    if service.is_none()&&!directory.exists(){return Ok(())}recover_pending(&directory,&image,&owner,service.as_ref())?;let mut record=load_receipt(&directory,&image,true)?;if record.owner_sid!=owner{return Err("辅助服务属于其他用户，已拒绝删除。".into())}check_update_removal(&directory,&image,&record)?;
    if directory.join(PENDING).exists(){return Err("辅助服务有待恢复记录，已保留安装。".into())}if let Some(service)=service{validate_config_inner(&service,&image,&owner,record.phase!="prepared")?;stop_update_service(&service,&image,&record.sha256)?;record.phase="removing".into();record.readiness="service_stopped_for_removal".into();write_receipt(&directory,&record)?;unsafe{DeleteService(service.0).map_err(win)?;}drop(service);
        let deadline=Instant::now()+Duration::from_secs(5);loop{match self::service(&manager,SERVICE_QUERY_STATUS){Ok(None)=>break,Err(_)=>return Err("辅助服务删除尚未确认，文件已保留。".into()),Ok(Some(_))=>{if Instant::now()>=deadline{return Err("辅助服务仍在删除，文件已保留。".into())}thread::sleep(Duration::from_millis(50));}}}}
    // Exact owned allowlist only. Never recursively remove Program Files or the parent product directory.
    if record.phase!="removing"{record.phase="removing".into();record.readiness="service_absent_for_removal".into();write_receipt(&directory,&record)?;}
    inventory(&directory)?;for path in [&image,&directory.join(LOADER),&directory.join(IMAGE_PENDING),&directory.join(LOADER_PENDING)]{if path.exists(){fs::remove_file(path).map_err(|_|"辅助服务文件仍被使用，已保留卸载恢复记录。")?;}}remove_update_recovery(&directory,&image,&record)?;fs::remove_file(directory.join(RECEIPT)).map_err(|_|"辅助服务安装记录清理未完成。")?;
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
    fn reply(&self,value:&Reply)->Result<(),String>{
        self.write(value)?;self.wait_received()
    }
    fn wait_received(&self)->Result<(),String>{let deadline=Instant::now()+Duration::from_secs(3);
        loop{if STOP.load(Ordering::Acquire){return Err("辅助服务正在停止。".into())}if let Some(received)=self.read::<ReplyReceived>()?{if received.protocol!=PROTOCOL{return Err("辅助服务响应确认无效。".into())}return Ok(())}if Instant::now()>=deadline{return Err("辅助服务响应未被客户端确认。".into())}thread::sleep(Duration::from_millis(10));}
    }
}
fn valid_token(token:&str)->bool{token.len()==32&&token.bytes().all(|v|v.is_ascii_hexdigit())}
fn compatible(record:&Receipt)->Result<(),String>{
    if record.protocol!=PROTOCOL{return Err("辅助服务协议不匹配，请使用匹配的新版本助手更新辅助服务；未降级或覆盖现有程序。".into())}if crate::core::version_parts(&record.version).is_none(){return Err("辅助服务版本记录无效，原文件已保留。".into())}if crate::core::newer_version(MIN_SERVICE_VERSION,&record.version){return Err("辅助服务版本较旧，请直接更新辅助服务；不需要手动卸载，原统计偏好会保留。".into())}Ok(())
}
fn update_in_progress(directory:&Path,image:&Path,owner:&str)->Result<bool,String>{let Some(journal)=read_update(directory,image)?else{return Ok(false)};if journal.owner_sid!=owner{return Err("辅助服务更新归属不匹配。".into())}Ok(journal.phase!="committed"||recovery_directory(directory)?.join(JOURNAL_PENDING).exists())}
// The trust boundary is the installed owner SID, not a particular GUI build.
// Requests can only start our protected collector; application scope and SID/lifetime are independently checked there.
fn trusted_gui(parent:HANDLE,record:&Receipt)->Result<(),String>{if process_sid(parent)?!=record.owner_sid{return Err("辅助服务客户端用户身份不匹配。".into())}Ok(())}
struct UserContext(OwnedHandle);
struct Impersonation;
impl Impersonation{fn begin(token:HANDLE)->Result<Self,String>{unsafe{ImpersonateLoggedOnUser(token).map_err(win)?;}Ok(Self)}}
impl Drop for Impersonation{fn drop(&mut self){
    // Never continue privileged collector work with an unexpected thread identity.
    if unsafe{RevertToSelf()}.is_err(){std::process::abort();}
}}
impl UserContext{
    fn from_parent(parent:HANDLE)->Result<Self,String>{unsafe{
        let mut raw=HANDLE::default();OpenProcessToken(parent,TOKEN_QUERY|TOKEN_DUPLICATE,&mut raw).map_err(win)?;let source=OwnedHandle(raw);let mut token=HANDLE::default();
        DuplicateTokenEx(source.0,TOKEN_QUERY|TOKEN_IMPERSONATE|TOKEN_DUPLICATE,None,SecurityImpersonation,TokenImpersonation,&mut token).map_err(win)?;Ok(Self(OwnedHandle(token)))
    }}
    fn root(&self)->Result<PathBuf,String>{unsafe{
        // The installed owner's real OS profile, never a client-supplied path or LOCALAPPDATA.
        let mut buffer=[0u16;260];SHGetFolderPathW(None,0x1c,Some(self.0.0),0,&mut buffer).map_err(win)?;
        let end=buffer.iter().position(|&value|value==0).ok_or("用户配置目录无效。")?;let base=PathBuf::from(OsString::from_wide(&buffer[..end]));
        let text=base.to_string_lossy().to_ascii_lowercase();if !base.is_absolute()||text.starts_with(r"\\?\unc\")||(text.starts_with(r"\\")&&!text.starts_with(r"\\?\")){return Err("辅助服务只读取本机用户配置。".into())}Ok(base.join("CodexProxyTray"))
    }}
    fn targets(&self,root:&Path,anchors:&[&crate::native::ProcessLease],requested:&[ProcessIdentity],owner:&str)->Result<Vec<crate::native::ProcessLease>,String>{
        let _user=Impersonation::begin(self.0.0)?;
        let config=scope_config(root)?;let mut roots=Vec::new();
        // Requested identities are lookup hints only. The service reopens and verifies the
        // real root object, including a root that exited after the GUI retained its handle.
        for &identity in requested{if let Some(lease)=crate::native::traffic_root_lease(root,&config,identity)?{let sid=process_sid(HANDLE(lease.as_handle())).map_err(|error|format!("无法核验应用根的用户归属（PID {}，创建标识 {}）：{error}",identity.pid,identity.created_filetime))?;if sid==owner{roots.push(lease);}}}
        let all=anchors.iter().copied().chain(roots.iter()).collect::<Vec<_>>();let mut discovered=crate::native::traffic_targets_with_anchors(root,&config,&all)?;discovered.extend(roots);Ok(discovered)
    }
}
#[derive(Deserialize)]struct ScopeConfig{#[serde(default)]cli_command:Vec<String>}
fn scope_config(root:&Path)->Result<crate::settings::Config,String>{
    let file=match File::open(root.join("config.json")){Ok(file)=>file,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(crate::settings::Config::default()),Err(_)=>return Err("辅助服务无法读取本用户配置，未扩大采样范围。".into())};
    let metadata=file.metadata().map_err(|_|"辅助服务无法核验本用户配置。")?;if !metadata.is_file()||metadata.len()>65536{return Err("本用户配置超过辅助服务读取上限。".into())}
    let mut bytes=Vec::new();file.take(65537).read_to_end(&mut bytes).map_err(|_|"辅助服务无法读取本用户配置。")?;if bytes.len()>65536{return Err("本用户配置超过辅助服务读取上限。".into())}
    if bytes.iter().find(|byte|!byte.is_ascii_whitespace())!=Some(&b'{'){return Err("本用户配置格式不受支持，未扩大采样范围。".into())}
    // Only this stable, bounded field is consumed; unrelated GUI preferences may evolve independently.
    let scope:ScopeConfig=serde_json::from_slice(&bytes).map_err(|_|"本用户 CLI 配置格式无效，未扩大采样范围。")?;
    if scope.cli_command.len()>16||scope.cli_command.iter().any(|part|part.is_empty()||part.contains('\0'))||scope.cli_command.iter().map(|part|part.encode_utf16().count()).sum::<usize>()>32767{return Err("本用户 CLI 配置超过辅助服务读取范围。".into())}
    Ok(crate::settings::Config{cli_command:scope.cli_command,..crate::settings::Config::default()})
}
fn lease_belongs_to(lease:&crate::native::ProcessLease,owner:&str)->bool{process_sid(HANDLE(lease.as_handle())).is_ok_and(|sid|sid==owner)}
pub(crate) struct CollectorScope{
    user:UserContext,root:PathBuf,owner_sid:String,allowed:HashMap<ProcessIdentity,(Instant,crate::native::ProcessLease)>,missed:HashSet<ProcessIdentity>,first_missed:Option<u64>,last_missed:Option<u64>,
}
impl CollectorScope{
    pub(crate) fn new(parent:HANDLE)->Result<Self,String>{let owner_sid=process_sid(parent)?;let user=UserContext::from_parent(parent)?;let root=user.root()?;Ok(Self{user,root,owner_sid,allowed:HashMap::new(),missed:HashSet::new(),first_missed:None,last_missed:None})}
    pub(crate) fn targets(&mut self,requested:&[ProcessIdentity])->Result<Vec<ProcessIdentity>,String>{
        crate::traffic_core::validate_targets(requested)?;let now=Instant::now();let mut expired=Vec::new();
        for (&identity,(seen,lease)) in &mut self.allowed{
            if lease.identity!=identity{return Err(format!("已核验目标的创建身份发生变化（PID {}，创建标识 {}），已拒绝采样。",identity.pid,identity.created_filetime))}
            if lease.is_running()?{if !lease_belongs_to(lease,&self.owner_sid){return Err(format!("已核验目标的用户归属无法确认（PID {}，创建标识 {}），已拒绝采样。",identity.pid,identity.created_filetime))}*seen=now;}
            // The exact held object already passed SID validation while alive.
            // Exited tombstones cannot acquire a new identity; expire them without reopening a dead token.
            else if now.saturating_duration_since(*seen)>=Duration::from_secs(60){expired.push(identity);}
        }
        for identity in expired{self.allowed.remove(&identity);}
        let unknown=requested.iter().filter(|identity|!self.allowed.contains_key(identity)).copied().collect::<Vec<_>>();
        let anchors=self.allowed.values().map(|(_,lease)|lease).collect::<Vec<_>>();let discovered=self.user.targets(&self.root,&anchors,&unknown,&self.owner_sid)?;
        for lease in discovered{if !lease_belongs_to(&lease,&self.owner_sid){continue}if let Some((seen,_))=self.allowed.get_mut(&lease.identity){*seen=now;}else{if self.allowed.len()>=4096{return Err("辅助服务进程交接保留数量达到上限。".into())}self.allowed.insert(lease.identity,(now,lease));}}
        let mut accepted=Vec::new();for &identity in requested{
            if self.allowed.contains_key(&identity){accepted.push(identity);continue}
            let alive=match crate::traffic::open_lifetime_identity(identity).map_err(win)?{Some(process)=>match unsafe{WaitForSingleObject(process.0,0)}{WAIT_TIMEOUT=>true,WAIT_OBJECT_0=>false,_=>return Err("辅助服务无法核验目标进程状态。".into())},None=>false};
            if alive{return Err(format!("目标进程不属于已核验的 ChatGPT / Codex CLI 范围，已拒绝采样（PID {}，创建标识 {}；服务未能独立确认应用根或完整父链）。",identity.pid,identity.created_filetime))}
            // An exited identity without our own application-scope proof never enters ETW bindings.
            if self.missed.len()>=4096&&!self.missed.contains(&identity){return Err("辅助服务遗漏身份记录达到上限。".into())}if self.missed.insert(identity){let at=crate::settings::now();self.first_missed=Some(self.first_missed.map_or(at,|first|first.min(at)));self.last_missed=Some(at);}
        }Ok(accepted)
    }
    pub(crate) fn mark_sample(&self,sample:&mut crate::traffic_etw::EngineSample){
        if self.missed.is_empty(){return}sample.partial=true;sample.had_targets=true;sample.missed_bindings=sample.missed_bindings.saturating_add(self.missed.len()as u64);sample.first_missed_at=match(sample.first_missed_at,self.first_missed){(Some(a),Some(b))=>Some(a.min(b)),(a,b)=>a.or(b)};sample.last_missed_at=match(sample.last_missed_at,self.last_missed){(Some(a),Some(b))=>Some(a.max(b)),(a,b)=>a.or(b)};
    }
}
fn installed_self()->Result<Receipt,String>{let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;if record.protocol!=PROTOCOL||record.version!=SERVICE_VERSION||record.phase!="installed"||own_image()!=Ok(image.canonicalize().map_err(|_|"辅助服务映像不存在。")?)||current_sid()?!="S-1-5-18"||record.sha256!=self_hash()?{return Err("当前进程不是受保护的系统采集辅助程序。".into())}Ok(record)}
pub(crate) fn validate_service_collector(parent:HANDLE)->Result<(),String>{let record=installed_self()?;trusted_gui(parent,&record)}
pub(crate) fn validate_collector(handle:HANDLE)->Result<(),String>{let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;compatible(&record)?;if record.phase!="installed"||record.owner_sid!=current_sid()?||process_image(handle)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?{return Err("采集辅助程序身份不匹配。".into())}Ok(())}
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
            let result=serve_request(&pipe,&record,&mut children);if let Err(error)=result{let _=pipe.reply(&Reply{protocol:PROTOCOL,handle:0,child:None,error:Some(error)});}unsafe{let _=DisconnectNamedPipe(pipe.0.0);}idle=Instant::now();
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}
fn serve_request(pipe:&BrokerPipe,record:&Receipt,children:&mut Vec<Child>)->Result<(),String>{unsafe{
    let(directory,image)=paths()?;if update_in_progress(&directory,&image,&record.owner_sid)?{return Err("辅助服务正在更新，未启动新的采集器。".into())}
    let mut client=0;GetNamedPipeClientProcessId(pipe.0.0,&mut client).map_err(win)?;let peer=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE|PROCESS_DUP_HANDLE,false,client).map_err(win)?);trusted_gui(peer.0,record)?;let identity=identity_on(peer.0).map_err(win)?;
    let deadline=Instant::now()+Duration::from_secs(3);let request:Request=loop{if STOP.load(Ordering::Acquire)||WaitForSingleObject(peer.0,0)!=WAIT_TIMEOUT{return Err("辅助服务请求已取消。".into())}if let Some(request)=pipe.read()?{break request}if Instant::now()>=deadline{return Err("辅助服务请求超时。".into())}thread::sleep(Duration::from_millis(10));};
    if request.protocol!=PROTOCOL||request.parent!=identity||!valid_token(&request.token){return Err("辅助服务请求身份不匹配。".into())}if children.len()>=4{return Err("辅助服务采集器数量已达上限。".into())}
    // The only launch target and arguments are fixed; no request-supplied executable or directory is accepted.
    let(_,image)=paths()?;let child=Command::new(&image).args(["--traffic-collector","--pipe-token",&request.token,"--parent-pid",&identity.pid.to_string(),"--parent-created",&identity.created_filetime.to_string()]).current_dir(image.parent().ok_or("辅助服务目录无效。")?).creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"系统采集器未能启动。")?;
    let child_handle=HANDLE(child.as_raw_handle());children.push(child);let child_identity=identity_on(child_handle).map_err(win)?;let mut remote=HANDLE::default();DuplicateHandle(GetCurrentProcess(),child_handle,peer.0,&mut remote,(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE).0,false,DUPLICATE_HANDLE_OPTIONS(0)).map_err(win)?;
    let response=Reply{protocol:PROTOCOL,handle:remote.0 as usize as u64,child:Some(child_identity),error:None};if let Err(error)=pipe.write(&response){let mut local=HANDLE::default();let _=DuplicateHandle(peer.0,remote,GetCurrentProcess(),&mut local,0,false,DUPLICATE_CLOSE_SOURCE);if !local.is_invalid(){let _=CloseHandle(local);}return Err(error)}
    // DisconnectNamedPipe discards unread replies. The client confirms the frame before this connection is released.
    pipe.wait_received()
}}
pub(crate) fn start_collector(token:&str,parent:ProcessIdentity)->Result<OwnedHandle,String>{unsafe{
    if !valid_token(token)||identity_on(GetCurrentProcess()).map_err(win)?!=parent{return Err("采集请求身份无效。".into())}let(directory,image)=paths()?;let record=load_receipt(&directory,&image,true)?;compatible(&record)?;if record.phase!="installed"||record.owner_sid!=current_sid()?{return Err("辅助服务未安装或用户身份不同。".into())}
    if update_in_progress(&directory,&image,&record.owner_sid)?{return Err("辅助服务更新尚未完成，请重试更新；统计偏好已保留。".into())}
    let manager=manager(SC_MANAGER_CONNECT)?;let service=service(&manager,SERVICE_QUERY_CONFIG|SERVICE_QUERY_STATUS|SERVICE_START|READ_CONTROL.0)?.ok_or("辅助服务未安装。")?;validate_config(&service,&image,&record.owner_sid)?;
    match StartServiceW(service.0,None){Ok(())=>{},Err(error)if code(&error)==1056=>{},Err(error)=>return Err(win(error))}
    let deadline=Instant::now()+Duration::from_secs(8);let pipe=loop{let state=status(&service)?.dwCurrentState;if state==SERVICE_RUNNING{if let Ok(pipe)=BrokerPipe::client(){break pipe}}else if state==SERVICE_STOPPED{match StartServiceW(service.0,None){Ok(())=>{},Err(error)if code(&error)==1056=>{},Err(error)=>return Err(win(error))}}if Instant::now()>=deadline{return Err("辅助服务启动尚未完成。".into())}thread::sleep(Duration::from_millis(25));};
    let mut server=0;GetNamedPipeServerProcessId(pipe.0.0,&mut server).map_err(win)?;let state=status(&service)?;if state.dwCurrentState!=SERVICE_RUNNING||state.dwProcessId!=server{return Err("辅助服务管道身份不匹配。".into())}let service_process=OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,server).map_err(win)?);if process_image(service_process.0)?!=image.canonicalize().map_err(|_|"辅助服务映像不存在。")?{return Err("辅助服务进程映像不匹配。".into())}
    pipe.write(&Request{protocol:PROTOCOL,token:token.into(),parent})?;let deadline=Instant::now()+Duration::from_secs(8);let reply:Reply=loop{if let Some(reply)=pipe.read()?{break reply}if Instant::now()>=deadline{return Err("辅助服务采集器响应超时。".into())}thread::sleep(Duration::from_millis(10));};
    if reply.protocol!=PROTOCOL{return Err("辅助服务响应协议不匹配。".into())}let returned=if reply.handle!=0{Some(OwnedHandle(HANDLE(reply.handle as usize as *mut std::ffi::c_void)))}else{None};pipe.write(&ReplyReceived{protocol:PROTOCOL})?;
    if reply.error.is_some(){return Err(reply.error.unwrap())}let handle=returned.ok_or("辅助服务采集器启动失败。")?;if identity_on(handle.0).map_err(win)?!=reply.child.ok_or("辅助服务进程身份缺失。")?{return Err("辅助服务返回的进程身份不匹配。".into())}validate_collector(handle.0)?;
        #[cfg(test)]tests::track_probe_collector(&handle,token)?;
    Ok(handle)
}}

#[cfg(test)]mod tests{
    use super::*;
    static PROBE_ACTIVE:AtomicBool=AtomicBool::new(false);
    static PROBE_COLLECTOR:std::sync::Mutex<Option<OwnedHandle>>=std::sync::Mutex::new(None);
    static PROBE_TRACE:std::sync::Mutex<Option<String>>=std::sync::Mutex::new(None);
    pub(super)fn track_probe_collector(process:&OwnedHandle,token:&str)->Result<(),String>{
        if !PROBE_ACTIVE.load(Ordering::Acquire){return Ok(())}let mut copy=HANDLE::default();unsafe{DuplicateHandle(GetCurrentProcess(),process.0,GetCurrentProcess(),&mut copy,(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_SYNCHRONIZE).0,false,DUPLICATE_HANDLE_OPTIONS(0)).map_err(win)?;}
        let copy=OwnedHandle(copy);let mut proof=PROBE_COLLECTOR.lock().map_err(|_|"普通权限验收句柄状态无效。")?;if proof.is_some(){return Err("普通权限验收出现重复采集器。".into())}*proof=Some(copy);*PROBE_TRACE.lock().map_err(|_|"验收会话标识状态无效。")?=Some(token.into());Ok(())
    }
    #[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct ScopeReady{identity:ProcessIdentity,port:u16,token:String}
    fn fixture_ready(directory:&Path)->Result<ScopeReady,String>{
        let deadline=Instant::now()+Duration::from_secs(10);loop{match File::open(directory.join("child-ready.json")){
            Ok(file)=>{let mut bytes=Vec::new();file.take(4097).read_to_end(&mut bytes).map_err(|_|"fixture_ready_read")?;if bytes.len()>4096{return Err("fixture_ready_size".into())}if let Ok(ready)=serde_json::from_slice::<ScopeReady>(&bytes){return Ok(ready)}},
            Err(error)if error.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err("fixture_ready_open".into())}
            if Instant::now()>=deadline{return Err("fixture_ready_timeout".into())}thread::sleep(Duration::from_millis(20));
        }
    }
    fn fixture_wait(child:&mut Child)->Result<(),String>{let deadline=Instant::now()+Duration::from_secs(5);loop{if let Some(exit)=child.try_wait().map_err(|_|"fixture_parent_wait")?{return if exit.success(){Ok(())}else{Err("fixture_parent_exit".into())}}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();return Err("fixture_parent_forced_cleanup".into())}thread::sleep(Duration::from_millis(20));}}
    struct ScopeFixture{directory:PathBuf,parent:Child,child:Option<crate::native::ProcessLease>,port:u16,closed:bool}
    impl ScopeFixture{
        fn start(directory:&Path)->Result<Self,String>{
            if fs::read_dir(directory).map_err(|_|"fixture_directory")?.next().is_some(){return Err("fixture_not_fresh".into())}
            let current=std::env::current_exe().map_err(|_|"fixture_image")?;let mut random=[0u8;16];if unsafe{BCryptGenRandom(None,&mut random,BCRYPT_USE_SYSTEM_PREFERRED_RNG)}.0<0{return Err("fixture_random".into())}let token=random.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
            fs::copy(&current,directory.join("codex.exe")).map_err(|_|"fixture_parent_copy")?;fs::copy(&current,directory.join("scope-child.exe")).map_err(|_|"fixture_child_copy")?;
            let parent=Command::new(directory.join("codex.exe")).args(["--exact","traffic_service::tests::scope_process_fixture"]).env("CODEX_PROXY_SCOPE_FIXTURE_ROLE","parent").env("CODEX_PROXY_SCOPE_FIXTURE_DIR",directory).env("CODEX_PROXY_SCOPE_FIXTURE_TOKEN",&token).current_dir(current.parent().ok_or("fixture_cwd")?).creation_flags(0x08000000).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"fixture_parent_start")?;
            let mut fixture=Self{directory:directory.into(),parent,child:None,port:0,closed:false};let ready=fixture_ready(directory)?;if ready.token!=token||ready.port==0{return Err("fixture_ready_binding".into())}
            let(identity,image,_,lease)=unsafe{crate::native::process_identity_with_lease(ready.identity.pid)}.ok_or("fixture_child_identity")?;
            if identity!=ready.identity||image!=crate::settings::normalized_executable(&directory.join("scope-child.exe")){return Err("fixture_child_image".into())}fixture.child=Some(lease);fixture.port=ready.port;Ok(fixture)
        }
        fn parent_identity(&self)->Result<ProcessIdentity,String>{identity_on(HANDLE(self.parent.as_raw_handle())).map_err(win)}
        fn child_identity(&self)->ProcessIdentity{self.child.as_ref().unwrap().identity}
        fn release_parent(&mut self)->Result<(),String>{if let Some(mut input)=self.parent.stdin.take(){input.write_all(b"x").map_err(|_|"fixture_parent_release")?;}fixture_wait(&mut self.parent)}
        fn transfer(&self)->Result<(),String>{use std::net::{TcpStream,SocketAddr};let mut connection=TcpStream::connect_timeout(&SocketAddr::from(([127,0,0,1],self.port)),Duration::from_secs(2)).map_err(|_|"fixture_loopback_connect")?;connection.set_read_timeout(Some(Duration::from_secs(2))).map_err(|_|"fixture_loopback_timeout")?;connection.write_all(b"d").map_err(|_|"fixture_loopback_request")?;let mut bytes=Vec::new();connection.take(32769).read_to_end(&mut bytes).map_err(|_|"fixture_loopback_read")?;if bytes!=vec![7;32768]{return Err("fixture_loopback_bytes".into())}Ok(())}
        fn close_child(&mut self)->Result<(),String>{if let Some(lease)=&self.child{if lease.is_running()?{let mut connection=std::net::TcpStream::connect_timeout(&([127,0,0,1],self.port).into(),Duration::from_secs(2)).map_err(|_|"fixture_stop_connect")?;connection.write_all(b"x").map_err(|_|"fixture_stop_write")?;}if unsafe{WaitForSingleObject(HANDLE(lease.as_handle()),10000)}!=WAIT_OBJECT_0{return Err("fixture_child_stop_unconfirmed".into())}}Ok(())}
        fn finish(&mut self)->Result<(),String>{if self.closed{return Ok(())}let parent=self.release_parent();let child=self.close_child();parent?;child?;let expected=["codex.exe","scope-child.exe","child-ready.json"];if fs::read_dir(&self.directory).map_err(|_|"fixture_cleanup_list")?.any(|entry|match entry{Ok(entry)=>!expected.iter().any(|name|entry.file_name()==OsStr::new(name)),Err(_)=>true}){return Err("fixture_cleanup_unknown_file".into())}self.child.take();for name in expected{fs::remove_file(self.directory.join(name)).map_err(|_|"fixture_cleanup_file")?;}fs::remove_dir(&self.directory).map_err(|_|"fixture_cleanup_directory")?;self.closed=true;Ok(())}
    }
    impl Drop for ScopeFixture{fn drop(&mut self){let _=self.finish();}}
    #[test]fn scope_process_fixture(){
        let Some(role)=std::env::var_os("CODEX_PROXY_SCOPE_FIXTURE_ROLE")else{return};let directory=PathBuf::from(std::env::var_os("CODEX_PROXY_SCOPE_FIXTURE_DIR").expect("fixture directory")).canonicalize().unwrap();let image=std::env::current_exe().unwrap().canonicalize().unwrap();assert_eq!(image.parent(),Some(directory.as_path()));
        let token=std::env::var("CODEX_PROXY_SCOPE_FIXTURE_TOKEN").unwrap();assert!(valid_token(&token));
        if role=="parent"{assert_eq!(image.file_name(),Some(OsStr::new("codex.exe")));let mut child=Command::new(directory.join("scope-child.exe")).args(["--exact","traffic_service::tests::scope_process_fixture"]).env("CODEX_PROXY_SCOPE_FIXTURE_ROLE","child").creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();let _=fixture_ready(&directory).unwrap();let _=std::io::stdin().read_exact(&mut [0u8;1]);let _=child.try_wait();}
        else{assert_eq!(role,"child");assert_eq!(image.file_name(),Some(OsStr::new("scope-child.exe")));let listener=std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).unwrap();listener.set_nonblocking(true).unwrap();let ready=ScopeReady{identity:identity_on(unsafe{GetCurrentProcess()}).unwrap(),port:listener.local_addr().unwrap().port(),token};let mut output=OpenOptions::new().create_new(true).write(true).open(directory.join("child-ready.json")).unwrap();output.write_all(&serde_json::to_vec(&ready).unwrap()).and_then(|_|output.sync_all()).unwrap();drop(output);let deadline=Instant::now()+Duration::from_secs(60);
            while Instant::now()<deadline{match listener.accept(){Ok((mut stream,address))=>{assert!(address.ip().is_loopback());stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();let mut request=[0];if stream.read_exact(&mut request).is_ok(){if request==*b"x"{return}assert_eq!(request,*b"d");stream.write_all(&[7;32768]).unwrap();}},Err(error)if error.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(10)),Err(error)=>panic!("fixture listener: {error}")}}panic!("fixture child lifetime exceeded");
        }
    }
    fn verified_handoff(scope:&mut CollectorScope,fixture:&mut ScopeFixture)->Result<(Vec<crate::native::ProcessLease>,Vec<&'static str>),String>{
        let config=scope_config(&scope.root)?;let parent=fixture.parent_identity()?;let child=fixture.child_identity();let mut leases=crate::native::traffic_targets_held(&scope.root,&config)?;leases.retain(|lease|lease.identity==parent||lease.identity==child);
        if ![parent,child].iter().all(|identity|leases.iter().any(|lease|lease.identity==*identity)){return Err("scope_gui_discovery_missing".into())}let mut checks=vec!["gui_discovered_parent_and_child"];
        fixture.release_parent()?;if unsafe{WaitForSingleObject(HANDLE(fixture.parent.as_raw_handle()),0)}!=WAIT_OBJECT_0||!fixture.child.as_ref().unwrap().is_running()?{return Err("scope_fixture_lifetimes_invalid".into())}checks.push("parent_exited_child_still_alive");
        let accepted=scope.targets(&[parent,child]).map_err(|error|{eprintln!("scope_first_handoff_validation: {}",error.chars().take(500).collect::<String>());"scope_first_handoff_refused"})?;if !accepted.contains(&child)||!scope.allowed.contains_key(&parent){return Err("scope_first_handoff_missing".into())}checks.push("first_handoff_independently_verified");
        scope.allowed.get_mut(&child).ok_or("scope_child_proof_missing")?.0=Instant::now()-Duration::from_secs(61);if !scope.targets(&[child])?.contains(&child){return Err("scope_live_child_expired".into())}checks.push("live_child_proof_survives_sixty_seconds");
        let wrong=ProcessIdentity{pid:child.pid,created_filetime:child.created_filetime.saturating_add(1)};if scope.targets(&[wrong])?.contains(&wrong){return Err("scope_wrong_creation_accepted".into())}checks.push("wrong_creation_excluded");
        let other=fixture.directory.join("other-scope");fs::create_dir(&other).map_err(|_|"scope_other_directory")?;let mut unrelated=ScopeFixture::start(&other)?;
        let root_rejected=scope.targets(&[unrelated.parent_identity()?]).is_err();let child_rejected=scope.targets(&[unrelated.child_identity()]).is_err();let unrelated_rejected=scope.targets(&[identity_on(unsafe{GetCurrentProcess()}).map_err(win)?]).is_err();unrelated.finish()?;
        if !root_rejected||!child_rejected||!unrelated_rejected{return Err("scope_unrelated_alive_accepted".into())}checks.extend(["same_name_other_path_rejected","other_scope_child_rejected","unrelated_alive_rejected"]);Ok((leases,checks))
    }
    #[test]fn exited_application_root_handoff_preserves_live_child_without_trusting_requested_identity(){
        let root=std::env::temp_dir().join(format!("codex-scope-handoff-{}-{}",std::process::id(),crate::settings::now()));fs::create_dir(&root).unwrap();let fixture_dir=root.join("fixture");fs::create_dir(&fixture_dir).unwrap();fs::write(root.join("config.json"),serde_json::to_vec(&serde_json::json!({"cli_command":[fixture_dir.join("codex.exe")]})).unwrap()).unwrap();
        let mut fixture=ScopeFixture::start(&fixture_dir).unwrap();let mut scope=CollectorScope::new(unsafe{GetCurrentProcess()}).unwrap();scope.root=root.clone();let(leases,checks)=verified_handoff(&mut scope,&mut fixture).unwrap();assert_eq!(checks.len(),8);
        let child=fixture.child_identity();fixture.close_child().unwrap();scope.allowed.get_mut(&child).unwrap().0=Instant::now()-Duration::from_secs(61);assert!(!scope.targets(&[child]).unwrap().contains(&child));drop(leases);drop(scope);fixture.finish().unwrap();fs::remove_file(root.join("config.json")).unwrap();fs::remove_dir(root).unwrap();
    }
    fn probe_trace_absent(token:&str)->Result<bool,String>{
        use windows::Win32::System::Diagnostics::Etw::{ControlTraceW,CONTROLTRACE_HANDLE,EVENT_TRACE_PROPERTIES,EVENT_TRACE_CONTROL_QUERY};
        if !valid_token(token){return Err("probe_trace_token_invalid".into())}let mut bytes=vec![0u64;(size_of::<EVENT_TRACE_PROPERTIES>()+4096).div_ceil(8)];let properties=bytes.as_mut_ptr().cast::<EVENT_TRACE_PROPERTIES>();let name=wide(format!("CodexProxyTray.Network.{token}"));
        unsafe{(*properties).Wnode.BufferSize=(bytes.len()*8)as u32;(*properties).LoggerNameOffset=size_of::<EVENT_TRACE_PROPERTIES>()as u32;(*properties).LogFileNameOffset=(*properties).LoggerNameOffset+2048;let result=ControlTraceW(CONTROLTRACE_HANDLE::default(),PCWSTR(name.as_ptr()),properties,EVENT_TRACE_CONTROL_QUERY);match result.0{4201=>Ok(true),0=>Ok(false),_=>Err(format!("probe_trace_query_{}",result.0))}}
    }
    #[test]#[ignore="Requires the isolated service QA case, a fresh synthetic real-profile config and protected SYSTEM service"]
    fn service_scope_handoff_probe(){
        #[derive(Serialize)]struct Probe{schema:u32,status:&'static str,marker:String,scope:&'static str,checks:Vec<&'static str>,samples:usize,stop_ack:bool,collector_exited:bool,collector_exit_code:Option<u32>,trace_absent:bool,fixture_closed:bool,service_version:Option<String>,failure_code:Option<String>,elapsed_ms:u64}
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"));assert_eq!(std::env::var("RUNNER_ENVIRONMENT").as_deref(),Ok("github-hosted"));
        let marker=std::env::var("CODEX_PROXY_SCOPE_PROBE_MARKER").expect("scope probe marker");assert!(marker.len()==64&&marker.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)));
        let boundary=PathBuf::from(std::env::var_os("RUNNER_TEMP").unwrap()).canonicalize().unwrap();let result=PathBuf::from(std::env::var_os("CODEX_PROXY_SCOPE_PROBE_RESULT").expect("scope result"));let directory=PathBuf::from(std::env::var_os("CODEX_PROXY_SCOPE_PROBE_FIXTURE").expect("scope fixture")).canonicalize().unwrap();
        assert!(result.is_absolute()&&result.file_name()==Some(OsStr::new("result.json"))&&result.parent().unwrap().canonicalize().unwrap().starts_with(&boundary));assert!(directory.starts_with(&boundary)&&directory!=boundary&&fs::symlink_metadata(&directory).unwrap().file_attributes()&0x400==0);assert!(fs::read_dir(&directory).unwrap().next().is_none());
        let mut output=OpenOptions::new().create_new(true).write(true).open(&result).expect("fresh scope result");let started=Instant::now();let mut probe=Probe{schema:1,status:"failed",marker,scope:"real service scope handoff; synthetic process tree and loopback only",checks:vec![],samples:0,stop_ack:false,collector_exited:false,collector_exit_code:None,trace_absent:false,fixture_closed:false,service_version:None,failure_code:None,elapsed_ms:0};
        let mut sampler=crate::traffic::Coordinator::new();let mut fixture=None;let mut scope=None;let mut sampling_started=false;
        let outcome=(||->Result<(),String>{
            let user=UserContext::from_parent(unsafe{GetCurrentProcess()})?;let root=user.root()?;let config=scope_config(&root)?;if config.cli_command.len()!=1||crate::settings::normalized_executable(Path::new(&config.cli_command[0]))!=crate::settings::normalized_executable(&directory.join("codex.exe")){return Err("scope_real_profile_config_mismatch".into())}probe.checks.push("real_profile_config_matches_fixture");
            let(service_directory,image)=paths()?;let record=load_receipt(&service_directory,&image,true)?;compatible(&record)?;if record.owner_sid!=current_sid()?{return Err("scope_service_owner_mismatch".into())}probe.service_version=Some(record.version);
            fixture=Some(ScopeFixture::start(&directory)?);scope=Some(CollectorScope::new(unsafe{GetCurrentProcess()})?);let(leases,checks)=verified_handoff(scope.as_mut().unwrap(),fixture.as_mut().unwrap())?;probe.checks.extend(checks);
            sampler.set_targets(leases)?;PROBE_ACTIVE.store(true,Ordering::Release);sampler.enable(crate::traffic::EnableMode::Normal)?;sampling_started=true;let deadline=Instant::now()+Duration::from_secs(25);let mut stamps=HashSet::new();let mut transferred=Instant::now()-Duration::from_secs(1);
            while Instant::now()<deadline{if transferred.elapsed()>=Duration::from_millis(500){fixture.as_ref().unwrap().transfer()?;transferred=Instant::now();}let snapshot=sampler.snapshot();if matches!(snapshot.state,crate::traffic::TrafficState::Unavailable|crate::traffic::TrafficState::NeedsPermission|crate::traffic::TrafficState::NeedsInstallation){return Err("scope_service_sampling_unavailable".into())}if snapshot.state==crate::traffic::TrafficState::Collecting&&snapshot.target_count>0&&snapshot.sent_bytes.is_some_and(|bytes|bytes>0)&&snapshot.received_bytes.is_some_and(|bytes|bytes>0){stamps.insert(snapshot.timestamp_ms);probe.samples=stamps.len();if probe.samples>=2{let trace=PROBE_TRACE.lock().map_err(|_|"scope_trace_lock")?.clone().ok_or("scope_trace_missing")?;if probe_trace_absent(&trace)?{return Err("scope_trace_missing_while_sampling".into())}probe.checks.push("two_service_samples_with_loopback_bytes");return Ok(())}}thread::sleep(Duration::from_millis(100));}Err("scope_service_samples_missing".into())
        })();
        let stopped=sampler.disable();probe.stop_ack=sampling_started&&probe.samples>=2&&stopped.is_ok()&&sampler.snapshot().state==crate::traffic::TrafficState::Disabled;PROBE_ACTIVE.store(false,Ordering::Release);
        if let Some(process)=PROBE_COLLECTOR.lock().unwrap_or_else(|error|error.into_inner()).take(){probe.collector_exited=unsafe{WaitForSingleObject(process.0,10000)}==WAIT_OBJECT_0;if probe.collector_exited{let mut code=0;if unsafe{GetExitCodeProcess(process.0,&mut code)}.is_ok(){probe.collector_exit_code=Some(code);}}}
        if let Some(token)=PROBE_TRACE.lock().unwrap_or_else(|error|error.into_inner()).take(){probe.trace_absent=probe_trace_absent(&token).unwrap_or(false);}
        let cleanup=(||->Result<(),String>{if let Some(fixture)=&mut fixture{fixture.close_child()?;if let Some(scope)=&mut scope{let child=fixture.child_identity();if let Some((seen,_))=scope.allowed.get_mut(&child){*seen=Instant::now()-Duration::from_secs(61);}if scope.targets(&[child])?.contains(&child){return Err("scope_exited_tombstone_not_expired".into())}probe.checks.push("exited_tombstone_expires");}fixture.finish()?;probe.fixture_closed=true;}Ok(())})();
        probe.failure_code=outcome.err().or_else(||cleanup.err()).map(|message|if message.len()<=100&&message.bytes().all(|byte|byte.is_ascii_lowercase()||byte.is_ascii_digit()||byte==b'_'){message}else{"scope_independent_validation_failed".into()}).or_else(||if !probe.stop_ack{Some("scope_stop_ack_missing".into())}else if !probe.collector_exited||probe.collector_exit_code!=Some(0){Some("scope_collector_exit_missing".into())}else if !probe.trace_absent{Some("scope_trace_stop_missing".into())}else if !probe.fixture_closed{Some("scope_fixture_cleanup_missing".into())}else{None});
        if probe.failure_code.is_none(){probe.status="passed";}probe.elapsed_ms=started.elapsed().as_millis().min(u128::from(u64::MAX))as u64;let bytes=serde_json::to_vec(&probe).unwrap();assert!(bytes.len()<=16384);output.write_all(&bytes).and_then(|_|output.sync_all()).expect("durable scope result");assert_eq!(probe.status,"passed","scope probe failed: {:?}",probe.failure_code);
    }
    #[test]#[ignore="Requires the isolated service QA script and a verified unelevated medium token"]
    fn unelevated_service_client_probe(){
        #[derive(Default,Serialize)]struct ProbeToken{elevated:Option<bool>,admin_enabled:Option<bool>,integrity_rid:Option<u32>,owner_sid_matches:Option<bool>}
        #[derive(Serialize)]struct Probe{schema:u32,status:&'static str,marker:String,scope:&'static str,token:ProbeToken,samples:usize,stop_ack:bool,collector_exited:bool,collector_exit_code:Option<u32>,service_version:Option<String>,failure_code:Option<&'static str>,elapsed_ms:u64}
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"));assert_eq!(std::env::var("RUNNER_ENVIRONMENT").as_deref(),Ok("github-hosted"));
        let marker=std::env::var("CODEX_PROXY_SERVICE_PROBE_MARKER").expect("explicit probe marker");assert!(marker.len()==64&&marker.bytes().all(|value|value.is_ascii_digit()||(b'a'..=b'f').contains(&value)));
        let result=PathBuf::from(std::env::var_os("CODEX_PROXY_SERVICE_PROBE_RESULT").expect("explicit result path"));let boundary=PathBuf::from(std::env::var_os("RUNNER_TEMP").expect("runner temp")).canonicalize().unwrap();
        assert!(result.is_absolute()&&result.file_name()==Some(OsStr::new("result.json")));let parent=result.parent().unwrap().canonicalize().unwrap();assert!(parent.starts_with(&boundary));
        let mut output=OpenOptions::new().create_new(true).write(true).open(&result).expect("fresh probe result");let started=Instant::now();
        let mut probe=Probe{schema:1,status:"failed",marker,scope:"unelevated Rust service client; no Tauri or WebView2",token:ProbeToken::default(),samples:0,stop_ack:false,collector_exited:false,collector_exit_code:None,service_version:None,failure_code:None,elapsed_ms:0};
        let mut sampler=crate::traffic::Coordinator::new();let mut sampling_started=false;
        let outcome=(||->Result<(),&'static str>{unsafe{
            let mut raw=HANDLE::default();OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut raw).map_err(|_|"probe_token_open_failed")?;let token=OwnedHandle(raw);let mut returned=0;let mut elevation=TOKEN_ELEVATION::default();
            GetTokenInformation(token.0,TokenElevation,Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),size_of::<TOKEN_ELEVATION>()as u32,&mut returned).map_err(|_|"probe_elevation_query_failed")?;probe.token.elevated=Some(elevation.TokenIsElevated!=0);
            let mut needed=0;let _=GetTokenInformation(token.0,TokenIntegrityLevel,None,0,&mut needed);if needed<size_of::<TOKEN_MANDATORY_LABEL>()as u32||needed>65536{return Err("probe_integrity_size_invalid")}
            let mut label=vec![0u64;(needed as usize).div_ceil(8)];GetTokenInformation(token.0,TokenIntegrityLevel,Some(label.as_mut_ptr().cast()),needed,&mut needed).map_err(|_|"probe_integrity_query_failed")?;
            let sid=sid_string((*(label.as_ptr()as *const TOKEN_MANDATORY_LABEL)).Label.Sid).map_err(|_|"probe_integrity_sid_invalid")?;probe.token.integrity_rid=sid.strip_prefix("S-1-16-").and_then(|value|value.parse().ok());
            let mut admin=PSID::default();let text=wide("S-1-5-32-544");ConvertStringSidToSidW(PCWSTR(text.as_ptr()),&mut admin).map_err(|_|"probe_admin_sid_failed")?;let mut member=windows::core::BOOL::default();let checked=CheckTokenMembership(None,admin,&mut member);let _=LocalFree(Some(HLOCAL(admin.0)));checked.map_err(|_|"probe_admin_query_failed")?;probe.token.admin_enabled=Some(member.as_bool());
            let(directory,image)=paths().map_err(|_|"probe_service_paths_failed")?;let record=load_receipt(&directory,&image,true).map_err(|_|"probe_service_integrity_failed")?;compatible(&record).map_err(|_|"probe_service_incompatible")?;
            probe.token.owner_sid_matches=Some(current_sid().map_err(|_|"probe_owner_query_failed")?==record.owner_sid);probe.service_version=Some(record.version);
            if probe.token.elevated!=Some(false)||probe.token.admin_enabled!=Some(false)||probe.token.integrity_rid!=Some(8192)||probe.token.owner_sid_matches!=Some(true){return Err("probe_not_unelevated_medium_owner")}
            let root=crate::settings::root();let config=crate::settings::load(&root).map_err(|_|"probe_config_invalid")?;sampler.set_targets(crate::native::traffic_targets_held(&root,&config).map_err(|_|"probe_target_discovery_failed")?).map_err(|_|"probe_targets_invalid")?;
            PROBE_ACTIVE.store(true,Ordering::Release);sampler.enable(crate::traffic::EnableMode::Normal).map_err(|_|"probe_enable_failed")?;sampling_started=true;
            let deadline=Instant::now()+Duration::from_secs(30);let mut stamps=HashSet::new();while Instant::now()<deadline{
                let snapshot=sampler.snapshot();if matches!(snapshot.state,crate::traffic::TrafficState::Unavailable|crate::traffic::TrafficState::NeedsPermission|crate::traffic::TrafficState::NeedsInstallation){return Err("probe_service_sampling_unavailable")}
                if snapshot.state==crate::traffic::TrafficState::Collecting&&snapshot.received_bytes.is_some()&&snapshot.sent_bytes.is_some(){stamps.insert(snapshot.timestamp_ms);probe.samples=stamps.len();if probe.samples>=2{return Ok(())}}
                thread::sleep(Duration::from_millis(100));
            }Err("probe_service_samples_missing")
        }})();
        let stopped=sampler.disable();probe.stop_ack=sampling_started&&probe.samples>=2&&stopped.is_ok()&&sampler.snapshot().state==crate::traffic::TrafficState::Disabled;
        PROBE_ACTIVE.store(false,Ordering::Release);if let Some(process)=PROBE_COLLECTOR.lock().unwrap_or_else(|error|error.into_inner()).take(){probe.collector_exited=unsafe{WaitForSingleObject(process.0,10000)}==WAIT_OBJECT_0;if probe.collector_exited{let mut code=0;if unsafe{GetExitCodeProcess(process.0,&mut code)}.is_ok(){probe.collector_exit_code=Some(code);}}}
        probe.failure_code=outcome.err().or(if !probe.stop_ack{Some("probe_stop_ack_missing")}else if !probe.collector_exited||probe.collector_exit_code!=Some(0){Some("probe_collector_exit_missing")}else{None});if probe.failure_code.is_none(){probe.status="passed";}probe.elapsed_ms=started.elapsed().as_millis().min(u128::from(u64::MAX))as u64;
        let bytes=serde_json::to_vec(&probe).unwrap();assert!(bytes.len()<=16384);output.write_all(&bytes).and_then(|_|output.sync_all()).expect("durable probe result");assert_eq!(probe.status,"passed","service client probe failed: {:?}",probe.failure_code);
    }
    #[test]fn protected_scope_rejects_unrelated_live_targets_keeps_tombstones_and_restores_user_context(){
        let root=std::env::temp_dir().join(format!("codex-traffic-scope-{}-{}",std::process::id(),crate::settings::now()));fs::create_dir(&root).unwrap();
        let mut scope=CollectorScope::new(unsafe{GetCurrentProcess()}).unwrap();scope.root=root.clone();
        let(identity,_,_,lease)=unsafe{crate::native::process_identity_with_lease(std::process::id())}.unwrap();assert!(lease_belongs_to(&lease,&scope.owner_sid));
        assert!(scope.targets(&[identity]).is_err());
        scope.allowed.insert(identity,(Instant::now()-Duration::from_secs(59),lease));assert_eq!(scope.targets(&[identity]).unwrap(),vec![identity]);
        scope.allowed.get_mut(&identity).unwrap().0=Instant::now()-Duration::from_secs(61);assert_eq!(scope.targets(&[identity]).unwrap(),vec![identity]);
        let mut child=Command::new(std::env::current_exe().unwrap()).args(["--exact","traffic_etw::tests::short_lived_binding_fixture"]).env("CODEX_TRAFFIC_BINDING_FIXTURE","1").creation_flags(0x08000000).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let(exited,_,_,held)=unsafe{crate::native::process_identity_with_lease(child.id())}.unwrap();assert!(lease_belongs_to(&held,&scope.owner_sid));scope.allowed.insert(exited,(Instant::now(),held));child.stdin.take().unwrap().write_all(b"x").unwrap();assert!(child.wait().unwrap().success());
        assert_eq!(scope.targets(&[exited]).unwrap(),vec![exited]);scope.allowed.get_mut(&exited).unwrap().0=Instant::now()-Duration::from_secs(61);assert!(scope.targets(&[exited]).unwrap().is_empty());
        let unknown=ProcessIdentity{pid:u32::MAX,created_filetime:1};assert!(scope.targets(&[unknown]).unwrap().is_empty());
        let mut sample=crate::traffic_etw::EngineSample{buffer_limited:false,missed_bindings:0,first_missed_at:None,last_missed_at:None,sent:0,received:0,seen:false,had_targets:false,partial:false,target_count:0,lost_events:Some(0),lost_buffers:Some(0),error:None};scope.mark_sample(&mut sample);assert!(sample.partial&&sample.had_targets);assert_eq!(sample.missed_bindings,2);assert!(sample.first_missed_at.is_some());
        fs::write(root.join("config.json"),br#"{"new_gui_feature":{"enabled":true},"network_enabled":"future-format","cli_command":[]}"#).unwrap();assert!(scope.targets(&[]).is_ok());
        for invalid in [b"{broken".as_slice(),br#"{"cli_command":null}"#.as_slice(),br#"{"cli_command":"codex.exe"}"#.as_slice(),br#"{"cli_command":[1]}"#.as_slice(),b"[]".as_slice()]{fs::write(root.join("config.json"),invalid).unwrap();assert!(scope.targets(&[]).is_err());let mut token=HANDLE::default();let error=unsafe{OpenThreadToken(GetCurrentThread(),TOKEN_QUERY,true,&mut token)}.unwrap_err();assert_eq!(code(&error),1008);}
        for command in [vec!["x".repeat(32768)],vec!["x".into();17]]{fs::write(root.join("config.json"),serde_json::to_vec(&serde_json::json!({"cli_command":command})).unwrap()).unwrap();assert!(scope.targets(&[]).is_err());}
        fs::remove_file(root.join("config.json")).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn service_compatibility_is_independent_of_gui_and_image_hash(){
        let mut record=Receipt{schema:1,product:PRODUCT.into(),owner_sid:"S-1-5-21-1".into(),version:SERVICE_VERSION.into(),protocol:PROTOCOL,sha256:"a".repeat(64),loader_sha256:LOADER_HASH.into(),image:String::new(),service_command:String::new(),phase:"installed".into(),readiness:String::new(),previous:None,created_parent:false};
        assert!(compatible(&record).is_ok());record.sha256="b".repeat(64);assert!(compatible(&record).is_ok());
        record.owner_sid=current_sid().unwrap();assert!(trusted_gui(unsafe{GetCurrentProcess()},&record).is_ok());record.owner_sid="S-1-5-21-999999".into();assert!(trusted_gui(unsafe{GetCurrentProcess()},&record).is_err());
        record.version="1.0.0".into();assert!(compatible(&record).is_err());record.version="1.9.0".into();assert!(compatible(&record).is_ok());
        record.version="0.9.0".into();assert!(compatible(&record).is_err());record.version="bad".into();assert!(compatible(&record).is_err());record.version=SERVICE_VERSION.into();record.protocol=0;assert!(compatible(&record).is_err());record.protocol=PROTOCOL+1;assert!(compatible(&record).is_err());
        // Legacy receipts stay readable for a safe explicit uninstall, but cannot claim compatibility.
        let mut legacy=serde_json::to_value(&record).unwrap();legacy.as_object_mut().unwrap().remove("protocol");let legacy:Receipt=serde_json::from_value(legacy).unwrap();assert_eq!(legacy.protocol,0);assert!(compatible(&legacy).is_err());
    }
    #[test]fn explicit_helper_updates_are_versioned_and_journal_transitions_reject_unrelated_backups(){
        let directory=std::env::temp_dir().join(format!("codex-update-policy-{}-{}",std::process::id(),crate::settings::now())).join("TrafficService");let image=directory.join(IMAGE);
        let old=Receipt{schema:1,product:PRODUCT.into(),owner_sid:"S-1-5-21-1".into(),version:"1.0.0".into(),protocol:PROTOCOL,sha256:"a".repeat(64),loader_sha256:LOADER_HASH.into(),image:image.to_string_lossy().into(),service_command:command(&image).unwrap(),phase:"installed".into(),readiness:"scm_configuration_verified".into(),previous:None,created_parent:true};
        assert!(update_policy(&old).unwrap());let mut current=old.clone();current.version=SERVICE_VERSION.into();current.sha256="b".repeat(64);assert!(!update_policy(&current).unwrap());current.sha256="d".repeat(64);assert!(!update_policy(&current).unwrap(),"GUI hash changes do not force helper replacement");current.sha256="b".repeat(64);
        let mut future=current.clone();future.version="1.9.0".into();assert!(!update_policy(&future).unwrap());future.protocol=PROTOCOL+1;assert!(update_policy(&future).is_err());future.protocol=PROTOCOL;future.version="bad".into();assert!(update_policy(&future).is_err());
        let prepared=UpdateJournal{schema:1,owner_sid:old.owner_sid.clone(),phase:"prepared".into(),old:old.clone(),new:current.clone(),previous:None,was_running:true};assert!(validate_update(&prepared,&directory,&image).is_ok());let mut committed=prepared.clone();committed.phase="committed".into();assert!(update_transition(&prepared,&committed));
        let bytes=serde_json::to_vec_pretty(&committed).unwrap();assert!(update_pending_prefix(&prepared,&bytes[..bytes.len()/2]));assert!(!update_pending_prefix(&prepared,b"unowned content"));assert!(!update_pending_prefix(&prepared,&bytes));
        let mut unrelated=committed.clone();unrelated.new.owner_sid="S-1-5-21-2".into();assert!(validate_update(&unrelated,&directory,&image).is_err());unrelated=committed.clone();unrelated.new.version="0.9.0".into();assert!(validate_update(&unrelated,&directory,&image).is_err());
        let mut previous=old.clone();previous.version="0.9.0".into();previous.sha256="c".repeat(64);unrelated=committed.clone();unrelated.previous=Some(previous.clone());assert!(!update_transition(&prepared,&unrelated));let mut with_previous=prepared.clone();with_previous.previous=Some(previous);assert!(validate_update(&with_previous,&directory,&image).is_ok());let mut changed=with_previous.clone();changed.previous.as_mut().unwrap().sha256="e".repeat(64);assert!(!update_transition(&with_previous,&changed));
        let mut value=serde_json::to_value(&prepared).unwrap();value.as_object_mut().unwrap().insert("path".into(),serde_json::Value::String("arbitrary".into()));assert!(serde_json::from_value::<UpdateJournal>(value).is_err());
    }
    #[test]fn broker_protocol_rejects_commands_and_unbounded_tokens(){
        assert!(valid_token("0123456789abcdef0123456789abcdef"));assert!(!valid_token("../codex.exe"));assert!(!valid_token(&"a".repeat(33)));
        assert!(serde_json::from_str::<Request>(r#"{"protocol":1,"token":"0123456789abcdef0123456789abcdef","parent":{"pid":1,"created_filetime":2},"command":"cmd.exe"}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"protocol":1,"token":"0123456789abcdef0123456789abcdef","parent":{"pid":1,"created_filetime":2,"path":"C:\\evil.exe"}}"#).is_err());
        assert!(serde_json::from_str::<ReplyReceived>(r#"{"protocol":2,"command":"cmd.exe"}"#).is_err());
    }
}
