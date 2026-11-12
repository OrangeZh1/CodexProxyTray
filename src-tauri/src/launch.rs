use std::{ffi::c_void,path::Path,ptr};
use std::os::windows::ffi::OsStrExt;
use crate::settings::{self,Config};

#[link(name="shell32")] extern "system" {
    fn ShellExecuteW(window:*mut c_void,operation:*const u16,file:*const u16,parameters:*const u16,directory:*const u16,show:i32)->isize;
}
fn wide(value:&std::ffi::OsStr)->Vec<u16>{value.encode_wide().chain(Some(0)).collect()}

/// Opens only the installed scoped entry; Windows chooses its configured console host.
/// Callers decide whether this is a user action or an explicitly selected startup target.
pub fn open_cli(root:&Path,config:&Config)->Result<(),String>{
    let shim=root.join("cli").join("codex.exe");
    if !shim.is_file(){return Err("Codex CLI 专用入口尚未就绪，请等待后台自动检测。".into())}
    let directory=settings::cli_directory(config)?;
    let verb:Vec<u16>="open\0".encode_utf16().collect();let file=wide(shim.as_os_str());let directory=wide(directory.as_os_str());
    let result=unsafe{ShellExecuteW(ptr::null_mut(),verb.as_ptr(),file.as_ptr(),ptr::null(),directory.as_ptr(),1)};
    if result<=32{Err(format!("无法打开 Codex CLI：Windows 错误 {result}"))}else{Ok(())}
}
