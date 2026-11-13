use std::{ffi::c_void, path::PathBuf, ptr};
use crate::core::{FAMILY, ResumeRequest};
type Handle = *mut c_void;
pub fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }
fn string(value: &[u16]) -> String { String::from_utf16_lossy(&value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())]) }
fn os_error() -> String { std::io::Error::last_os_error().to_string() }

#[repr(C)] struct Guid { a: u32, b: u16, c: u16, d: [u8;8] }
#[repr(C)] struct Interface { vtable: *const Vtable }
#[repr(C)] struct Vtable {
    query: usize, add: usize,
    release: unsafe extern "system" fn(*mut Interface) -> u32,
    enable: unsafe extern "system" fn(*mut Interface, *const u16, *const u16, *const u16) -> i32,
    disable: unsafe extern "system" fn(*mut Interface, *const u16) -> i32,
}
#[link(name="ole32")] extern "system" {
    fn CoInitializeEx(reserved: Handle, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(class: *const Guid, outer: Handle, context: u32, interface: *const Guid, result: *mut *mut Interface) -> i32;
}
#[link(name="kernel32")] extern "system" {
    fn GetPackagesByPackageFamily(family: *const u16, count: *mut u32, names: *mut *mut u16, length: *mut u32, buffer: *mut u16) -> i32;
    fn GetPackagePathByFullName(name: *const u16, length: *mut u32, path: *mut u16) -> i32;
    fn GetPackageFamilyName(process: Handle, length: *mut u32, family: *mut u16) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> Handle;
    fn GetProcessIdOfThread(thread: Handle) -> u32;
    fn ResumeThread(thread: Handle) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn GetProcessTimes(process: Handle, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    fn QueryFullProcessImageNameW(process:Handle,flags:u32,name:*mut u16,length:*mut u32)->i32;
    fn GetCurrentProcess() -> Handle;
    fn GetSystemTimeAsFileTime(time:*mut u64);
    fn GetModuleHandleW(name:*const u16) -> Handle;
}
#[link(name="user32")] extern "system" {
    fn GetDpiForSystem()->u32; fn GetDpiForWindow(window:Handle)->u32;
    fn GetSystemMetricsForDpi(index:i32,dpi:u32)->i32;
    fn LoadImageW(instance:Handle,name:*const u16,kind:u32,width:i32,height:i32,flags:u32)->Handle;
    fn SendMessageW(window:Handle,message:u32,wparam:usize,lparam:isize)->isize;
}
#[repr(C)] struct MemoryCounters { size:u32,faults:u32,peak:usize,working:usize,peak_paged:usize,paged:usize,peak_nonpaged:usize,nonpaged:usize,pagefile:usize,peak_pagefile:usize }
#[link(name="psapi")] extern "system" { fn GetProcessMemoryInfo(process:Handle,counters:*mut MemoryCounters,size:u32)->i32; }
#[link(name="iphlpapi")] extern "system" { fn GetExtendedTcpTable(table:Handle,length:*mut u32,order:i32,family:u32,kind:u32,reserved:u32)->u32; }
#[link(name="shell32")] extern "system" { fn ShellExecuteW(window: Handle, operation: *const u16, file: *const u16, parameters: *const u16, directory: *const u16, show: i32) -> isize; }
#[repr(C)] struct ProcessEntry { size:u32, usage:u32, pid:u32, heap:usize, module:u32, threads:u32, parent:u32, priority:i32, flags:u32, exe:[u16;260] }

