//! User-invoked folder selection only. Does not save config or read file contents.
use std::{path::Path,sync::atomic::{AtomicBool,Ordering}};
use windows::{core::{PCWSTR,HRESULT},Win32::{Foundation::HWND,System::Com::{CoCreateInstance,CoInitializeEx,CoUninitialize,CoTaskMemFree,CLSCTX_INPROC_SERVER,COINIT_APARTMENTTHREADED,COINIT_DISABLE_OLE1DDE},UI::{Shell::{IFileOpenDialog,IShellItem,FileOpenDialog,SHCreateItemFromParsingName,FOS_PICKFOLDERS,FOS_FORCEFILESYSTEM,FOS_PATHMUSTEXIST,FOS_NOCHANGEDIR,FOS_DONTADDTORECENT,SIGDN_FILESYSPATH},WindowsAndMessaging::IsWindow}}};
static OPEN:AtomicBool=AtomicBool::new(false);
const CANCELLED:HRESULT=HRESULT(0x800704c7u32 as i32);
struct Busy;
impl Drop for Busy{fn drop(&mut self){OPEN.store(false,Ordering::Release);}}
struct Apartment;
impl Drop for Apartment{fn drop(&mut self){unsafe{CoUninitialize();}}}
fn error(stage:&str,value:windows::core::Error)->String{format!("{stage}（0x{:08X}）。",value.code().0 as u32)}
fn hint(value:Option<String>)->Option<Vec<u16>>{let value=value?;let value=value.trim();if value.is_empty()||value.len()>131072||value.contains('\0')||value.encode_utf16().count()>32767||!Path::new(value).is_absolute(){None}else{Some(crate::native::wide(value))}}
fn cancelled(result:Result<(),windows::core::Error>)->Result<bool,String>{match result{Ok(())=>Ok(false),Err(value)if value.code()==CANCELLED=>Ok(true),Err(value)=>Err(error("文件夹选择未完成",value))}}
fn show(initial:Option<String>,owner:usize)->Result<Option<String>,String>{unsafe{
    CoInitializeEx(None,COINIT_APARTMENTTHREADED|COINIT_DISABLE_OLE1DDE).ok().map_err(|value|error("无法准备文件夹选择器",value))?;let _apartment=Apartment;
    let owner=HWND(owner as *mut std::ffi::c_void);if owner.0.is_null()||!IsWindow(Some(owner)).as_bool(){return Err("主窗口已变化，请重新打开设置后选择文件夹。".into())}
    let dialog:IFileOpenDialog=CoCreateInstance(&FileOpenDialog,None,CLSCTX_INPROC_SERVER).map_err(|value|error("无法打开文件夹选择器",value))?;
    let options=dialog.GetOptions().map_err(|value|error("无法读取文件夹选择选项",value))?;
    dialog.SetOptions(options|FOS_PICKFOLDERS|FOS_FORCEFILESYSTEM|FOS_PATHMUSTEXIST|FOS_NOCHANGEDIR|FOS_DONTADDTORECENT).map_err(|value|error("无法设置文件夹选择范围",value))?;
    let title=crate::native::wide("选择 Codex CLI 工作目录");dialog.SetTitle(PCWSTR(title.as_ptr())).map_err(|value|error("无法设置文件夹选择标题",value))?;
    if let Some(path)=hint(initial){if let Ok(folder)=SHCreateItemFromParsingName::<_,_,IShellItem>(PCWSTR(path.as_ptr()),None){let _=dialog.SetFolder(&folder);}}
    if cancelled(dialog.Show(Some(owner)))?{return Ok(None)}
    let selected=dialog.GetResult().map_err(|value|error("无法取得所选文件夹",value))?;let raw=selected.GetDisplayName(SIGDN_FILESYSPATH).map_err(|value|error("无法取得所选文件夹路径",value))?;
    let path=raw.to_string();CoTaskMemFree(Some(raw.0.cast()));let path=path.map_err(|value|error("所选文件夹路径无法识别",value))?;
    if hint(Some(path.clone())).is_none(){return Err("所选项目没有有效的文件系统路径。".into())}Ok(Some(path))
}}
pub fn choose(initial:Option<String>,owner:usize)->Result<Option<String>,String>{
    OPEN.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_|"文件夹选择器已经打开，请先完成当前选择。".to_owned())?;let _busy=Busy;
    std::thread::Builder::new().name("cli-folder-picker".into()).spawn(move||show(initial,owner)).map_err(|_|"文件夹选择线程无法启动。".to_owned())?.join().map_err(|_|"文件夹选择未能完成。".to_owned())?
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn cancellation_is_not_an_error_and_invalid_initial_hints_do_not_open_anything(){
        assert!(cancelled(Err(windows::core::Error::from_hresult(CANCELLED))).unwrap());assert!(!cancelled(Ok(())).unwrap());assert!(hint(Some("relative/path".into())).is_none());assert!(hint(Some("C:\\private\0suffix".into())).is_none());assert!(hint(Some("shell:AppsFolder".into())).is_none());assert!(hint(None).is_none());assert!(hint(Some("C:\\fixture".into())).is_some());
    }
}
