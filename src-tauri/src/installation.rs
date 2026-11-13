//! Per-user product identity and registration. Explicit CLI maintenance only;
//! portable notifications share the product shortcut without registering an app.
use std::{collections::HashSet,ffi::{c_void,OsString},fs::{self,File},io::Read,mem::size_of,os::windows::fs::MetadataExt,path::{Component,Path,PathBuf},ptr};
use serde::{Deserialize,Serialize};
use windows::{core::{GUID,Interface,PCWSTR,PWSTR},Win32::{Foundation::{CloseHandle,HANDLE,HLOCAL,LocalFree,PROPERTYKEY,WAIT_ABANDONED,WAIT_OBJECT_0},Security::{GetTokenInformation,OpenProcessToken,TokenUser,TOKEN_QUERY,TOKEN_USER,Authorization::ConvertSidToStringSidW,Cryptography::*},System::{Com::{CoCreateInstance,CoInitializeEx,CoTaskMemFree,CoUninitialize,IPersistFile,CLSCTX_INPROC_SERVER,COINIT_MULTITHREADED,STGM_READ,StructuredStorage::{PROPVARIANT,PropVariantClear,InitPropVariantFromCLSID}},Threading::{CreateMutexW,GetCurrentProcess,ReleaseMutex,WaitForSingleObject},Variant::{VT_CLSID,VT_LPWSTR}},UI::{Shell::{IShellLinkW,ShellLink,SHGetKnownFolderPath,FOLDERID_Programs,KF_FLAG_DEFAULT,SHChangeNotify,SHCNE_CREATE,SHCNE_DELETE,SHCNE_UPDATEITEM,SHCNF_PATHW,SetCurrentProcessExplicitAppUserModelID,PropertiesSystem::IPropertyStore},WindowsAndMessaging::SW_SHOWNORMAL}}};
use crate::{core,native::wide};

pub const APP_ID:&str="OrangeZh1.CodexProxyTray";
const PRODUCT:&str="CodexProxyTray.Installation";
const GUI:&str="CodexProxyTray.exe";
const REGISTRY:&str=r"Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexProxyTray";
const REG_OWNER:&str="CodexProxyTrayOwner";
const STUB_ACTIVATOR:GUID=GUID::from_u128(0x22b3d738_35ef_4949_9b82_bc4c49fd96f8);
const APP_MODEL:GUID=GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3);
const APP_ID_KEY:PROPERTYKEY=PROPERTYKEY{fmtid:APP_MODEL,pid:5};
const ACTIVATOR_KEY:PROPERTYKEY=PROPERTYKEY{fmtid:APP_MODEL,pid:26};
const PAYLOAD:[&str;10]=[GUI,"codex-proxy-cli.exe","WebView2Loader.dll","LICENSE","THIRD_PARTY_NOTICES.md","flag-icons-license.txt","package-receipt.json","QUICKSTART.txt","fluent-icons-license.txt","SHA256SUMS.txt"];
const PACKAGE_FILES:[&str;6]=[GUI,"codex-proxy-cli.exe","WebView2Loader.dll","LICENSE","THIRD_PARTY_NOTICES.md","flag-icons-license.txt"];

#[derive(Clone,Serialize,Deserialize)]#[serde(deny_unknown_fields)]
struct Receipt{schema:u32,product:String,install_id:String,owner_sid:String,root:String,uninstaller_sha256:String}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct FileHash{name:String,sha256:String}
#[derive(Deserialize)]struct ActiveSlot{path:String,version:String,files:Vec<FileHash>}
#[derive(Deserialize)]struct Active{schema:u32,state:String,current:ActiveSlot}
#[derive(Deserialize)]struct Package{schema:u32,product:String,version:String,files:Vec<FileHash>}
struct Installation{root:PathBuf,exe:PathBuf,receipt:Receipt,version:String}
struct Handle(HANDLE);
impl Drop for Handle{fn drop(&mut self){unsafe{let _=CloseHandle(self.0);}}}
struct IdentityLock(Handle);
impl Drop for IdentityLock{fn drop(&mut self){unsafe{let _=ReleaseMutex(self.0.0);}}}
struct Apartment;
impl Drop for Apartment{fn drop(&mut self){unsafe{CoUninitialize();}}}

