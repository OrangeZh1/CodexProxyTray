//! Cooperative desktop lifecycle only. No process termination or global close broadcast.
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,ffi::c_void,ptr,sync::{Mutex,atomic::{AtomicBool,AtomicU8,Ordering}},thread,time::{Duration,Instant}};
use crate::{core::FAMILY,native};

type Handle=*mut c_void;
const WAIT_OBJECT_0:u32=0;
const WAIT_TIMEOUT:u32=258;
const MAX_TREE:usize=256;
const CLOSE_TIMEOUT:Duration=Duration::from_secs(15);
static LOCK:Mutex<()>=Mutex::new(());
static BUSY:AtomicBool=AtomicBool::new(false);
static STAGE:AtomicU8=AtomicU8::new(0);

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub pid:u32,
    // FILETIME is larger than JavaScript's safe integer range. Never round-trip as f64.
    pub created_filetime:String,
    pub package_family:String,
}
#[derive(Clone,Serialize)]pub struct OperationState {pub busy:bool,pub stage:&'static str}
#[derive(Clone,Serialize)]pub struct Outcome {pub pid:u32,pub restart_requested:bool}
pub fn in_progress()->bool {BUSY.load(Ordering::Acquire)}
pub fn snapshot()->OperationState {OperationState{busy:in_progress(),stage:match STAGE.load(Ordering::Acquire){1=>"verifying",2=>"closing",3=>"waiting",4=>"restarting",_=>"idle"}}}
struct BusyGuard;
impl Drop for BusyGuard {fn drop(&mut self){STAGE.store(0,Ordering::Release);BUSY.store(false,Ordering::Release);}}