pub struct DebugSettings { pointer: *mut Interface }
impl DebugSettings {
    pub fn new() -> Result<Self, String> { unsafe {
        let hr = CoInitializeEx(ptr::null_mut(), 2); if hr < 0 { return Err(format!("COM 初始化失败：0x{:08X}",hr)); }
        let class=Guid{a:0xB1AEC16F,b:0x2383,c:0x4852,d:[0xB0,0xE9,0x8F,0x0B,0x1D,0xC6,0x6B,0x4D]};
        let iid=Guid{a:0xF27C3930,b:0x8029,c:0x4AD1,d:[0x94,0xE3,0x3D,0xBA,0x41,0x78,0x10,0xC1]};
        let mut pointer=ptr::null_mut(); let hr=CoCreateInstance(&class,ptr::null_mut(),23,&iid,&mut pointer);
        if hr<0 { CoUninitialize(); return Err(format!("Windows 应用接口不可用：0x{:08X}",hr)); }
        Ok(Self{pointer})
    } }
    pub fn enable(&self, package:&str, executable:&str, block:&[u16]) -> Result<(),String> { unsafe {
        let package=wide(package);let debugger=wide(&format!("\"{executable}\""));
        let hr=((*(*self.pointer).vtable).enable)(self.pointer,package.as_ptr(),debugger.as_ptr(),block.as_ptr());
        if hr<0 { Err(format!("无法开启桌面端守护：0x{:08X}",hr)) } else { Ok(()) }
    } }
    pub fn disable(&self, package:&str) -> Result<(),String> { unsafe { let p=wide(package);let hr=((*(*self.pointer).vtable).disable)(self.pointer,p.as_ptr());if hr<0 {Err(format!("无法释放应用设置：0x{:08X}",hr))}else{Ok(())} } }
}
impl Drop for DebugSettings { fn drop(&mut self) { unsafe { ((*(*self.pointer).vtable).release)(self.pointer); CoUninitialize(); } } }

#[derive(Clone,Debug)] pub struct Package { pub full_name:String, pub version:String }
pub fn packages() -> Result<Vec<Package>,String> { unsafe {
    let family=wide(FAMILY);let(mut count,mut length)=(0,0);
    let result=GetPackagesByPackageFamily(family.as_ptr(),&mut count,ptr::null_mut(),&mut length,ptr::null_mut());
    if result==0 && count==0 {return Ok(vec![])}
    if result!=122 {return Err(format!("无法读取 ChatGPT 安装信息：{result}"))}
    let mut names=vec![ptr::null_mut();count as usize];let mut buffer=vec![0u16;length as usize];
    let result=GetPackagesByPackageFamily(family.as_ptr(),&mut count,names.as_mut_ptr(),&mut length,buffer.as_mut_ptr());
    if result!=0 {return Err(format!("读取应用包失败：{result}"))}
    let mut packages=vec![];
    for name in names.into_iter().take(count as usize) {
        if name.is_null(){continue}let offset=name.offset_from(buffer.as_ptr()) as usize;let name=string(&buffer[offset..]);
        let mut path_len=0;let w=wide(&name);if GetPackagePathByFullName(w.as_ptr(),&mut path_len,ptr::null_mut())!=122 {continue}
        let mut path=vec![0u16;path_len as usize];if GetPackagePathByFullName(w.as_ptr(),&mut path_len,path.as_mut_ptr())!=0 {continue}
        if PathBuf::from(string(&path)).join("app").join("ChatGPT.exe").is_file() {packages.push(Package{version:name.split('_').nth(1).unwrap_or("").into(),full_name:name});}
    }
    Ok(packages)
} }

fn family_matches(process:Handle) -> bool {unsafe{let mut buffer=[0u16;256];let mut len=buffer.len() as u32;GetPackageFamilyName(process,&mut len,buffer.as_mut_ptr())==0&&string(&buffer)==FAMILY}}
pub fn resume(request:ResumeRequest) -> Result<(),String> { unsafe {
    let thread=OpenThread(0x0802,0,request.tid);if thread.is_null(){return Err(os_error())}
    let result=(||{if GetProcessIdOfThread(thread)!=request.pid{return Err("启动线程与进程不匹配".into())}let process=OpenProcess(0x1000,0,request.pid);if process.is_null(){return Err(os_error())}let allowed=family_matches(process);CloseHandle(process);if !allowed{return Err("目标不属于 ChatGPT 应用包".into())}if ResumeThread(thread)==u32::MAX{return Err(os_error())}Ok(())})();CloseHandle(thread);result
} }

