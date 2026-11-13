//! Local Windows update notices. No notification, shortcut or network work at startup.
//! Desktop Toast requires a per-user Start Menu shortcut carrying the matching AUMID:
//! https://learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid
//! A stub activator CLSID supports persisted protocol-only notifications; no COM server
//! or custom URI handler is registered. A click opens only the fixed public release page.
use std::{fs,io::Write,path::{Path,PathBuf},sync::{Mutex,OnceLock,atomic::{AtomicU64,Ordering}}};
use serde::{Deserialize,Serialize};
use windows::{core::HSTRING,Data::Xml::Dom::XmlDocument,Foundation::TypedEventHandler,UI::Notifications::{NotificationSetting,ToastFailedEventArgs,ToastNotification,ToastNotificationManager},Win32::System::WinRT::{RoInitialize,RoUninitialize,RO_INIT_MULTITHREADED}};
use crate::{core,settings,update::UpdateState,installation::APP_ID};

const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const RECEIPT:&str="update-notice-v1.json";


#[derive(Clone,Default,Serialize)]
pub struct NoticeSnapshot{pub submitted_version:Option<String>,pub submitted_at:Option<u64>,pub error:Option<String>}
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]
struct Receipt{schema:u32,version:String,submitted_at:u64}
struct State{root:PathBuf,last_version:Option<String>,writable:bool,last_attempt:Option<(Option<u64>,String)>,view:NoticeSnapshot}
static STATE:OnceLock<Mutex<State>>=OnceLock::new();
static LAST_FAILURE:Mutex<Option<String>>=Mutex::new(None);
static GENERATION:AtomicU64=AtomicU64::new(0);
// Retain the last WinRT object so asynchronous delivery errors remain observable.
static LAST_TOAST:Mutex<Option<ToastNotification>>=Mutex::new(None);
fn state()->&'static Mutex<State>{STATE.get_or_init(||Mutex::new(State::load(settings::root())))}
pub fn snapshot()->NoticeSnapshot{
    let mut view=state().lock().unwrap_or_else(|error|error.into_inner()).view.clone();
    if let Some(error)=LAST_FAILURE.lock().ok().and_then(|value|value.clone()){view.error=Some(error);}view
}
/// true means Windows accepted Show(), not that its banner was visible. Focus Assist
/// or other OS presentation policy may still suppress it. Call only for completed checks.
pub fn maybe_notify(update:&UpdateState)->Result<bool,String>{
    let mut state=state().lock().map_err(|_|"更新通知状态暂不可用。".to_owned())?;
    state.notify(update,|version|{
        let generation=GENERATION.fetch_add(1,Ordering::AcqRel).wrapping_add(1);
        *LAST_FAILURE.lock().map_err(|_|"更新通知状态暂不可用。".to_owned())?=None;
        let version=version.to_owned();
        std::thread::Builder::new().name("update-notice".into()).spawn(move||send_to_windows(&version,generation)).map_err(|_|"Windows 更新通知线程未能启动。".to_owned())?.join().map_err(|_|"Windows 更新通知未能完成。".to_owned())?
    })
}
impl State{
    fn load(root:PathBuf)->Self{
        let path=root.join(RECEIPT);let loaded=(||->Result<Option<Receipt>,String>{
            let metadata=match fs::metadata(&path){Ok(value)=>value,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err("无法读取更新通知记录，已有记录已保留。".into())};
            if metadata.len()>4096{return Err("更新通知记录无效，已有记录已保留。".into())}
            let receipt:Receipt=serde_json::from_slice(&fs::read(&path).map_err(|_|"无法读取更新通知记录，已有记录已保留。")?).map_err(|_|"更新通知记录无效，已有记录已保留。")?;
            if receipt.schema!=1||core::version_parts(&receipt.version).is_none()||receipt.submitted_at==0||receipt.submitted_at>8_640_000_000_000_000{return Err("更新通知记录无效，已有记录已保留。".into())}Ok(Some(receipt))
        })();
        let writable=loaded.is_ok();let mut view=NoticeSnapshot::default();let last_version=match loaded{Ok(Some(receipt))=>{view.submitted_version=Some(receipt.version.clone());view.submitted_at=Some(receipt.submitted_at);Some(receipt.version)},Ok(None)=>None,Err(error)=>{view.error=Some(error);None}};
        Self{root,last_version,writable,last_attempt:None,view}
    }
    fn notify(&mut self,update:&UpdateState,send:impl FnOnce(&str)->Result<(),String>)->Result<bool,String>{
        if update.checking||update.status!="available"{return Ok(false)}
        let Some(parts)=update.latest.as_deref().and_then(core::version_parts)else{return Ok(false)};let version=format!("{}.{}.{}",parts[0],parts[1],parts[2]);
        if !core::newer_version(&version,env!("CARGO_PKG_VERSION"))||self.last_version.as_ref().is_some_and(|previous|!core::newer_version(&version,previous)){return Ok(false)}
        if !self.writable{return Err(self.view.error.clone().unwrap_or_else(||"更新通知记录不可写。".into()))}
        let attempt=(update.checked_at,version.clone());if self.last_attempt.as_ref()==Some(&attempt){return Ok(false)}self.last_attempt=Some(attempt);
        if let Err(error)=send(&version){self.view.error=Some(error.clone());return Err(error)}
        // Memory and durable dedup both move only after Windows accepted this version.
        let at=settings::now();self.last_version=Some(version.clone());self.view=NoticeSnapshot{submitted_version:Some(version.clone()),submitted_at:Some(at),error:None};
        if let Err(error)=save_receipt(&self.root,&Receipt{schema:1,version,submitted_at:at}){self.view.error=Some(error.clone());return Err(error)}Ok(true)
    }
}
fn save_receipt(root:&Path,receipt:&Receipt)->Result<(),String>{
    let write=(||->std::io::Result<()>{fs::create_dir_all(root)?;let temporary=root.join(format!(".update-notice-{}-{}.tmp",std::process::id(),settings::now()));
        let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let result=(||{file.write_all(&serde_json::to_vec(receipt).map_err(std::io::Error::other)?)?;file.sync_all()?;drop(file);fs::rename(&temporary,root.join(RECEIPT))})();
        if result.is_err(){let _=fs::remove_file(&temporary);}result
    })();write.map_err(|_|"已提交 Windows 更新通知，但去重记录未能保存；重开助手后可能再次提醒。".into())
}
fn windows_error(stage:&str,error:windows::core::Error)->String{format!("{stage}（0x{:08X}）。",error.code().0 as u32)}
fn toast_xml(version:&str)->Result<String,String>{
    if core::version_parts(version).is_none(){return Err("更新通知版本无效。".into())}
    Ok(format!(r#"<toast activationType="protocol" launch="{RELEASES}"><visual><binding template="ToastGeneric"><text>CodexProxyTray {version} 已发布</text><text>当前版本 {}。点击查看发布说明和下载入口。</text></binding></visual><audio silent="true"/></toast>"#,env!("CARGO_PKG_VERSION")))
}
struct Apartment;
impl Drop for Apartment{fn drop(&mut self){unsafe{RoUninitialize();}}}
fn send_to_windows(version:&str,generation:u64)->Result<(),String>{
    unsafe{RoInitialize(RO_INIT_MULTITHREADED).map_err(|error|windows_error("无法初始化 Windows 通知",error))?;}let _apartment=Apartment;
    crate::installation::ensure_notification_shortcut()?;
    let notifier=ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)).map_err(|error|windows_error("无法连接 Windows 通知服务",error))?;
    let setting=notifier.Setting().map_err(|error|windows_error("无法读取 Windows 通知状态",error))?;
    if setting!=NotificationSetting::Enabled{return Err(match setting{NotificationSetting::DisabledForApplication=>"Windows 已关闭本应用的通知。",NotificationSetting::DisabledForUser=>"Windows 已关闭当前用户的通知。",NotificationSetting::DisabledByGroupPolicy=>"Windows 策略禁用了通知。",_=>"Windows 当前不允许显示本应用通知。"}.into())}
    let document=XmlDocument::new().map_err(|error|windows_error("无法创建更新通知",error))?;document.LoadXml(&HSTRING::from(toast_xml(version)?)).map_err(|error|windows_error("无法读取更新通知内容",error))?;
    let toast=ToastNotification::CreateToastNotification(&document).map_err(|error|windows_error("无法创建 Windows 更新通知",error))?;
    toast.SetTag(&HSTRING::from("new-version")).map_err(|error|windows_error("无法设置更新通知标识",error))?;toast.SetGroup(&HSTRING::from("updates")).map_err(|error|windows_error("无法设置更新通知分组",error))?;
    toast.Failed(&TypedEventHandler::<ToastNotification,ToastFailedEventArgs>::new(move|_,args|{
        if GENERATION.load(Ordering::Acquire)!=generation{return Ok(())}
        let code=args.as_ref().and_then(|value|value.ErrorCode().ok()).map(|value|format!("（0x{:08X}）",value.0 as u32)).unwrap_or_default();
        if let Ok(mut last)=LAST_FAILURE.lock(){if GENERATION.load(Ordering::Acquire)==generation{*last=Some(format!("Windows 未能投递更新通知{code}。请在设置中查看新版本。"));}}Ok(())
    })).map_err(|error|windows_error("无法跟踪更新通知结果",error))?;
    *LAST_TOAST.lock().map_err(|_|"无法保留更新通知状态。".to_owned())?=Some(toast.clone());
    notifier.Show(&toast).map_err(|error|windows_error("Windows 未接受更新通知",error))?;
    if let Some(error)=LAST_FAILURE.lock().ok().and_then(|value|value.clone()){return Err(error)}Ok(())
}

