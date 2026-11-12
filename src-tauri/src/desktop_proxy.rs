//! Read-only verification of the five proxy variables for one exact packaged
//! process. Unrelated environment values are never decoded, returned or persisted.
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,ffi::c_void,fs,mem::size_of,path::Path,sync::{Mutex,OnceLock},time::{Duration,Instant}};
use crate::{core,native,settings,traffic_core::ProcessIdentity};

const RECEIPT:&str="desktop-proxy-v1.json";
const KEYS:[&str;5]=["HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","NO_PROXY","CODEX_PROXY_TRAY"];
#[derive(Clone,Serialize)]pub struct ProxyState{pub state:&'static str,pub reason:Option<String>,pub checked_at:Option<u64>}
impl ProxyState{fn new(state:&'static str,reason:Option<String>)->Self{Self{state,reason,checked_at:Some(settings::now())}}}
#[derive(Clone,Serialize,Deserialize)]#[serde(deny_unknown_fields)]
struct Receipt{schema:u32,pid:u32,created_filetime:String,package_family:String,proxy_url:String,verified_at:u64}
struct Cache{receipt:Option<Receipt>,writable:bool,last:Option<(ProcessIdentity,String,Instant,ProxyState)>}
static CACHE:OnceLock<Mutex<Cache>>=OnceLock::new();
fn cache()->&'static Mutex<Cache>{CACHE.get_or_init(||Mutex::new(load(&settings::root())))}
fn load(root:&Path)->Cache{
    let path=root.join(RECEIPT);let loaded=(||->Result<Option<Receipt>,()>{let metadata=match fs::metadata(&path){Ok(value)=>value,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err(())};if metadata.len()>4096{return Err(())}
        let receipt:Receipt=serde_json::from_slice(&fs::read(&path).map_err(|_|())?).map_err(|_|())?;if !valid_receipt(&receipt){return Err(())}Ok(Some(receipt))})();
    Cache{writable:loaded.is_ok(),receipt:loaded.ok().flatten(),last:None}
}
fn valid_receipt(receipt:&Receipt)->bool{
    receipt.schema==1&&receipt.pid!=0&&receipt.created_filetime.bytes().all(|value|value.is_ascii_digit())&&receipt.created_filetime.parse::<u64>().ok().is_some_and(|value|value!=0)&&receipt.package_family==core::FAMILY&&core::endpoint(&receipt.proxy_url).is_ok()&&receipt.verified_at>0&&receipt.verified_at<=8_640_000_000_000_000
}
fn receipt_matches(receipt:&Receipt,identity:ProcessIdentity,proxy:&str)->bool{
    valid_receipt(receipt)&&receipt.pid==identity.pid&&receipt.created_filetime==identity.created_filetime.to_string()&&receipt.proxy_url==proxy
}
#[derive(Default)]struct SelectedEnvironment{selected:BTreeMap<String,String>,name:Vec<u16>,value:Vec<u16>,key:Option<&'static str>,separator:bool,previous_zero:bool}
impl SelectedEnvironment{
    fn push(&mut self,unit:u16)->Result<bool,String>{
        if unit==0{
            if self.previous_zero{return Ok(true)}self.previous_zero=true;
            if let Some(key)=self.key.take(){let value=String::from_utf16(&self.value).map_err(|_|"目标代理变量无法读取。")?;if self.selected.insert(key.into(),value).is_some(){return Err("目标代理变量存在重复定义，无法确定当前值。".into())}}
            self.name.clear();self.value.clear();self.separator=false;return Ok(false)
        }
        self.previous_zero=false;
        if !self.separator{
            if unit==b'='as u16{self.separator=true;self.key=String::from_utf16(&self.name).ok().and_then(|name|KEYS.iter().copied().find(|key|name.eq_ignore_ascii_case(key)));self.name.clear();}
            else if self.name.len()<64{self.name.push(unit)}else{self.name.clear();self.separator=true;}
        }else if self.key.is_some(){if self.value.len()>=4096{return Err("目标代理变量长度超出检查范围。".into())}self.value.push(unit);}
        Ok(false)
    }
}
#[cfg(test)]fn selected_environment(units:&[u16])->Result<BTreeMap<String,String>,String>{let mut reader=SelectedEnvironment::default();for &unit in units{if reader.push(unit)?{return Ok(reader.selected)}}Err("环境块未完整终止。".into())}
fn environment_matches(selected:&BTreeMap<String,String>,proxy:&str)->bool{core::proxy_pairs(proxy).iter().all(|(key,value)|selected.get(*key)==Some(value))}
pub fn state(desktop:Option<&native::DesktopInfo>,proxy:&str,guard_ready:bool)->ProxyState{
    if !guard_ready{return ProxyState::new("inactive",None)}let Some(desktop)=desktop else{return ProxyState::new("not_running",None)};
    let identity=ProcessIdentity{pid:desktop.pid,created_filetime:desktop.created_filetime};
    if core::endpoint(proxy).is_err(){return ProxyState::new("unknown",Some("当前代理配置无法核验。".into()))}
    // Revalidate the exact live package/creation before trusting a cached receipt.
    if !unsafe{native::process_identity(identity.pid)}.is_some_and(|(actual,_,packaged)|actual==identity&&packaged){return ProxyState::new("unknown",Some("ChatGPT 实例正在变化，暂不能确认启动环境。".into()))}
    let mut cache=cache().lock().unwrap_or_else(|error|error.into_inner());
    if let Some((old_identity,old_proxy,at,view))=&cache.last{if *old_identity==identity&&old_proxy==proxy&&at.elapsed()<Duration::from_secs(10){return view.clone()}}
    if cache.receipt.as_ref().is_some_and(|receipt|receipt_matches(receipt,identity,proxy)){
        let view=ProxyState::new("applied",None);cache.last=Some((identity,proxy.into(),Instant::now(),view.clone()));return view
    }
    let view=match read_selected(identity){
        Ok(selected)if environment_matches(&selected,proxy)=>{
            // This stores verified identity and the already validated, credential-free
            // local proxy setting. No raw environment variable block is retained.
            let receipt=Receipt{schema:1,pid:identity.pid,created_filetime:identity.created_filetime.to_string(),package_family:core::FAMILY.into(),proxy_url:proxy.into(),verified_at:settings::now()};
            if cache.writable{let _=settings::write_json(&settings::root().join(RECEIPT),&receipt);}cache.receipt=Some(receipt);ProxyState::new("applied",None)
        },
        Ok(_)=>ProxyState::new("restart_required",Some("当前 ChatGPT 的启动环境与当前代理设置不同；保存工作后重开才能应用。".into())),
        Err(_)=>ProxyState::new("unknown",Some("暂不能核验当前 ChatGPT 的启动环境。守护仍作用于后续启动；可在保存工作后重开以应用当前设置。".into())),
    };
    cache.last=Some((identity,proxy.into(),Instant::now(),view.clone()));view
}

type Handle=*mut c_void;
#[repr(C)]struct BasicInformation{exit_status:i32,_padding:i32,peb:usize,affinity:usize,priority:i32,_padding2:i32,pid:usize,parent:usize}
#[repr(C)]struct Region{base:usize,allocation_base:usize,allocation_protect:u32,partition:u16,_padding:u16,size:usize,state:u32,protect:u32,kind:u32,_padding2:u32}
#[link(name="kernel32")]extern "system"{
    fn OpenProcess(access:u32,inherit:i32,pid:u32)->Handle;
    fn CloseHandle(process:Handle)->i32;
    fn GetModuleHandleW(name:*const u16)->Handle;
    fn GetProcAddress(module:Handle,name:*const u8)->Handle;
    fn ReadProcessMemory(process:Handle,address:*const c_void,buffer:*mut c_void,size:usize,read:*mut usize)->i32;
    fn VirtualQueryEx(process:Handle,address:*const c_void,region:*mut Region,size:usize)->usize;
    fn GetProcessTimes(process:Handle,created:*mut u64,exited:*mut u64,kernel:*mut u64,user:*mut u64)->i32;
    fn WaitForSingleObject(process:Handle,timeout:u32)->u32;
}
struct Process(Handle);
impl Drop for Process{fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
fn read_exact(process:Handle,address:usize,bytes:&mut[u8])->Result<(),String>{unsafe{
    if address==0||address.checked_add(bytes.len()).is_none(){return Err("目标内存地址无效。".into())}let mut read=0;
    if ReadProcessMemory(process,address as *const c_void,bytes.as_mut_ptr().cast(),bytes.len(),&mut read)==0||read!=bytes.len(){return Err("目标启动环境不可读取。".into())}Ok(())
}}
fn pointer(process:Handle,address:usize)->Result<usize,String>{let mut bytes=[0u8;8];read_exact(process,address,&mut bytes)?;Ok(u64::from_le_bytes(bytes)as usize)}
fn environment(process:Handle,address:usize)->Result<BTreeMap<String,String>,String>{unsafe{
    let mut reader=SelectedEnvironment::default();let mut offset=0usize;
    while offset<256*1024{
        let cursor=address.checked_add(offset).ok_or("目标环境地址无效。")?;let mut region:Region=std::mem::zeroed();
        if VirtualQueryEx(process,cursor as *const c_void,&mut region,size_of::<Region>())!=size_of::<Region>()||region.state!=0x1000||region.protect&0x101!=0{return Err("目标环境区域不可读取。".into())}
        let end=region.base.checked_add(region.size).filter(|&end|end>cursor).ok_or("目标环境区域无效。")?;
        let count=(end-cursor).min(1024).min(256*1024-offset)&!1;if count==0{return Err("目标环境区域长度无效。".into())}
        let mut bytes=vec![0u8;count];read_exact(process,cursor,&mut bytes)?;
        for at in (0..bytes.len()).step_by(2){let unit=u16::from_le_bytes([bytes[at],bytes[at+1]]);match reader.push(unit){Ok(true)=>{bytes.fill(0);return Ok(reader.selected)},Ok(false)=>{},Err(error)=>{bytes.fill(0);return Err(error)}}}bytes.fill(0);offset+=count;
    }Err("目标环境超过安全读取上限。".into())
}}
fn read_selected(identity:ProcessIdentity)->Result<BTreeMap<String,String>,String>{unsafe{
    if size_of::<usize>()!=8{return Err("当前进程位数不支持启动环境核验。".into())}
    let raw=OpenProcess(0x400|0x10|0x100000,0,identity.pid);if raw.is_null(){return Err("没有读取目标启动环境的权限。".into())}let process=Process(raw);
    let valid=||{let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);GetProcessTimes(process.0,&mut created,&mut exited,&mut kernel,&mut user)!=0&&created==identity.created_filetime&&WaitForSingleObject(process.0,0)==258};
    if !valid(){return Err("目标进程身份已变化。".into())}
    let kernel=GetModuleHandleW(native::wide("kernel32.dll").as_ptr());let machine=GetProcAddress(kernel,b"IsWow64Process2\0".as_ptr());
    if machine.is_null(){return Err("当前 Windows 不支持安全核验目标位数。".into())}
    let machine:unsafe extern "system" fn(Handle,*mut u16,*mut u16)->i32=std::mem::transmute(machine);let(mut process_machine,mut native_machine)=(0,0);
    if machine(process.0,&mut process_machine,&mut native_machine)==0||process_machine!=0||native_machine!=0x8664{return Err("目标不是受支持的原生 x64 进程。".into())}
    let library=GetModuleHandleW(native::wide("ntdll.dll").as_ptr());let query=GetProcAddress(library,b"NtQueryInformationProcess\0".as_ptr());if query.is_null(){return Err("当前 Windows 无法核验目标启动环境。".into())}
    let query:unsafe extern "system" fn(Handle,u32,*mut c_void,u32,*mut u32)->i32=std::mem::transmute(query);let mut basic:BasicInformation=std::mem::zeroed();let mut length=0;
    if query(process.0,0,(&mut basic as *mut BasicInformation).cast(),size_of::<BasicInformation>()as u32,&mut length)<0||length as usize!=size_of::<BasicInformation>()||basic.pid!=identity.pid as usize||basic.peb==0{return Err("目标启动信息布局不可验证。".into())}
    // The x64 PEB/RTL_USER_PROCESS_PARAMETERS environment pointer is intentionally
    // guarded by architecture, pointer, bounded-read, stability and identity checks.
    // Unsupported future layouts produce unknown; they never imply a missing proxy.
    let parameters=pointer(process.0,basic.peb.checked_add(0x20).ok_or("目标参数地址无效。")?)?;
    // Validate the documented ImagePathName field of the same parameter block.
    // CommandLine is deliberately not read. A changed ABI is unknown, not mismatch.
    let mut image_header=[0u8;16];read_exact(process.0,parameters.checked_add(0x60).ok_or("目标映像地址无效。")?,&mut image_header)?;
    let image_length=u16::from_le_bytes(image_header[..2].try_into().unwrap())as usize;let image_maximum=u16::from_le_bytes(image_header[2..4].try_into().unwrap())as usize;
    if image_length==0||image_length%2!=0||image_length>image_maximum{return Err("目标启动参数布局无法验证。".into())}
    let mut image_bytes=vec![0u8;image_length];read_exact(process.0,u64::from_le_bytes(image_header[8..16].try_into().unwrap())as usize,&mut image_bytes)?;
    let image_units=image_bytes.chunks_exact(2).map(|pair|u16::from_le_bytes([pair[0],pair[1]])).collect::<Vec<_>>();let image=String::from_utf16(&image_units).map_err(|_|"目标映像路径不可验证。")?;
    let actual=unsafe{native::process_identity(identity.pid)}.filter(|(actual,_,_)|*actual==identity).ok_or("目标进程身份已变化。")?;
    if settings::normalized_executable(Path::new(&image))!=actual.1{return Err("目标启动参数布局与进程映像不匹配。".into())}
    let environment_pointer=parameters.checked_add(0x80).ok_or("目标环境地址无效。")?;let address=pointer(process.0,environment_pointer)?;
    let first=environment(process.0,address)?;let second=environment(process.0,address)?;
    if pointer(process.0,basic.peb+0x20)?!=parameters||pointer(process.0,environment_pointer)?!=address||first!=second||!valid(){return Err("目标启动环境正在变化。".into())}Ok(first)
}}

#[cfg(test)]mod tests{
    use super::*;
    fn receipt()->Receipt{Receipt{schema:1,pid:42,created_filetime:"133786986147085981".into(),package_family:core::FAMILY.into(),proxy_url:core::DEFAULT_PROXY.into(),verified_at:100}}
    #[test]fn restart_and_resume_reuse_only_the_same_verified_process_and_proxy(){
        let record=receipt();let identity=ProcessIdentity{pid:42,created_filetime:133786986147085981};
        // Helper start times and guard pause/resume times are deliberately absent.
        assert!(receipt_matches(&record,identity,core::DEFAULT_PROXY));assert!(receipt_matches(&record,identity,core::DEFAULT_PROXY));
        assert!(!receipt_matches(&record,identity,"http://127.0.0.1:7891"));assert!(!receipt_matches(&record,ProcessIdentity{pid:42,created_filetime:133786986147085982},core::DEFAULT_PROXY));assert!(!receipt_matches(&record,ProcessIdentity{pid:43,..identity},core::DEFAULT_PROXY));
        let json=serde_json::to_string(&record).unwrap();for key in KEYS{assert!(!json.contains(key));}
    }
    #[test]fn selected_keys_detect_first_setup_and_config_change_without_retaining_other_values(){
        let mut block=core::environment_block(core::DEFAULT_PROXY);block.pop();block.extend("UNRELATED_SECRET=never-retain-this\0\0".encode_utf16());
        let selected=selected_environment(&block).unwrap();assert_eq!(selected.len(),5);assert!(environment_matches(&selected,core::DEFAULT_PROXY));assert!(!environment_matches(&selected,"http://127.0.0.1:7891"));assert!(!environment_matches(&BTreeMap::new(),core::DEFAULT_PROXY));assert!(!format!("{selected:?}").contains("never-retain-this"));
        block.pop();block.extend("http_proxy=http://127.0.0.1:9999\0\0".encode_utf16());assert!(selected_environment(&block).is_err());
    }
    #[test]fn bounded_native_environment_reader_observes_only_its_own_test_process(){
        let identity=unsafe{native::process_identity(std::process::id())}.unwrap().0;
        let selected=read_selected(identity).expect("The isolated x64 test process environment must be readable.");
        assert!(KEYS.iter().all(|key|selected.get(*key).cloned()==std::env::var(key).ok()),"Selected proxy values must match without printing their contents.");
    }
}