type Key=*mut c_void;
const HKCU:Key=(-2147483647isize)as Key;
#[link(name="advapi32")]unsafe extern "system"{
    fn RegOpenKeyExW(key:Key,subkey:*const u16,options:u32,access:u32,result:*mut Key)->i32;
    fn RegCreateKeyExW(key:Key,subkey:*const u16,reserved:u32,class:*const u16,options:u32,access:u32,security:*const c_void,result:*mut Key,disposition:*mut u32)->i32;
    fn RegGetValueW(key:Key,subkey:*const u16,name:*const u16,flags:u32,kind:*mut u32,data:*mut c_void,len:*mut u32)->i32;
    fn RegSetValueExW(key:Key,name:*const u16,reserved:u32,kind:u32,data:*const u8,len:u32)->i32;
    fn RegDeleteKeyW(key:Key,subkey:*const u16)->i32;
    fn RegFlushKey(key:Key)->i32;
    fn RegCloseKey(key:Key)->i32;
}
struct OwnedKey(Key);
impl Drop for OwnedKey{fn drop(&mut self){unsafe{RegCloseKey(self.0);}}}
fn windows_error(stage:&str,error:windows::core::Error)->String{format!("{stage}（0x{:08X}）。",error.code().0 as u32)}
fn hex(value:&str,length:usize)->bool{value.len()==length&&value.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))}
fn path_text(path:&Path)->Result<String,String>{
    let text=path.to_str().ok_or("安装路径无法编码。")?.replace('/',"\\");
    Ok(if let Some(tail)=text.strip_prefix(r"\\?\UNC\"){format!(r"\\{tail}")}else{text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()})
}
fn absolute(path:&Path)->bool{path.is_absolute()&&!path.components().any(|part|matches!(part,Component::ParentDir|Component::CurDir))&&path.to_str().is_some_and(|value|!value.contains(['\0','"'])&&value.encode_utf16().count()<32767)}
fn same_path(first:&Path,second:&Path)->bool{matches!((path_text(first),path_text(second)),(Ok(a),Ok(b))if a.eq_ignore_ascii_case(&b))}
fn plain(path:&Path)->Result<(),String>{
    for part in path.ancestors(){let metadata=fs::symlink_metadata(part).map_err(|_|"安装路径无法验证，原文件已保留。")?;if metadata.file_attributes()&0x400!=0{return Err("安装路径包含重解析点，原文件已保留。".into())}}Ok(())
}
fn read_json<T:for<'de>Deserialize<'de>>(path:&Path,limit:u64)->Result<T,String>{
    plain(path)?;let file=File::open(path).map_err(|_|"安装记录无法读取。")?;let metadata=file.metadata().map_err(|_|"安装记录无法验证。")?;if !metadata.is_file()||metadata.len()>limit{return Err("安装记录超过读取范围。".into())}
    let mut bytes=vec![];file.take(limit+1).read_to_end(&mut bytes).map_err(|_|"安装记录读取失败。")?;if bytes.len()as u64>limit{return Err("安装记录超过读取范围。".into())}serde_json::from_slice(&bytes).map_err(|_|"安装记录格式无效，原文件已保留。".into())
}
fn sha256(path:&Path)->Result<String,String>{unsafe{
    plain(path)?;let metadata=fs::metadata(path).map_err(|_|"安装文件无法读取。")?;if !metadata.is_file()||metadata.len()>128*1024*1024{return Err("安装文件超过校验范围。".into())}
    let mut algorithm=BCRYPT_ALG_HANDLE::default();if BCryptOpenAlgorithmProvider(&mut algorithm,BCRYPT_SHA256_ALGORITHM,PCWSTR::null(),BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).0<0{return Err("无法创建安装文件校验。".into())}
    let mut hash=BCRYPT_HASH_HANDLE::default();let result=(||{if BCryptCreateHash(algorithm,&mut hash,None,None,0).0<0{return Err("无法初始化安装文件校验。".into())}let mut file=File::open(path).map_err(|_|"安装文件校验读取失败。")?;let mut bytes=[0u8;65536];let mut total=0u64;loop{let count=file.read(&mut bytes).map_err(|_|"安装文件校验读取失败。")?;if count==0{break}total+=count as u64;if total>128*1024*1024{return Err("安装文件超过校验范围。".into())}if BCryptHashData(hash,&bytes[..count],0).0<0{return Err("安装文件校验失败。".into())}}if total!=metadata.len(){return Err("安装文件在校验期间发生变化。".into())}let mut digest=[0u8;32];if BCryptFinishHash(hash,&mut digest,0).0<0{return Err("安装文件校验失败。".into())}Ok(digest.iter().map(|byte|format!("{byte:02x}")).collect())})();if !hash.0.is_null(){let _=BCryptDestroyHash(hash);}let _=BCryptCloseAlgorithmProvider(algorithm,0);result
}}
fn current_sid()->Result<String,String>{unsafe{
    let mut raw=HANDLE::default();OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut raw).map_err(|error|windows_error("无法验证当前用户",error))?;let token=Handle(raw);let mut needed=0;let _=GetTokenInformation(token.0,TokenUser,None,0,&mut needed);if needed<size_of::<TOKEN_USER>()as u32||needed>65536{return Err("当前用户身份长度无效。".into())}
    let mut bytes=vec![0u64;(needed as usize).div_ceil(8)];GetTokenInformation(token.0,TokenUser,Some(bytes.as_mut_ptr().cast()),needed,&mut needed).map_err(|error|windows_error("无法读取当前用户",error))?;let mut raw=PWSTR::null();ConvertSidToStringSidW((*(bytes.as_ptr()as *const TOKEN_USER)).User.Sid,&mut raw).map_err(|error|windows_error("无法编码当前用户",error))?;let result=raw.to_string().map_err(|_|"当前用户身份无效。".to_owned());let _=LocalFree(Some(HLOCAL(raw.0.cast())));let sid=result?;if matches!(sid.as_str(),"S-1-5-18"|"S-1-5-19"|"S-1-5-20"){return Err("安装注册仅限当前交互用户。".into())}Ok(sid)
}}
fn identity_lock(owner:&str)->Result<IdentityLock,String>{unsafe{
    let name=wide(&format!(r"Global\CodexProxyTray.Installation.{owner}"));let raw=CreateMutexW(None,false,PCWSTR(name.as_ptr())).map_err(|error|windows_error("无法取得产品入口操作锁",error))?;let handle=Handle(raw);match WaitForSingleObject(handle.0,5000){WAIT_OBJECT_0|WAIT_ABANDONED=>Ok(IdentityLock(handle)),_=>Err("另一个产品入口操作尚未完成。".into())}
}}
fn validate_receipt(receipt:&Receipt,root:&Path,owner:&str)->Result<(),String>{
    if receipt.schema!=1||receipt.product!=PRODUCT||!(hex(&receipt.install_id,32)||hex(&receipt.install_id,64))||receipt.owner_sid!=owner||!absolute(Path::new(&receipt.root))||!same_path(Path::new(&receipt.root),root)||!hex(&receipt.uninstaller_sha256,64){return Err("安装身份记录不属于当前用户或目录。".into())}Ok(())
}
fn file_list_valid(files:&[FileHash])->bool{let mut seen=HashSet::new();files.len()==PAYLOAD.len()&&files.iter().all(|file|PAYLOAD.contains(&file.name.as_str())&&hex(&file.sha256,64)&&seen.insert(file.name.as_str()))}
fn package_list_valid(files:&[FileHash])->bool{let mut seen=HashSet::new();files.len()==PACKAGE_FILES.len()&&files.iter().all(|file|PACKAGE_FILES.contains(&file.name.as_str())&&hex(&file.sha256,64)&&seen.insert(file.name.as_str()))}
fn payload(root:&Path,version:Option<&str>)->Result<String,String>{
    let current=root.join("current");let active:Active=read_json(&root.join("active-version.json"),262144)?;
    if active.schema!=1||active.state!="active"||!absolute(Path::new(&active.current.path))||!same_path(Path::new(&active.current.path),&current)||active.current.version.starts_with('v')||core::version_parts(&active.current.version).is_none()||version.is_some_and(|value|active.current.version!=value)||!file_list_valid(&active.current.files){return Err("活动版本记录或文件清单不匹配。".into())}
    for file in &active.current.files{if sha256(&current.join(&file.name))?!=file.sha256{return Err("活动版本文件校验不匹配。".into())}}
    let package:Package=read_json(&current.join("package-receipt.json"),65536)?;if package.schema!=1||package.product!="CodexProxyTray"||package.version!=active.current.version||!package_list_valid(&package.files)||package.files.iter().any(|file|!active.current.files.iter().any(|active|active.name==file.name&&active.sha256==file.sha256)){return Err("活动版本与产品打包身份不匹配。".into())}Ok(active.current.version)
}
impl Installation{
    fn load(root:&Path,owner:&str,version:Option<&str>)->Result<Self,String>{
        if !absolute(root){return Err("安装根目录不是有效的绝对路径。".into())}plain(root)?;let root=root.canonicalize().map_err(|_|"安装根目录无法验证。")?;let receipt:Receipt=read_json(&root.join("installation.json"),16384)?;validate_receipt(&receipt,&root,owner)?;
        if sha256(&root.join("uninstall.exe"))?!=receipt.uninstaller_sha256{return Err("卸载程序校验不匹配，原安装已保留。".into())}let version=payload(&root,version)?;Ok(Self{exe:root.join("current").join(GUI),root,receipt,version})
    }
    fn own(owner:&str)->Result<Self,String>{
        let exe=std::env::current_exe().map_err(|_|"无法确定当前程序。")?;if !absolute(&exe)||exe.file_name().is_none_or(|name|name!=GUI)||exe.parent().and_then(Path::file_name).is_none_or(|name|!name.eq_ignore_ascii_case("current")){return Err("此注册操作仅适用于已核验安装的 current 程序。".into())}
        plain(&exe)?;let root=exe.parent().and_then(Path::parent).ok_or("安装目录结构无效。")?;let installation=Self::load(root,owner,Some(env!("CARGO_PKG_VERSION")))?;if !same_path(&exe.canonicalize().map_err(|_|"当前程序无法验证。")?,&installation.exe){return Err("当前程序不属于已登记安装。".into())}Ok(installation)
    }
}
fn reg_key()->Result<Option<OwnedKey>,String>{unsafe{let name=wide(REGISTRY);let mut raw=ptr::null_mut();match RegOpenKeyExW(HKCU,name.as_ptr(),0,1,&mut raw){0=>Ok(Some(OwnedKey(raw))),2=>Ok(None),status=>Err(format!("无法读取用户安装注册项（{status}）。"))}}}
fn reg_string(key:&OwnedKey,name:&str)->Result<Option<String>,String>{unsafe{
    let name=wide(name);let mut length=0;let status=RegGetValueW(key.0,ptr::null(),name.as_ptr(),2,ptr::null_mut(),ptr::null_mut(),&mut length);if status==2{return Ok(None)}if status!=0||length<2||length>65536||length%2!=0{return Err("安装注册字段无法验证，原值已保留。".into())}
    let mut bytes=vec![0u16;length as usize/2];if RegGetValueW(key.0,ptr::null(),name.as_ptr(),2,ptr::null_mut(),bytes.as_mut_ptr().cast(),&mut length)!=0{return Err("安装注册字段读取失败。".into())}let end=bytes.iter().position(|value|*value==0).ok_or("安装注册字段没有结束标记。")?;if bytes[end..].iter().any(|value|*value!=0){return Err("安装注册字段含有额外数据。".into())}String::from_utf16(&bytes[..end]).map(Some).map_err(|_|"安装注册字段编码无效。".into())
}}
fn uninstall_command(root:&Path)->Result<String,String>{Ok(format!("\"{}\" --uninstall",path_text(&root.join("uninstall.exe"))?))}
fn registration(owner:&str)->Result<Option<Receipt>,String>{
    let Some(key)=reg_key()?else{return Ok(None)};let marker=reg_string(&key,REG_OWNER)?.ok_or("同名安装注册项归属不明，已保留。")?;if marker.len()>16384{return Err("安装注册身份超过读取范围。".into())}let receipt:Receipt=serde_json::from_str(&marker).map_err(|_|"安装注册身份无效，原值已保留。")?;validate_receipt(&receipt,Path::new(&receipt.root),owner)?;
    // The single owner value is written first. Missing presentation values can
    // therefore be repaired after an interrupted write; conflicting values cannot.
    let command=uninstall_command(Path::new(&receipt.root))?;
    if reg_string(&key,"DisplayName")?.is_some_and(|value|value!="CodexProxyTray")||reg_string(&key,"InstallLocation")?.is_some_and(|value|!same_path(Path::new(&value),Path::new(&receipt.root)))||reg_string(&key,"UninstallString")?.is_some_and(|value|value!=command){return Err("同名安装注册项与已登记产品不匹配，已保留。".into())}Ok(Some(receipt))
}
fn same_installation(first:&Receipt,second:&Receipt)->bool{first.product==second.product&&first.install_id==second.install_id&&first.owner_sid==second.owner_sid&&same_path(Path::new(&first.root),Path::new(&second.root))}
fn register(installation:&Installation)->Result<(),String>{unsafe{
    if let Some(previous)=registration(&installation.receipt.owner_sid)?{if !same_installation(&previous,&installation.receipt){return Err("当前用户已有另一处 CodexProxyTray 安装，未接管它。".into())}}
    let name=wide(REGISTRY);let mut raw=ptr::null_mut();let mut disposition=0;if RegCreateKeyExW(HKCU,name.as_ptr(),0,ptr::null(),0,3,ptr::null(),&mut raw,&mut disposition)!=0{return Err("无法创建当前用户的安装注册项。".into())}let key=OwnedKey(raw);
    if disposition!=1&&registration(&installation.receipt.owner_sid)?.is_none_or(|previous|!same_installation(&previous,&installation.receipt)){return Err("安装注册项在写入前发生变化，已保留。".into())}
    let values=[(REG_OWNER,serde_json::to_string(&installation.receipt).map_err(|_|"无法编码安装身份。")?),("DisplayName","CodexProxyTray".into()),("DisplayVersion",installation.version.clone()),("InstallLocation",path_text(&installation.root)?),("UninstallString",uninstall_command(&installation.root)?),("Publisher","OrangeZh1".into()),("URLInfoAbout",crate::update::SOURCE.into()),("DisplayIcon",format!("\"{}\",0",path_text(&installation.exe)?))];
    for(name,value)in values{let name=wide(name);let value=wide(&value);if RegSetValueExW(key.0,name.as_ptr(),0,1,value.as_ptr().cast(),(value.len()*2)as u32)!=0{return Err("当前用户的安装注册尚未完整写入。".into())}}
    for name in ["NoModify","NoRepair"]{let name=wide(name);let one=1u32.to_le_bytes();if RegSetValueExW(key.0,name.as_ptr(),0,4,one.as_ptr(),4)!=0{return Err("当前用户的卸载入口选项未能保存。".into())}}if RegFlushKey(key.0)!=0{return Err("当前用户的安装注册尚未确认保存。".into())}Ok(())
}}
fn unregister(installation:&Installation)->Result<(),String>{unsafe{
    let Some(previous)=registration(&installation.receipt.owner_sid)?else{return Ok(())};if !same_installation(&previous,&installation.receipt){return Err("当前用户的产品注册项属于其他安装，已保留。".into())}let name=wide(REGISTRY);let status=RegDeleteKeyW(HKCU,name.as_ptr());if status!=0&&status!=2{return Err("安装注册项仍有未知子项或无法删除，已保留。".into())}Ok(())
}}
fn apartment()->Result<Apartment,String>{unsafe{CoInitializeEx(None,COINIT_MULTITHREADED).ok().map_err(|error|windows_error("无法初始化产品入口",error))?;}Ok(Apartment)}
fn programs()->Result<PathBuf,String>{unsafe{let raw=SHGetKnownFolderPath(&FOLDERID_Programs,KF_FLAG_DEFAULT,None).map_err(|error|windows_error("无法确定本用户的开始菜单",error))?;let value=raw.to_string();CoTaskMemFree(Some(raw.0.cast()));let path=PathBuf::from(value.map_err(|_|"开始菜单路径无法编码。")?);plain(&path)?;Ok(path)}}
fn link_identity(path:&Path)->Result<Option<PathBuf>,String>{unsafe{
    if !path.exists(){return Ok(None)}plain(path)?;if fs::metadata(path).map_err(|_|"产品入口无法读取。")?.len()>65536{return Err("产品入口超过验证范围，原入口已保留。".into())}
    let link:IShellLinkW=CoCreateInstance(&ShellLink,None,CLSCTX_INPROC_SERVER).map_err(|error|windows_error("无法读取产品入口",error))?;let persist:IPersistFile=link.cast().map_err(|_|"产品入口无法验证。")?;let path=wide(&path_text(path)?);persist.Load(PCWSTR(path.as_ptr()),STGM_READ).map_err(|_|"产品入口无法验证，原入口已保留。")?;let store:IPropertyStore=link.cast().map_err(|_|"产品入口归属不明。")?;
    let mut id=store.GetValue(&APP_ID_KEY).map_err(|_|"产品入口身份无法读取。")?;let owned=id.Anonymous.Anonymous.vt==VT_LPWSTR&&!id.Anonymous.Anonymous.Anonymous.pwszVal.0.is_null()&&id.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok().as_deref()==Some(APP_ID);let _=PropVariantClear(&mut id);if !owned{return Err("同名产品入口属于其他应用，已保留。".into())}
    let mut activator=store.GetValue(&ACTIVATOR_KEY).map_err(|_|"产品入口激活标识无法读取。")?;let owned=activator.Anonymous.Anonymous.vt==VT_CLSID&&!activator.Anonymous.Anonymous.Anonymous.puuid.is_null()&&*activator.Anonymous.Anonymous.Anonymous.puuid==STUB_ACTIVATOR;let _=PropVariantClear(&mut activator);if !owned{return Err("产品入口激活标识不匹配，已保留。".into())}
    let mut target=vec![0u16;32768];link.GetPath(&mut target,ptr::null_mut(),4).map_err(|_|"产品入口目标无法验证。")?;let end=target.iter().position(|value|*value==0).ok_or("产品入口目标无效。")?;let target=PathBuf::from(String::from_utf16(&target[..end]).map_err(|_|"产品入口目标编码无效。")?);let mut args=vec![0u16;32768];link.GetArguments(&mut args).map_err(|_|"产品入口参数无法验证。")?;let end=args.iter().position(|value|*value==0).ok_or("产品入口参数无效。")?;let args=String::from_utf16(&args[..end]).map_err(|_|"产品入口参数编码无效。")?;
    if !absolute(&target)||target.file_name().is_none_or(|name|!name.eq_ignore_ascii_case(GUI))||!matches!(args.as_str(),""|"--background"){return Err("产品入口的程序或参数归属不明，已保留。".into())}Ok(Some(target))
}}
fn controlled_target(target:&Path,root:&Path)->bool{target.file_name().is_some_and(|name|name.eq_ignore_ascii_case(GUI))&&target.parent().and_then(Path::file_name).is_some_and(|name|name.eq_ignore_ascii_case("current")||name.eq_ignore_ascii_case("previous"))&&target.parent().and_then(Path::parent).is_some_and(|parent|same_path(parent,root))}
fn legacy_payload(target:&Path)->bool{
    let Some(directory)=target.parent()else{return false};
    if directory.file_name().is_some_and(|name|name.eq_ignore_ascii_case("current"))&&directory.parent().is_some_and(|root|payload(root,None).is_ok()){return true}
    let checked=(||{let package:Package=read_json(&directory.join("package-receipt.json"),65536)?;if package.schema!=1||package.product!="CodexProxyTray"||package.version.starts_with('v')||core::version_parts(&package.version).is_none()||!package_list_valid(&package.files){return Err("旧便携包身份无效。".to_owned())}for file in package.files{if sha256(&directory.join(file.name))?!=file.sha256{return Err("旧便携包校验不匹配。".into())}}Ok(())})();checked.is_ok()
}
fn owned_target(target:&Path,intended:&Path,registered_root:Option<&Path>,allow_legacy:bool)->bool{
    same_path(target,intended)||registered_root.is_some_and(|root|controlled_target(target,root))||allow_legacy&&legacy_payload(target)
}
fn changed(path:&Path,deleted:bool,existed:bool){if let Ok(path)=path_text(path){let path=wide(&path);unsafe{SHChangeNotify(if deleted{SHCNE_DELETE}else if existed{SHCNE_UPDATEITEM}else{SHCNE_CREATE},SHCNF_PATHW,Some(path.as_ptr().cast()),None);}}}
fn write_link(path:&Path,exe:&Path,existed:bool)->Result<(),String>{unsafe{
    let link:IShellLinkW=CoCreateInstance(&ShellLink,None,CLSCTX_INPROC_SERVER).map_err(|error|windows_error("无法准备产品入口",error))?;let executable=wide(&path_text(exe)?);let empty=wide("");let directory=wide(&path_text(exe.parent().ok_or("产品目录无效。")?)?);link.SetPath(PCWSTR(executable.as_ptr())).map_err(|error|windows_error("无法设置产品入口",error))?;link.SetArguments(PCWSTR(empty.as_ptr())).map_err(|error|windows_error("无法设置普通打开参数",error))?;link.SetWorkingDirectory(PCWSTR(directory.as_ptr())).map_err(|error|windows_error("无法设置产品工作目录",error))?;link.SetShowCmd(SW_SHOWNORMAL).map_err(|error|windows_error("无法设置产品窗口显示方式",error))?;link.SetIconLocation(PCWSTR(executable.as_ptr()),0).map_err(|error|windows_error("无法设置产品图标",error))?;
    let store:IPropertyStore=link.cast().map_err(|_|"无法设置产品入口身份。")?;let mut id=wide(APP_ID);let mut value=PROPVARIANT::default();(*value.Anonymous.Anonymous).vt=VT_LPWSTR;(*value.Anonymous.Anonymous).Anonymous.pwszVal=PWSTR(id.as_mut_ptr());store.SetValue(&APP_ID_KEY,&value).map_err(|error|windows_error("无法设置产品身份",error))?;
    let mut activator=InitPropVariantFromCLSID(&STUB_ACTIVATOR).map_err(|error|windows_error("无法设置通知激活标识",error))?;let result=store.SetValue(&ACTIVATOR_KEY,&activator);let _=PropVariantClear(&mut activator);result.map_err(|error|windows_error("无法设置通知激活方式",error))?;store.Commit().map_err(|error|windows_error("无法保存产品入口身份",error))?;
    let temporary=path.with_file_name(format!(".CodexProxyTray-{}-{}.tmp",std::process::id(),crate::settings::now()));if temporary.exists(){return Err("产品入口临时文件已存在，未覆盖它。".into())}let name=wide(&path_text(&temporary)?);let persist:IPersistFile=link.cast().map_err(|_|"无法保存产品入口。")?;
    let saved=persist.Save(PCWSTR(name.as_ptr()),false).map_err(|error|windows_error("无法保存产品入口",error)).and_then(|_|fs::rename(&temporary,path).map_err(|_|"无法替换产品入口，原入口已保留。".into()));if saved.is_err(){let _=fs::remove_file(&temporary);}saved?;changed(path,false,existed);Ok(())
}}
fn shortcut(exe:&Path,root:Option<&Path>,migrate_autostart:bool)->Result<(),String>{
    let _apartment=apartment()?;let folder=programs()?;let destination=folder.join("CodexProxyTray.lnk");let mut previous=vec![];if let Some(target)=link_identity(&destination)?{if !owned_target(&target,exe,root,true){return Err("现有产品入口属于其他目录，已保留。".into())}previous.push(target);}
    let legacy=[folder.join("CodexProxyTray 更新通知.lnk"),folder.join("CodexProxyTray更新通知.lnk")];let mut remove=vec![];
    for path in &legacy{if let Some(target)=link_identity(path)?{if !owned_target(&target,exe,root,true){return Err("旧通知入口的目标归属不明，已保留。".into())}previous.push(target);remove.push(path);}}
    // Migrate intent before replacing the only durable reference to its old
    // target. A failed Run write leaves every verified shortcut available to retry.
    if migrate_autostart{for target in &previous{if crate::platform::migrate_autostart(target,&PathBuf::from(path_text(exe)?))?{break}}}
    let portable_target=if root.is_none()&&!migrate_autostart{previous.first().map(PathBuf::as_path).unwrap_or(exe)}else{exe};
    write_link(&destination,portable_target,destination.exists())?;
    for path in remove{fs::remove_file(path).map_err(|_|"产品入口已保存，但旧通知入口尚未清理。")?;changed(path,true,true);}Ok(())
}
fn remove_shortcuts(installation:&Installation)->Result<(),String>{
    let _apartment=apartment()?;let folder=programs()?;let mut remove=vec![];
    for name in ["CodexProxyTray.lnk","CodexProxyTray 更新通知.lnk","CodexProxyTray更新通知.lnk"]{let path=folder.join(name);if let Some(target)=link_identity(&path)?{if controlled_target(&target,&installation.root){remove.push(path);}}}
    for path in remove{fs::remove_file(&path).map_err(|_|"当前安装的产品入口无法移除。")?;changed(&path,true,true);}Ok(())
}
/// Used only on an explicit notification attempt, never at portable startup.
pub fn ensure_notification_shortcut()->Result<(),String>{
    let owner=current_sid()?;let _lock=identity_lock(&owner)?;
    if let Some(registered)=registration(&owner)?{let installation=Installation::load(Path::new(&registered.root),&owner,None)?;if !same_installation(&registered,&installation.receipt){return Err("已登记安装的身份发生变化，入口已保留。".into())}return shortcut(&installation.exe,Some(&installation.root),false)}
    let exe=std::env::current_exe().map_err(|_|"无法确定通知所属程序。")?;if !absolute(&exe)||exe.file_name().is_none_or(|name|name!=GUI){return Err("通知程序路径不属于产品 GUI。".into())}plain(&exe)?;shortcut(&exe,None,false)
}
pub fn set_process_identity()->Result<(),String>{unsafe{let id=wide(APP_ID);SetCurrentProcessExplicitAppUserModelID(PCWSTR(id.as_ptr())).map_err(|error|windows_error("无法设置产品任务栏身份",error))}}
pub fn entry(args:&[OsString])->Option<i32>{
    if !args.iter().any(|value|value=="--register-installation"||value=="--unregister-installation"){return None}let action=args.first().and_then(|value|value.to_str()).unwrap_or("");
    Some(match (||{if args.len()!=1||!matches!(action,"--register-installation"|"--unregister-installation"){return Err("安装注册参数无效。".to_owned())}let owner=current_sid()?;let _lock=identity_lock(&owner)?;let installation=Installation::own(&owner)?;
        if action=="--register-installation"{register(&installation)?;shortcut(&installation.exe,Some(&installation.root),true)}else{if let Some(previous)=registration(&owner)?{if !same_installation(&previous,&installation.receipt){return Err("产品注册项属于其他安装，未移除任何入口。".into())}}remove_shortcuts(&installation)?;crate::platform::remove_autostart_for(&PathBuf::from(path_text(&installation.exe)?))?;unregister(&installation)}
    })(){Ok(())=>0,Err(error)=>{eprintln!("{error}");2}})
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn installation_identity_and_payload_manifest_are_strict(){
        let root=Path::new(r"C:\CodexProxyTray");let mut receipt=Receipt{schema:1,product:PRODUCT.into(),install_id:"a".repeat(32),owner_sid:"S-1-5-21-1".into(),root:root.display().to_string(),uninstaller_sha256:"b".repeat(64)};
        assert!(validate_receipt(&receipt,root,"S-1-5-21-1").is_ok());assert!(validate_receipt(&receipt,root,"S-1-5-21-2").is_err());receipt.install_id="A".repeat(32);assert!(validate_receipt(&receipt,root,"S-1-5-21-1").is_err());receipt.install_id="a".repeat(64);assert!(validate_receipt(&receipt,root,"S-1-5-21-1").is_ok());
        let mut files=PAYLOAD.iter().map(|name|FileHash{name:(*name).into(),sha256:"a".repeat(64)}).collect::<Vec<_>>();assert!(file_list_valid(&files));files[0].name="../CodexProxyTray.exe".into();assert!(!file_list_valid(&files));files[0].name=files[1].name.clone();assert!(!file_list_valid(&files));
        let package=PACKAGE_FILES.iter().map(|name|FileHash{name:(*name).into(),sha256:"a".repeat(64)}).collect::<Vec<_>>();assert!(package_list_valid(&package));assert!(!file_list_valid(&package));
        assert!(controlled_target(&root.join("current").join(GUI),root));assert!(!controlled_target(Path::new(r"C:\Other\current\CodexProxyTray.exe"),root));assert!(!absolute(Path::new(r"C:\CodexProxyTray\..\Other")));
        let mut json=serde_json::to_value(&receipt).unwrap();json["phase"]=serde_json::json!("installed");assert!(serde_json::from_value::<Receipt>(json).is_err());
        assert_eq!(entry(&["--register-installation".into(),"--root".into()]),Some(2));assert_eq!(entry(&["--background".into(),"--unregister-installation".into()]),Some(2));
    }
}