#[cfg(test)]mod tests{
    use super::*;
    fn update(version:&str,checked_at:u64)->UpdateState{UpdateState{status:"available".into(),latest:Some(version.into()),checked_at:Some(checked_at),..Default::default()}}
    #[test]fn dedup_is_committed_only_after_os_handoff_and_survives_restart(){
        let root=std::env::temp_dir().join(format!("codex-proxy-notice-{}-{}",std::process::id(),settings::now()));assert!(!root.exists());fs::create_dir(&root).unwrap();
        let mut state=State::load(root.clone());let candidate=update("999.1.0",1);let mut calls=0;
        assert!(state.notify(&candidate,|_|{calls+=1;Err("mock OS unavailable".into())}).is_err());assert!(!root.join(RECEIPT).exists());assert!(state.last_version.is_none());
        assert!(!state.notify(&candidate,|_|panic!("Do not spin-retry the same completed check")).unwrap());
        assert!(state.notify(&update("999.1.0",2),|_|{calls+=1;Ok(())}).unwrap());assert_eq!(calls,2);
        let mut reloaded=State::load(root.clone());assert!(!reloaded.notify(&update("v999.1.0",3),|_|panic!("Same version must not show twice")).unwrap());assert!(!reloaded.notify(&update("999.0.0",4),|_|panic!("A stale cache must not notify an older version")).unwrap());
        assert!(reloaded.notify(&update("999.2.0",5),|_|Ok(())).unwrap());fs::remove_file(root.join(RECEIPT)).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn invalid_receipts_and_untrusted_versions_are_preserved_or_rejected(){
        let root=std::env::temp_dir().join(format!("codex-proxy-notice-invalid-{}-{}",std::process::id(),settings::now()));assert!(!root.exists());fs::create_dir(&root).unwrap();fs::write(root.join(RECEIPT),b"{invalid").unwrap();let mut state=State::load(root.clone());
        assert!(state.notify(&update("999.1.0",1),|_|panic!("Unknown dedup state must not be overwritten")).is_err());assert_eq!(fs::read(root.join(RECEIPT)).unwrap(),b"{invalid");assert!(toast_xml("<script>").is_err());
        let xml=toast_xml("999.1.0").unwrap();assert!(xml.contains("activationType=\"protocol\""));assert!(xml.contains(RELEASES));assert!(!xml.contains("<actions>"));assert!(!xml.contains("powershell"));
        fs::remove_file(root.join(RECEIPT)).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn receipt_failure_is_reported_after_handoff_without_repeating_in_process(){
        let root=std::env::temp_dir().join(format!("codex-proxy-notice-save-{}-{}",std::process::id(),settings::now()));assert!(!root.exists());fs::create_dir(&root).unwrap();let mut state=State::load(root.clone());
        fs::create_dir(root.join(RECEIPT)).unwrap();assert!(state.notify(&update("999.1.0",1),|_|Ok(())).is_err());assert_eq!(state.view.submitted_version.as_deref(),Some("999.1.0"));assert!(state.view.error.as_deref().unwrap().contains("去重记录未能保存"));
        assert!(!state.notify(&update("999.1.0",2),|_|panic!("Already handed off in this process")).unwrap());assert_eq!(fs::read_dir(&root).unwrap().count(),1,"Owned temporary receipt is cleaned after failure");fs::remove_dir(root.join(RECEIPT)).unwrap();fs::remove_dir(root).unwrap();
    }
}