#[link(name="kernel32")]extern "system" {
    fn OpenProcess(access:u32,inherit:i32,pid:u32)->Handle;
    fn CloseHandle(handle:Handle)->i32;
    fn GetProcessTimes(process:Handle,created:*mut u64,exited:*mut u64,kernel:*mut u64,user:*mut u64)->i32;
    fn GetPackageFamilyName(process:Handle,length:*mut u32,family:*mut u16)->i32;
    fn QueryFullProcessImageNameW(process:Handle,flags:u32,name:*mut u16,length:*mut u32)->i32;
    fn WaitForSingleObject(handle:Handle,timeout:u32)->u32;
    fn CreateToolhelp32Snapshot(flags:u32,pid:u32)->Handle;
    fn Process32FirstW(snapshot:Handle,entry:*mut ProcessEntry)->i32;
    fn Process32NextW(snapshot:Handle,entry:*mut ProcessEntry)->i32;
    fn GetLastError()->u32;
}
#[link(name="user32")]extern "system" {
    fn EnumWindows(callback:unsafe extern "system" fn(Handle,isize)->i32,data:isize)->i32;
    fn GetWindowThreadProcessId(window:Handle,pid:*mut u32)->u32;
    fn IsWindowVisible(window:Handle)->i32;
    fn GetWindow(window:Handle,command:u32)->Handle;
    fn SendMessageTimeoutW(window:Handle,message:u32,wparam:usize,lparam:isize,flags:u32,timeout:u32,result:*mut usize)->isize;
}
#[repr(C)]struct ProcessEntry{size:u32,usage:u32,pid:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,exe:[u16;260]}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle{fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
struct Process{handle:OwnedHandle,pid:u32,created:u64}
struct Row{pid:u32,parent:u32,chatgpt:bool}
fn failure(context:&str)->String {format!("{context}（Windows 错误 {}）。",unsafe{GetLastError()})}
fn rows()->Result<Vec<Row>,String>{unsafe{
    let handle=CreateToolhelp32Snapshot(2,0);if handle as isize == -1{return Err(failure("无法读取进程列表，未操作 ChatGPT"))}let snapshot=OwnedHandle(handle);
    let mut entry:ProcessEntry=std::mem::zeroed();entry.size=std::mem::size_of::<ProcessEntry>()as u32;
    if Process32FirstW(snapshot.0,&mut entry)==0{return Err(failure("无法读取进程列表，未操作 ChatGPT"))}
    let mut result=vec![];
    loop{let end=entry.exe.iter().position(|&c|c==0).unwrap_or(entry.exe.len());result.push(Row{pid:entry.pid,parent:entry.parent,chatgpt:String::from_utf16_lossy(&entry.exe[..end]).eq_ignore_ascii_case("ChatGPT.exe")});if Process32NextW(snapshot.0,&mut entry)==0{break}}
    if GetLastError()!=18{return Err(failure("进程列表读取不完整，未操作 ChatGPT"))}Ok(result)
}}
impl Process{
    fn open(pid:u32)->Result<Option<Self>,String>{unsafe{
        let raw=OpenProcess(0x1000|0x100000,0,pid);if raw.is_null(){let code=GetLastError();return if code==87{Ok(None)}else{Err(format!("无法验证目标进程（Windows 错误 {code}），未关闭应用。"))}}
        let handle=OwnedHandle(raw);let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);
        if GetProcessTimes(handle.0,&mut created,&mut exited,&mut kernel,&mut user)==0{return Err(failure("无法验证进程创建时间"))}
        if created==0{return Err("目标进程创建时间无效，未关闭应用。".into())}
        Ok(Some(Self{handle,pid,created}))
    }}
    fn alive(&self)->Result<bool,String>{match unsafe{WaitForSingleObject(self.handle.0,0)}{WAIT_TIMEOUT=>Ok(true),WAIT_OBJECT_0=>Ok(false),_=>Err(failure("无法确认目标进程是否退出"))}}
    fn family(&self)->Result<bool,String>{unsafe{
        let mut buffer=[0u16;256];let mut length=buffer.len()as u32;let result=GetPackageFamilyName(self.handle.0,&mut length,buffer.as_mut_ptr());
        if result==15700{return Ok(false)}if result!=0{return Err(format!("无法验证应用包身份（Windows 错误 {result}），未关闭应用。"))}
        let end=buffer.iter().position(|&c|c==0).unwrap_or(buffer.len());Ok(String::from_utf16_lossy(&buffer[..end])==FAMILY)
    }}
    fn desktop_image(&self)->Result<bool,String>{unsafe{
        let mut buffer=vec![0u16;32768];let mut length=buffer.len()as u32;
        if QueryFullProcessImageNameW(self.handle.0,0,buffer.as_mut_ptr(),&mut length)==0{return Err(failure("无法验证目标程序路径"))}
        buffer.truncate(length as usize);let image=String::from_utf16_lossy(&buffer);
        Ok(std::path::Path::new(&image).file_name().is_some_and(|name|name.to_string_lossy().eq_ignore_ascii_case("ChatGPT.exe")))
    }}
    fn last_creation(&self)->Result<u64,String>{unsafe{
        if self.alive()?{return Ok(u64::MAX)}let(mut created,mut exited,mut kernel,mut user)=(0,0,0,0);
        if GetProcessTimes(self.handle.0,&mut created,&mut exited,&mut kernel,&mut user)==0||created!=self.created||exited==0{return Err("无法验证目标进程的退出时间，未重开应用。".into())}Ok(exited)
    }}
    fn target(&self)->Target{Target{pid:self.pid,created_filetime:self.created.to_string(),package_family:FAMILY.into()}}
}
fn verified_roots(snapshot:&[Row])->Result<Vec<Process>,String>{
    let mut processes=BTreeMap::new();
    for row in snapshot.iter().filter(|row|row.chatgpt){if let Some(process)=Process::open(row.pid)?{if process.alive()?&&process.family()?&&process.desktop_image()?{processes.insert(row.pid,process);}}}
    let ids=processes.keys().copied().filter(|pid|{let row=snapshot.iter().find(|row|row.pid==*pid).unwrap();processes.get(&row.parent).is_none_or(|parent|parent.created>processes[pid].created)}).collect::<Vec<_>>();
    Ok(ids.into_iter().filter_map(|pid|processes.remove(&pid)).collect())
}
fn one_root(snapshot:&[Row])->Result<Process,String>{
    let mut roots=verified_roots(snapshot)?;
    match roots.len(){0=>Err("未找到可验证的 ChatGPT 主进程，请重新查看应用状态。".into()),1=>Ok(roots.remove(0)),_=>Err("检测到多个 ChatGPT 主进程，请先在应用内检查各个实例；助手未关闭任何实例。".into())}
}
fn expected_creation(target:&Target)->Result<u64,String>{
    if target.pid==0||target.package_family!=FAMILY||target.created_filetime.is_empty()||!target.created_filetime.bytes().all(|c|c.is_ascii_digit()){return Err("目标应用身份无效，未关闭应用。".into())}
    target.created_filetime.parse::<u64>().ok().filter(|&value|value!=0).ok_or_else(||"目标进程创建时间无效，未关闭应用。".into())
}
pub fn capture()->Result<Target,String>{
    if in_progress(){return Err("ChatGPT 关闭或重启操作正在进行，请等待完成。".into())}
    Ok(one_root(&rows()?)?.target())
}
// All normal launch paths share this lock, including startup preferences and tray actions.
pub fn launch_desktop()->Result<(),String>{
    let _lock=LOCK.try_lock().map_err(|_|"ChatGPT 关闭或重启操作正在进行，未重复打开应用。".to_owned())?;
    native::activate_desktop()
}
fn extend_tree(snapshot:&[Row],tree:&mut BTreeMap<u32,Process>)->Result<(),String>{
    loop{let mut added=false;
        for row in snapshot{if tree.contains_key(&row.pid){continue}let Some(parent)=tree.get(&row.parent)else{continue};
            let parent_created=parent.created;let parent_end=parent.last_creation()?;
            if let Some(process)=Process::open(row.pid)?{if process.created>=parent_created&&process.created<=parent_end{
                if tree.len()>=MAX_TREE{return Err("目标进程树超过安全检查上限，未重开应用。".into())}tree.insert(row.pid,process);added=true;
            }}
        }
        if !added{return Ok(())}
    }
}
struct Windows{pid:u32,handles:Vec<Handle>}
unsafe extern "system" fn collect_windows(window:Handle,data:isize)->i32{unsafe{
    let found=&mut *(data as *mut Windows);let mut pid=0;
    if GetWindowThreadProcessId(window,&mut pid)!=0&&pid==found.pid&&IsWindowVisible(window)!=0&&GetWindow(window,4).is_null(){found.handles.push(window);}1
}}
fn send_close(window:Handle,root:&Process,timeout:u32)->Result<(),String>{unsafe{
    let mut pid=0;if !root.alive()?||GetWindowThreadProcessId(window,&mut pid)==0||pid!=root.pid{return Err("目标窗口已变化，未继续关闭或重启。".into())}
    let mut result=0;
    // WM_CLOSE lets the application refuse, ask to save work, or merely hide its window.
    // SMTO_BLOCK | SMTO_ABORTIFHUNG bounds the send without terminating any process.
    if SendMessageTimeoutW(window,0x0010,0,0,3,timeout,&mut result)==0{return Err("ChatGPT 未响应正常关闭请求，未强制结束或重新打开。请保存工作后从 ChatGPT 菜单退出。".into())}Ok(())
}}
pub fn perform(target:Target,restart:bool)->Result<Outcome,String>{
    let _lock=LOCK.try_lock().map_err(|_|"另一项 ChatGPT 操作正在进行，请等待完成。".to_owned())?;
    BUSY.store(true,Ordering::Release);STAGE.store(1,Ordering::Release);let _busy=BusyGuard;
    let created=expected_creation(&target)?;let before=rows()?;let root=one_root(&before)?;
    if root.pid!=target.pid||root.created!=created{return Err("ChatGPT 进程已变化。请关闭此确认框，重新确认当前实例。".into())}
    let mut tree=BTreeMap::new();tree.insert(root.pid,root);extend_tree(&before,&mut tree)?;
    let mut windows=Windows{pid:target.pid,handles:vec![]};
    if unsafe{EnumWindows(collect_windows,&mut windows as *mut Windows as isize)}==0{return Err(failure("无法验证 ChatGPT 窗口，未关闭应用"))}
    if windows.handles.is_empty(){return Err("未找到可正常关闭的 ChatGPT 窗口。请打开 ChatGPT 并从其菜单退出；助手未强制结束或重开。".into())}
    if windows.handles.len()>8{return Err("目标窗口数量超出安全检查范围，未关闭应用。".into())}
    let deadline=Instant::now()+CLOSE_TIMEOUT;STAGE.store(2,Ordering::Release);
    for window in windows.handles{if !tree[&target.pid].alive()?{break}let remaining=deadline.saturating_duration_since(Instant::now()).as_millis().min(1200)as u32;if remaining==0{return Err("正常关闭等待超时，未重新打开应用。".into())}send_close(window,&tree[&target.pid],remaining)?;}
    STAGE.store(3,Ordering::Release);
    loop{
        extend_tree(&rows()?,&mut tree)?;
        let alive=tree.values().map(Process::alive).collect::<Result<Vec<_>,_>>()?.into_iter().any(|live|live);
        if !alive{break}if Instant::now()>=deadline{return Err("ChatGPT 仍未完整退出，可能已拒绝关闭或仅收起窗口。助手未强制结束或重复启动；请保存工作并从 ChatGPT 菜单退出后重试。".into())}thread::sleep(Duration::from_millis(100));
    }
    // A new independent instance must never be mistaken for completion of the confirmed one.
    if !verified_roots(&rows()?)?.is_empty(){return Err("检测到另一个 ChatGPT 实例，未重复打开应用。".into())}
    if restart{STAGE.store(4,Ordering::Release);native::activate_desktop()?;}
    Ok(Outcome{pid:target.pid,restart_requested:restart})
}

