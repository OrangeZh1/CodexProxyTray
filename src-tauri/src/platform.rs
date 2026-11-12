use std::{ffi::c_void, ptr, path::Path};
use crate::native::wide;
type Handle=*mut c_void;
const HKCU:Handle=(-2147483647isize) as Handle;
const RUN:&str="Software\\Microsoft\\Windows\\CurrentVersion\\Run";
#[link(name="advapi32")]extern "system"{
    fn RegGetValueW(key:Handle,subkey:*const u16,name:*const u16,flags:u32,kind:*mut u32,data:*mut c_void,len:*mut u32)->i32;
    fn RegCreateKeyExW(key:Handle,subkey:*const u16,reserved:u32,class:*const u16,options:u32,access:u32,security:Handle,result:*mut Handle,disposition:*mut u32)->i32;
    fn RegSetValueExW(key:Handle,name:*const u16,reserved:u32,kind:u32,data:*const u8,len:u32)->i32;
    fn RegDeleteValueW(key:Handle,name:*const u16)->i32;
    fn RegCloseKey(key:Handle)->i32;
}
#[link(name="user32")]extern "system"{fn SendMessageTimeoutW(window:Handle,message:u32,wparam:usize,lparam:isize,flags:u32,timeout:u32,result:*mut usize)->isize;}
#[link(name="kernel32")]extern "system"{
    fn CreateEventW(security:Handle,manual:i32,initial:i32,name:*const u16)->Handle;
    fn OpenEventW(access:u32,inherit:i32,name:*const u16)->Handle;
    fn SetEvent(event:Handle)->i32;fn ResetEvent(event:Handle)->i32;fn CloseHandle(handle:Handle)->i32;
    fn WaitForSingleObject(handle:Handle,milliseconds:u32)->u32;
    fn SetConsoleCtrlHandler(handler:Option<unsafe extern "system" fn(u32)->i32>,add:i32)->i32;
}
fn reg_read(key:&str,name:&str)->Result<Option<(String,u32)>,String>{unsafe{let k=wide(key);let n=wide(name);let(mut len,mut kind)=(0,0);let status=RegGetValueW(HKCU,k.as_ptr(),n.as_ptr(),0x10000006,&mut kind,ptr::null_mut(),&mut len);if status==2{return Ok(None)}if status!=0{return Err(format!("读取用户设置失败：{status}"))}let mut data=vec![0u16;len as usize/2];let status=RegGetValueW(HKCU,k.as_ptr(),n.as_ptr(),0x10000006,&mut kind,data.as_mut_ptr().cast(),&mut len);if status!=0{return Err(format!("读取用户设置失败：{status}"))}Ok(Some((String::from_utf16_lossy(&data[..data.iter().position(|v|*v==0).unwrap_or(data.len())]),kind)))}}
fn reg_write(key:&str,name:&str,value:Option<&str>,kind:u32)->Result<(),String>{unsafe{let k=wide(key);let n=wide(name);let mut handle=ptr::null_mut();let status=RegCreateKeyExW(HKCU,k.as_ptr(),0,ptr::null(),0,3,ptr::null_mut(),&mut handle,ptr::null_mut());if status!=0{return Err(format!("无法写入用户设置：{status}"))}let status=match value{Some(v)=>{let data=wide(v);RegSetValueExW(handle,n.as_ptr(),0,kind,data.as_ptr().cast(),(data.len()*2)as u32)},None=>RegDeleteValueW(handle,n.as_ptr())};RegCloseKey(handle);if status!=0&&status!=2{Err(format!("无法保存用户设置：{status}"))}else{Ok(())}}}
fn autostart_command(exe:&Path)->String{format!("\"{}\" --background",exe.display())}
pub fn autostart()->bool{std::env::current_exe().ok().and_then(|exe|reg_read(RUN,"CodexProxyTray").ok().flatten().map(|(value,_)|value.eq_ignore_ascii_case(&autostart_command(&exe)))).unwrap_or(false)}
pub fn set_autostart(enabled:bool,exe:&Path)->Result<(),String>{let command=enabled.then(||autostart_command(exe));reg_write(RUN,"CodexProxyTray",command.as_deref(),1)}
pub fn ensure_user_cli_path(dir:&Path)->Result<(),String>{
    let(original,kind)=reg_read("Environment","Path")?.unwrap_or((String::new(),2));let directory=dir.to_string_lossy();
    if original.split(';').any(|entry|entry.trim().trim_matches('"').trim_end_matches('\\').eq_ignore_ascii_case(directory.trim_end_matches('\\'))||entry.trim().eq_ignore_ascii_case("%APPDATA%\\npm")){return Ok(())}
    let updated=if original.is_empty(){directory.into_owned()}else{format!("{original};{directory}")};reg_write("Environment","Path",Some(&updated),kind)?;
    std::thread::spawn(||unsafe{let env=wide("Environment");let mut result=0;SendMessageTimeoutW(0xffffusize as Handle,0x001A,0,env.as_ptr()as isize,2,1000,&mut result);});Ok(())
}
pub fn remove_legacy_cli_path(dir:&Path)->Result<bool,String>{
    let (original,kind)=reg_read("Environment","Path")?.unwrap_or((String::new(),2));let directory=dir.to_string_lossy();
    let updated=original.split(';').filter(|entry|!entry.trim().trim_matches('"').trim_end_matches('\\').eq_ignore_ascii_case(directory.trim_end_matches('\\'))).collect::<Vec<_>>().join(";");
    if updated==original{return Ok(false)}reg_write("Environment","Path",Some(&updated),kind)?;
    std::thread::spawn(||unsafe{let env=wide("Environment");let mut result=0;SendMessageTimeoutW(0xffffusize as Handle,0x001A,0,env.as_ptr()as isize,2,1000,&mut result);});Ok(true)
}
pub fn wait_for_console_child(){
    // This handler belongs only to the shim; unlike the ignore flag it is not inherited.
    // Let the real CLI receive Ctrl+C, while the shim waits to return its exit code.
    unsafe extern "system" fn handler(signal:u32)->i32{if signal==0||signal==1{1}else{0}}
    unsafe{SetConsoleCtrlHandler(Some(handler),1);}
}
pub struct GuardEvent(Handle);
impl GuardEvent{pub fn new(name:&str)->Result<Self,String>{unsafe{let n=wide(name);let h=CreateEventW(ptr::null_mut(),1,0,n.as_ptr());if h.is_null(){Err(std::io::Error::last_os_error().to_string())}else{Ok(Self(h))}}}pub fn set(&self,enabled:bool){unsafe{if enabled{SetEvent(self.0);}else{ResetEvent(self.0);}}}}
impl Drop for GuardEvent{fn drop(&mut self){unsafe{ResetEvent(self.0);CloseHandle(self.0);}}}
pub fn guard_active(name:&str)->bool{unsafe{let n=wide(name);let h=OpenEventW(0x00100000,0,n.as_ptr());if h.is_null(){return false}let active=WaitForSingleObject(h,0)==0;CloseHandle(h);active}}