pub struct DesktopInfo {pub pid:u32,pub started_at:u64,pub created_filetime:u64,pub pids:Vec<u32>}
pub fn desktop_info() -> Option<DesktopInfo> { unsafe {
    let snapshot=CreateToolhelp32Snapshot(2,0);if snapshot as isize == -1{return None}
    let mut entry:ProcessEntry=std::mem::zeroed();entry.size=std::mem::size_of::<ProcessEntry>() as u32;let mut rows=vec![];
    let mut more=Process32FirstW(snapshot,&mut entry);while more!=0 {rows.push((entry.pid,entry.parent,string(&entry.exe).eq_ignore_ascii_case("ChatGPT.exe")));more=Process32NextW(snapshot,&mut entry);}CloseHandle(snapshot);
    let mut roots=vec![];
    for &(pid,parent,is_chatgpt) in &rows {if !is_chatgpt||rows.iter().any(|&(p,_,chatgpt)|p==parent&&chatgpt){continue}let process=OpenProcess(0x1000,0,pid);if process.is_null(){continue}let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);let good=family_matches(process)&&GetProcessTimes(process,&mut created,&mut exited,&mut kernel,&mut user)!=0;CloseHandle(process);if good {roots.push((created,pid));}}
    roots.sort_unstable();let &(created_filetime,pid)=roots.first()?;let ids=roots.iter().map(|r|r.1).collect::<Vec<_>>();
    Some(DesktopInfo{pid,started_at:(created_filetime/10000).saturating_sub(11644473600000),created_filetime,pids:crate::core::descendant_pids(&rows.iter().map(|r|(r.0,r.1)).collect::<Vec<_>>(),&ids)})
} }
/// Attribution is based on live process identity and exact installed paths. A
/// similarly named codex.exe/node.exe is never enough to join the target set.
// Open handles keep the exact process objects/IDs alive through the IPC handoff.
// https://devblogs.microsoft.com/oldnewthing/20110107-00/?p=11803
pub(crate) struct ProcessLease{pub identity:crate::traffic_core::ProcessIdentity,handle:Handle}
unsafe impl Send for ProcessLease{} unsafe impl Sync for ProcessLease{}
impl ProcessLease{
    fn allows_child(&self,child_created:u64)->bool{unsafe{let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);GetProcessTimes(self.handle,&mut created,&mut exited,&mut kernel,&mut user)!=0&&created==self.identity.created_filetime&&child_created>=created&&(exited==0||child_created<=exited)}}
    pub(crate) fn belongs_to(&self,owner:&str)->bool{crate::traffic::process_sid(windows::Win32::Foundation::HANDLE(self.handle)).is_ok_and(|sid|sid==owner)}
}
impl Drop for ProcessLease{fn drop(&mut self){unsafe{CloseHandle(self.handle);}}}
pub fn traffic_targets(root:&std::path::Path,config:&crate::settings::Config)->Result<Vec<crate::traffic_core::ProcessIdentity>,String>{traffic_targets_held(root,config).map(|leases|leases.into_iter().map(|lease|lease.identity).collect())}
pub(crate) fn traffic_targets_held(root:&std::path::Path,config:&crate::settings::Config)->Result<Vec<ProcessLease>,String>{traffic_targets_with_anchors(root,config,&[])}
pub(crate) fn traffic_targets_with_anchors(root:&std::path::Path,config:&crate::settings::Config,anchors:&[&ProcessLease])->Result<Vec<ProcessLease>,String>{unsafe{
    let mut snapshot_started=0;GetSystemTimeAsFileTime(&mut snapshot_started);
    let snapshot=CreateToolhelp32Snapshot(2,0);if snapshot as isize == -1{return Err(os_error())}
    let mut entry:ProcessEntry=std::mem::zeroed();entry.size=std::mem::size_of::<ProcessEntry>()as u32;let mut rows=vec![];
    let mut more=Process32FirstW(snapshot,&mut entry);while more!=0{rows.push((entry.pid,entry.parent,string(&entry.exe)));more=Process32NextW(snapshot,&mut entry);}CloseHandle(snapshot);
    let allowed=crate::settings::cli_process_paths(root,config);let mut identified=std::collections::BTreeMap::new();let mut selected=std::collections::BTreeSet::new();let mut leases=std::collections::BTreeMap::new();
    let held=anchors.iter().map(|lease|(lease.identity.pid,*lease)).collect::<std::collections::BTreeMap<_,_>>();
    for lease in anchors{identified.insert(lease.identity.pid,lease.identity);selected.insert(lease.identity.pid);}
    for &(pid,_,ref name) in &rows{
        if !name.eq_ignore_ascii_case("ChatGPT.exe")&&!name.eq_ignore_ascii_case("codex.exe"){continue}
        if let Some((identity,image,desktop,lease))=process_identity_with_lease(pid){if desktop||allowed.iter().any(|path|path==&image){selected.insert(pid);}identified.insert(pid,identity);leases.insert(pid,lease);}
    }
    // Anchored parent identities cannot be reused; child creation must fall within
    // the real parent lifetime, including a parent that exited during discovery.
    loop{let mut added=false;for &(pid,parent,_)in &rows{
        if selected.contains(&pid)||!selected.contains(&parent){continue}
        let Some(parent_identity)=identified.get(&parent)else{continue};let parent_created=parent_identity.created_filetime;
        if let Some((identity,_,_,lease))=process_identity_with_lease(pid){if identity.created_filetime>=parent_created&&identity.created_filetime<=snapshot_started&&leases.get(&parent).or_else(||held.get(&parent).copied()).is_some_and(|lease|lease.allows_child(identity.created_filetime)){identified.insert(pid,identity);leases.insert(pid,lease);selected.insert(pid);added=true;}}
    }if !added{break}}
    let targets=selected.into_iter().filter_map(|pid|leases.remove(&pid)).collect::<Vec<_>>();crate::traffic_core::validate_targets(&targets.iter().map(|lease|lease.identity).collect::<Vec<_>>())?;Ok(targets)
}}
pub(crate) unsafe fn process_identity(pid:u32)->Option<(crate::traffic_core::ProcessIdentity,String,bool)>{process_identity_with_lease(pid).map(|(identity,image,desktop,_)|(identity,image,desktop))}
pub(crate) unsafe fn process_identity_with_lease(pid:u32)->Option<(crate::traffic_core::ProcessIdentity,String,bool,ProcessLease)>{
    let process=OpenProcess(0x101000,0,pid);if process.is_null(){return None}
    let mut path=vec![0u16;32768];let mut length=path.len()as u32;let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);
    let valid=QueryFullProcessImageNameW(process,0,path.as_mut_ptr(),&mut length)!=0&&GetProcessTimes(process,&mut created,&mut exited,&mut kernel,&mut user)!=0&&created!=0&&exited==0;
    let desktop=valid&&family_matches(process);if !valid{CloseHandle(process);return None}
    path.truncate(length as usize);let image=crate::settings::normalized_executable(&PathBuf::from(String::from_utf16_lossy(&path)));
    let identity=crate::traffic_core::ProcessIdentity{pid,created_filetime:created};Some((identity,image,desktop,ProcessLease{identity,handle:process}))
}