#[cfg(test)]mod tests{
    use super::*;
    #[repr(C)]struct WindowClass{style:u32,procedure:Option<unsafe extern "system" fn(Handle,u32,usize,isize)->isize>,class_extra:i32,window_extra:i32,instance:Handle,icon:Handle,cursor:Handle,background:Handle,menu:*const u16,name:*const u16}
    #[repr(C)]struct Message{window:Handle,message:u32,wparam:usize,lparam:isize,time:u32,point:[i32;2],private:u32}
    #[link(name="kernel32")]extern "system"{fn GetModuleHandleW(name:*const u16)->Handle;}
    #[link(name="user32")]extern "system"{
        fn RegisterClassW(class:*const WindowClass)->u16;
        fn UnregisterClassW(name:*const u16,instance:Handle)->i32;
        fn CreateWindowExW(extended:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,width:i32,height:i32,parent:Handle,menu:Handle,instance:Handle,data:Handle)->Handle;
        fn DefWindowProcW(window:Handle,message:u32,wparam:usize,lparam:isize)->isize;
        fn DestroyWindow(window:Handle)->i32;
        fn IsWindow(window:Handle)->i32;
        fn PostQuitMessage(code:i32);
        fn GetMessageW(message:*mut Message,window:Handle,minimum:u32,maximum:u32)->i32;
        fn TranslateMessage(message:*const Message)->i32;
        fn DispatchMessageW(message:*const Message)->isize;
        fn PostMessageW(window:Handle,message:u32,wparam:usize,lparam:isize)->i32;
    }
    static CLOSE_SEEN:AtomicBool=AtomicBool::new(false);
    unsafe extern "system" fn refusing_window(window:Handle,message:u32,wparam:usize,lparam:isize)->isize{unsafe{
        match message{0x10=>{CLOSE_SEEN.store(true,Ordering::Release);0},0x8001=>{DestroyWindow(window);0},2=>{PostQuitMessage(0);0},_=>DefWindowProcW(window,message,wparam,lparam)}
    }}
    #[test]fn target_filetime_round_trips_without_javascript_rounding(){
        let target=Target{pid:42,created_filetime:"133786986147085981".into(),package_family:FAMILY.into()};
        let value=serde_json::to_value(&target).unwrap();assert!(value["created_filetime"].is_string());
        assert_eq!(serde_json::from_value::<Target>(value).unwrap(),target);assert_eq!(expected_creation(&target).unwrap(),133786986147085981);
        for invalid in ["","0","1.3e17","133786986147085981.0"," 133786986147085981","18446744073709551616"]{let mut invalid_target=target.clone();invalid_target.created_filetime=invalid.into();assert!(expected_creation(&invalid_target).is_err());}
        let mut invalid=target;invalid.package_family="Other.Application".into();assert!(expected_creation(&invalid).is_err());
    }
    #[test]fn foreign_package_is_rejected_without_sending_any_close(){
        // The test process has no ChatGPT package identity. This never targets a user app.
        let process=Process::open(std::process::id()).unwrap().unwrap();assert!(!process.family().unwrap());
        let mut target=process.target();target.package_family="Foreign.Package".into();assert!(expected_creation(&target).is_err());
    }
    #[test]fn normal_close_is_bounded_and_a_test_window_can_refuse(){
        // Hidden, task-owned window in a test thread. No package activation, user app,
        // process shutdown, desktop input, or termination is involved in this check.
        CLOSE_SEEN.store(false,Ordering::Release);let(tx,rx)=std::sync::mpsc::channel();
        let owner=thread::spawn(move||unsafe{
            let name=native::wide(&format!("CodexProxyTray.Lifecycle.Test.{}",crate::settings::now()));let instance=GetModuleHandleW(ptr::null());
            let class=WindowClass{style:0,procedure:Some(refusing_window),class_extra:0,window_extra:0,instance,icon:ptr::null_mut(),cursor:ptr::null_mut(),background:ptr::null_mut(),menu:ptr::null(),name:name.as_ptr()};
            assert_ne!(RegisterClassW(&class),0);let window=CreateWindowExW(0,name.as_ptr(),name.as_ptr(),0,0,0,32,32,ptr::null_mut(),ptr::null_mut(),instance,ptr::null_mut());
            assert!(!window.is_null());tx.send(window as usize).unwrap();let mut message:Message=std::mem::zeroed();while GetMessageW(&mut message,ptr::null_mut(),0,0)>0{TranslateMessage(&message);DispatchMessageW(&message);}UnregisterClassW(name.as_ptr(),instance);
        });
        let window=rx.recv_timeout(Duration::from_secs(5)).unwrap()as Handle;let root=Process::open(std::process::id()).unwrap().unwrap();
        let at=Instant::now();let result=send_close(window,&root,1000);let seen=CLOSE_SEEN.load(Ordering::Acquire);let retained=unsafe{IsWindow(window)}!=0;
        // Only the test window's private cleanup message destroys this owned fixture.
        unsafe{PostMessageW(window,0x8001,0,0);}owner.join().unwrap();
        assert!(result.is_ok());assert!(seen);assert!(retained,"A refused WM_CLOSE must not destroy or force-close a window.");assert!(at.elapsed()<Duration::from_secs(5));
    }
}