pub fn memory_bytes()->Option<u64>{unsafe{let mut counters:MemoryCounters=std::mem::zeroed();let size=std::mem::size_of::<MemoryCounters>()as u32;counters.size=size;if GetProcessMemoryInfo(GetCurrentProcess(),&mut counters,size)!=0{Some(counters.working as u64)}else{None}}}
pub fn tcp_connections(pids:&[u32])->Option<usize>{unsafe{
    if pids.is_empty(){return Some(0)}let mut total=0;
    for (family,size,state,pid) in [(2,24,0,20),(23,56,48,52)] {let mut length=0;let code=GetExtendedTcpTable(ptr::null_mut(),&mut length,0,family,5,0);if code!=122||length>16*1024*1024{return None}let mut bytes=vec![0u8;length as usize];if GetExtendedTcpTable(bytes.as_mut_ptr().cast(),&mut length,0,family,5,0)!=0{return None}bytes.truncate(length as usize);total+=crate::core::count_tcp_rows(&bytes,size,state,pid,pids)?;}Some(total)
}}
pub fn small_icon_size()->u32{unsafe{GetSystemMetricsForDpi(49,GetDpiForSystem()).max(16)as u32}}
pub fn set_window_icons(window:Handle){unsafe{let dpi=GetDpiForWindow(window);for (kind,metric) in [(0,49),(1,11)]{let size=GetSystemMetricsForDpi(metric,dpi);let icon=LoadImageW(GetModuleHandleW(ptr::null()),32512usize as *const u16,1,size,size,0x8000);if !icon.is_null(){SendMessageW(window,0x0080,kind,icon as isize);}}}}

pub fn open_desktop() -> Result<(),String> { crate::desktop_lifecycle::launch_desktop() }
pub(crate) fn activate_desktop() -> Result<(),String> { open(&format!("shell:AppsFolder\\{FAMILY}!App")) }
pub fn open(target:&str) -> Result<(),String> { unsafe {let verb=wide("open");let file=wide(target);let result=ShellExecuteW(ptr::null_mut(),verb.as_ptr(),file.as_ptr(),ptr::null(),ptr::null(),1);if result<=32{Err(format!("无法打开目标：Windows 错误 {result}"))}else{Ok(())}} }
