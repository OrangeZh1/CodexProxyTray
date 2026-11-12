//! Management is intentionally limited to verified, current-user npm global installs.
//! It never removes a native executable or touches Codex settings/history.
use serde::Serialize;
use std::{collections::hash_map::DefaultHasher,fs,hash::{Hash,Hasher},io::Read,os::windows::process::CommandExt,path::{Path,PathBuf},process::{Command,Stdio},sync::{Mutex,OnceLock,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
use crate::settings::{self,Config};

const REGISTRY:&str="https://registry.npmjs.org";
const PACKAGE:&str="@openai/codex";
static BUSY:AtomicBool=AtomicBool::new(false);
static MUTATING:AtomicBool=AtomicBool::new(false);
static STATE:OnceLock<Mutex<CliState>>=OnceLock::new();

#[derive(Clone,Debug,Serialize)]
pub struct CliState {
    pub detected:bool,pub supported:bool,pub source:String,pub current:Option<String>,pub latest:Option<String>,
    pub target_token:Option<String>,pub checked_at:Option<u64>,pub operation:Option<String>,pub progress:String,
    pub error:Option<String>,pub reason:Option<String>,pub install_path:Option<String>,
}
impl Default for CliState{fn default()->Self{Self{detected:false,supported:false,source:"unknown".into(),current:None,latest:None,target_token:None,checked_at:None,operation:None,progress:String::new(),error:None,reason:Some("尚未识别 CLI 安装来源。".into()),install_path:None}}}
fn state()->&'static Mutex<CliState>{STATE.get_or_init(||Mutex::new(CliState::default()))}
pub fn snapshot()->CliState{state().lock().unwrap_or_else(|error|error.into_inner()).clone()}
pub fn is_mutating()->bool{MUTATING.load(Ordering::Acquire)}
pub fn is_busy()->bool{BUSY.load(Ordering::Acquire)}
struct Operation;
impl Operation{
    fn begin(name:&str,mutating:bool)->Result<Self,String>{
        BUSY.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_|"另一个 CLI 管理操作仍在进行。".to_owned())?;
        MUTATING.store(mutating,Ordering::Release);let mut view=state().lock().unwrap_or_else(|error|error.into_inner());
        view.operation=Some(name.into());view.progress=match name{"check"=>"正在检查 Codex CLI 最新版本。","update"=>"正在验证 CLI 安装和活动进程。","uninstall"=>"正在验证要卸载的 CLI。",_=>"正在识别 CLI 安装来源。"}.into();view.error=None;Ok(Self)
    }
}
impl Drop for Operation{fn drop(&mut self){state().lock().unwrap_or_else(|error|error.into_inner()).operation=None;MUTATING.store(false,Ordering::Release);BUSY.store(false,Ordering::Release);}}
fn progress(message:impl Into<String>){state().lock().unwrap_or_else(|error|error.into_inner()).progress=message.into();}
fn record_error(error:&str){let mut view=state().lock().unwrap_or_else(|error|error.into_inner());view.error=Some(error.into());view.progress="CLI 操作未完成。".into();}
fn finish<T>(result:Result<T,String>)->Result<T,String>{if let Err(error)=&result{record_error(error);}result}

#[derive(Debug)]
struct Install{node:PathBuf,npm:PathBuf,prefix:PathBuf,package:PathBuf,version:String,token:String}
fn read_small(path:&Path,limit:u64)->Result<Vec<u8>,String>{
    let file=fs::File::open(path).map_err(|_|"安装文件不存在或无法读取。".to_owned())?;
    if file.metadata().map_err(|_|"无法读取安装文件信息。")?.len()>limit{return Err("安装文件超过安全读取范围。".into())}
    let mut bytes=vec![];file.take(limit+1).read_to_end(&mut bytes).map_err(|_|"无法读取安装文件。")?;if bytes.len()as u64>limit{return Err("安装文件过大。".into())}Ok(bytes)
}
fn regular_tree(path:&Path,boundary:&Path)->bool{
    use std::os::windows::fs::MetadataExt;
    let Ok(relative)=path.strip_prefix(boundary)else{return false};let mut cursor=boundary.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)){
        if let Some(component)=component{cursor.push(component.as_os_str());}
        let Ok(metadata)=fs::symlink_metadata(&cursor)else{return false};
        if metadata.file_attributes()&0x400!=0{return false}
    }true
}
fn valid_version(value:&str)->bool{!value.starts_with('v')&&crate::core::version_parts(value).is_some()}
fn package_metadata(path:&Path,name:&str)->Result<(serde_json::Value,Vec<u8>),String>{
    let bytes=read_small(path,256*1024)?;let metadata:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|"安装元数据无效。")?;
    if metadata["name"].as_str()!=Some(name){return Err("安装包身份与预期不一致。".into())}Ok((metadata,bytes))
}
fn fingerprint(paths:&[&Path],contents:&[&[u8]])->Result<String,String>{
    // Confirmation token, not a signature: recheck exact paths, bytes and file stamps before mutation.
    let mut hash=DefaultHasher::new();for path in paths{settings::normalized_executable(path).hash(&mut hash);let metadata=fs::metadata(path).map_err(|_|"安装文件已变化。")?;metadata.len().hash(&mut hash);metadata.modified().ok().and_then(|value|value.duration_since(UNIX_EPOCH).ok()).map(|value|value.as_nanos()).hash(&mut hash);}
    for bytes in contents{bytes.hash(&mut hash);}Ok(format!("{:016x}",hash.finish()))
}
fn inspect_install(command:&[String],prefix:&Path)->Result<Install,String>{
    if command.len()!=2{return Err("此安装不是受支持的当前用户 npm 全局安装，请使用原安装工具管理。".into())}
    let node=PathBuf::from(&command[0]);let script=PathBuf::from(&command[1]);
    let package=prefix.join("node_modules/@openai/codex");let expected=package.join("bin/codex.js");
    if !node.is_absolute()||!node.is_file()||!node.file_name().is_some_and(|name|name.eq_ignore_ascii_case("node.exe"))||settings::normalized_executable(&script)!=settings::normalized_executable(&expected){return Err("CLI 来源无法归属于受支持的 npm 安装，未启用更新或卸载。".into())}
    if !regular_tree(&expected,prefix)||!regular_tree(&package.join("package.json"),prefix)||!regular_tree(&prefix.join("codex.cmd"),prefix)||prefix.join("node_modules/.modules.yaml").exists()||prefix.join("node_modules/.pnpm").exists(){return Err("检测到链接或其他包管理器布局，请使用原安装工具管理。".into())}
    let (metadata,bytes)=package_metadata(&package.join("package.json"),PACKAGE)?;
    let version=metadata["version"].as_str().filter(|value|valid_version(value)).ok_or("CLI 版本格式无法安全管理，请使用原安装工具。")?.to_owned();
    if metadata["bin"]["codex"].as_str()!=Some("bin/codex.js"){return Err("CLI 命令入口与官方 npm 包布局不一致。".into())}
    let shim=read_small(&prefix.join("codex.cmd"),16*1024)?;
    if !String::from_utf8_lossy(&shim).replace('/',"\\").contains("node_modules\\@openai\\codex\\bin\\codex.js"){return Err("npm 命令入口的归属无法确认。".into())}
    let node_directory=node.parent().ok_or("无法定位 Node.js 所在目录。")?;
    let npm=node_directory.join("node_modules/npm/bin/npm-cli.js");
    let npm_package=node_directory.join("node_modules/npm/package.json");
    let (npm_metadata,npm_bytes)=package_metadata(&npm_package,"npm")?;
    if npm_metadata["bin"]["npm"].as_str()!=Some("bin/npm-cli.js")||!npm.is_file(){return Err("无法验证与 Node.js 配套的 npm，请使用原安装工具管理。".into())}
    let script_bytes=read_small(&script,1024*1024)?;let npm_entry=read_small(&npm,1024*1024)?;
    let token=fingerprint(&[&node,&script,&npm,&prefix.join("codex.cmd")],&[&bytes,&npm_bytes,&script_bytes,&npm_entry,&shim])?;
    Ok(Install{node,npm,prefix:prefix.to_path_buf(),package,version,token})
}
fn local_install()->Result<Option<Install>,String>{
    let command=settings::detect_cli(&settings::root());if command.is_empty(){return Ok(None)}
    let appdata=std::env::var_os("APPDATA").filter(|value|!value.is_empty()).ok_or("无法确定当前用户的 npm 安装目录。")?;
    inspect_install(&command,&PathBuf::from(appdata).join("npm")).map(Some)
}
fn apply_local(result:&Result<Option<Install>,String>){
    let mut view=state().lock().unwrap_or_else(|error|error.into_inner());
    match result{
        Ok(Some(install))=>{if view.target_token.as_ref()!=Some(&install.token){view.latest=None;view.checked_at=None;}view.detected=true;view.supported=true;view.source="npm_user_global".into();view.current=Some(install.version.clone());view.target_token=Some(install.token.clone());view.reason=None;view.install_path=Some(install.package.display().to_string());},
        Ok(None)=>{view.detected=false;view.supported=false;view.source="not_installed".into();view.current=None;view.latest=None;view.checked_at=None;view.target_token=None;view.reason=Some("未检测到 Windows Codex CLI。".into());view.install_path=None;},
        Err(message)=>{view.detected=true;view.supported=false;view.source="unmanaged".into();view.current=None;view.latest=None;view.target_token=None;view.reason=Some(message.clone());view.install_path=None;},
    }
}
/// Local filesystem reads only. Call at startup or when the detected command changes.
pub fn inspect()->Result<CliState,String>{
    let operation=Operation::begin("inspect",false)?;let local=local_install();apply_local(&local);progress("CLI 安装来源已识别。");drop(operation);Ok(snapshot())
}
// Silent local refresh: share the operation lease, never clear a user action's
// error or expose a background spinner, and never inspect during a mutation.
fn refresh_with(read:impl FnOnce()->Result<Option<Install>,String>)->bool{
    if BUSY.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return false}
    let _operation=Operation;apply_local(&read());true
}
pub fn refresh_local(){refresh_with(local_install);}
fn fetch_latest(config:&Config)->Result<String,String>{
    if !config.network_enabled{return Err("已关闭联网检测，请先在设置中允许联网。".into())}
    let client=reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION")))
        .proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"代理设置无效。")?).build().map_err(|_|"CLI 更新检查暂不可用。")?;
    let response=client.get(format!("{REGISTRY}/@openai%2fcodex/latest")).send().map_err(|_|"无法连接 npm 官方仓库，请检查本地代理。")?;
    if !response.status().is_success(){return Err(format!("npm 官方仓库暂不可用（HTTP {}）。",response.status().as_u16()))}
    let mut bytes=vec![];response.take(65537).read_to_end(&mut bytes).map_err(|_|"CLI 版本信息读取失败。")?;
    if bytes.len()>65536{return Err("CLI 版本信息超过允许范围。".into())}
    let data:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|"CLI 版本信息无效。")?;
    let version=data["version"].as_str().filter(|value|valid_version(value)).ok_or("官方 CLI 版本格式无效。")?;
    if data["name"].as_str()!=Some(PACKAGE){return Err("CLI 更新信息与官方包身份不匹配。".into())}Ok(version.into())
}
pub fn check()->Result<CliState,String>{
    let operation=Operation::begin("check",false)?;
    let result=(||{let local=local_install();apply_local(&local);let _install=local?.ok_or("未检测到可管理的 Codex CLI。")?;
        {let mut view=state().lock().unwrap_or_else(|error|error.into_inner());view.latest=None;view.checked_at=None;}
        let config=settings::load(&settings::root()).map_err(|_|"无法读取联网配置。")?;let latest=fetch_latest(&config)?;
        let mut view=state().lock().unwrap_or_else(|error|error.into_inner());view.latest=Some(latest);view.checked_at=Some(settings::now());view.progress="CLI 更新检查已完成。".into();Ok(())})();
    finish(result)?;drop(operation);Ok(snapshot())
}
#[derive(Clone,Copy)]enum Mutation{Update,Uninstall}
impl Mutation{fn name(self)->&'static str{match self{Self::Update=>"update",Self::Uninstall=>"uninstall"}}}
fn mutation_args(install:&Install,action:Mutation,version:Option<&str>,scratch:&Path)->Result<Vec<String>,String>{
    let mut args=vec![install.npm.display().to_string(),match action{Mutation::Update=>"install",Mutation::Uninstall=>"uninstall"}.into()];
    args.push(match action{Mutation::Update=>format!("{PACKAGE}@{}",version.filter(|value|valid_version(value)).ok_or("缺少已验证的目标版本。")?),Mutation::Uninstall=>PACKAGE.into()});
    args.extend(["--global".into(),"--prefix".into(),install.prefix.display().to_string(),"--ignore-scripts".into(),"--no-audit".into(),"--no-fund".into(),"--progress=false".into(),"--update-notifier=false".into(),"--registry".into(),REGISTRY.into(),format!("--@openai:registry={REGISTRY}"),"--userconfig".into(),scratch.join("user.npmrc").display().to_string(),"--globalconfig".into(),scratch.join("global.npmrc").display().to_string(),"--fetch-timeout=30000".into(),"--fetch-retries=1".into()]);Ok(args)
}
fn drain(mut reader:impl Read)->String{let mut tail=vec![];let mut chunk=[0u8;4096];while let Ok(count)=reader.read(&mut chunk){if count==0{break}tail.extend_from_slice(&chunk[..count]);if tail.len()>16384{tail.drain(..tail.len()-16384);}}String::from_utf8_lossy(&tail).into_owned()}
fn execute_npm(install:&Install,args:&[String],config:&Config,action:Mutation)->Result<(),String>{
    let mut command=Command::new(&install.node);command.args(args).current_dir(&install.prefix).creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).env_clear();
    for name in ["SystemRoot","WINDIR","TEMP","TMP","USERPROFILE","LOCALAPPDATA","APPDATA","PATH","PATHEXT","ComSpec"]{if let Some(value)=std::env::var_os(name){command.env(name,value);}}
    // The package is public; user npmrc, scope registry overrides and Node hooks are not inherited.
    command.env("NPM_CONFIG_UPDATE_NOTIFIER","false").env("NPM_CONFIG_YES","true").env("NPM_CONFIG_LOGS_MAX","0");
    if matches!(action,Mutation::Update){command.env("HTTP_PROXY",&config.proxy_url).env("HTTPS_PROXY",&config.proxy_url).env("ALL_PROXY",&config.proxy_url).env("NO_PROXY","localhost,127.0.0.1,::1");}
    let mut child=command.spawn().map_err(|_|"无法启动已验证的 npm 管理器。")?;
    let stdout=child.stdout.take().ok_or("无法读取 npm 输出。")?;let stderr=child.stderr.take().ok_or("无法读取 npm 错误。")?;
    let out=thread::spawn(move||drain(stdout));let err=thread::spawn(move||drain(stderr));let started=Instant::now();
    let status=loop{match child.try_wait(){Ok(Some(status))=>break status,Ok(None)=>{progress(format!("npm 正在{} Codex CLI，已运行 {} 秒。",if matches!(action,Mutation::Update){"更新"}else{"卸载"},started.elapsed().as_secs()));thread::sleep(Duration::from_millis(250));},Err(_)=>{let status=child.wait().map_err(|_|"无法确认 npm 的最终状态，请查看安装目录后再操作。")?;break status}}};
    let output=out.join().unwrap_or_default()+&err.join().unwrap_or_default();
    if !status.success(){let code=output.lines().find_map(|line|line.trim().strip_prefix("npm error code ")).filter(|value|value.len()<=48&&value.chars().all(|value|value.is_ascii_alphanumeric()||value=='_')).unwrap_or("UNKNOWN");return Err(format!("npm {}未完成（退出码 {}，{}）。安装文件可能已变化，请重新检查安装状态；用户配置和历史未由助手删除。",if matches!(action,Mutation::Update){"更新"}else{"卸载"},status.code().map_or("未知".into(),|value|value.to_string()),code))}Ok(())
}
fn active_cli(root:&Path,install:&Install)->Result<bool,String>{
    use windows::{core::PWSTR,Win32::{Foundation::{CloseHandle,ERROR_INVALID_PARAMETER},System::{Diagnostics::ToolHelp::{CreateToolhelp32Snapshot,Process32FirstW,Process32NextW,PROCESSENTRY32W,TH32CS_SNAPPROCESS},Threading::{OpenProcess,QueryFullProcessImageNameW,PROCESS_QUERY_LIMITED_INFORMATION,PROCESS_NAME_WIN32}}}};
    let config=Config{cli_command:vec![install.node.display().to_string(),install.package.join("bin/codex.js").display().to_string()],..Config::default()};
    let allowed=settings::cli_process_paths(root,&config);
    unsafe{
        let snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0).map_err(|_|"无法检查正在运行的 CLI，未开始修改。")?;
        let result=(||{let mut entry=PROCESSENTRY32W{dwSize:std::mem::size_of::<PROCESSENTRY32W>()as u32,..Default::default()};Process32FirstW(snapshot,&mut entry).map_err(|_|"无法读取进程列表，未开始修改 CLI。".to_owned())?;let mut more=true;
            while more{let name=String::from_utf16_lossy(&entry.szExeFile[..entry.szExeFile.iter().position(|value|*value==0).unwrap_or(entry.szExeFile.len())]);
                if name.eq_ignore_ascii_case("codex.exe"){
                    let handle=match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,entry.th32ProcessID){Ok(handle)=>Some(handle),Err(error)if error.code().0 as u32&0xffff==ERROR_INVALID_PARAMETER.0=>None,Err(_)=>return Err("无法确认某个 CLI 进程的归属，未开始更新或卸载。".into())};
                    if let Some(handle)=handle{
                    let mut image=vec![0u16;32768];let mut length=image.len()as u32;let queried=QueryFullProcessImageNameW(handle,PROCESS_NAME_WIN32,PWSTR(image.as_mut_ptr()),&mut length);let _=CloseHandle(handle);
                    if queried.is_err(){return Err("无法读取某个 CLI 进程的路径，未开始修改。".into())}image.truncate(length as usize);
                    if allowed.contains(&settings::normalized_executable(&PathBuf::from(String::from_utf16_lossy(&image)))){return Ok(true)}
                    }
                }more=match Process32NextW(snapshot,&mut entry){Ok(())=>true,Err(error)if error.code().0 as u32&0xffff==18=>false,Err(_)=>return Err("进程列表读取不完整，未开始修改 CLI。".into())};
            }Ok(false)})();let _=CloseHandle(snapshot);result
    }
}
fn mutate(token:&str,action:Mutation)->Result<CliState,String>{
    let operation=Operation::begin(action.name(),true)?;
    let result=(||{
        let before=snapshot();let install=local_install()?.ok_or("Codex CLI 已不存在，请重新检查。")?;
        if token!=install.token||before.target_token.as_deref()!=Some(token){return Err("CLI 安装已变化，请重新检查后再次确认操作。".into())}
        let config=settings::load(&settings::root()).map_err(|_|"无法读取设置，未开始修改 CLI。")?;
        let version=if matches!(action,Mutation::Update){
            if !config.network_enabled{return Err("已关闭联网检测，无法下载 CLI 更新。".into())}
            let latest=before.latest.as_deref().filter(|value|valid_version(value)).ok_or("请先检查 CLI 更新。")?;
            if !crate::core::newer_version(latest,&install.version){return Err("当前没有已验证的新版本。".into())}
            if before.checked_at.is_none_or(|at|settings::now().saturating_sub(at)>15*60*1000){return Err("CLI 版本检查已过期，请重新检查后更新。".into())}Some(latest)
        }else{None};
        if active_cli(&settings::root(),&install)?{return Err("Codex CLI 仍在运行。请先保存并结束相关 CLI 任务，再更新或卸载；助手不会终止这些任务。".into())}
        let scratch=std::env::temp_dir().join(format!("CodexProxyTray-cli-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()));
        fs::create_dir(&scratch).map_err(|_|"无法创建本次 npm 操作目录。")?;
        let result=(||{fs::write(scratch.join("user.npmrc"),b"").and_then(|_|fs::write(scratch.join("global.npmrc"),b"")).map_err(|_|"无法隔离 npm 配置。")?;
            let args=mutation_args(&install,action,version,&scratch)?;
            // Revalidate immediately before starting the only mutating process.
            if local_install()?.is_none_or(|now|now.token!=install.token){return Err("CLI 安装在确认后已变化，操作已停止。".into())}
            if active_cli(&settings::root(),&install)?{return Err("检测到新的 CLI 任务，操作已停止，请结束任务后再试。".into())}
            execute_npm(&install,&args,&config,action)
        })();
        // This exact new directory contains only the two task-owned npm config files.
        for name in ["user.npmrc","global.npmrc"]{let _=fs::remove_file(scratch.join(name));}let _=fs::remove_dir(&scratch);
        let after=local_install();apply_local(&after);result?;
        match action{
            Mutation::Update=>{let after=after?.ok_or("npm 已退出，但没有检测到更新后的 CLI。")?;if Some(after.version.as_str())!=version{return Err("npm 已退出，但安装版本与请求版本不一致，请重新检查。".into())}progress(format!("Codex CLI 已更新到 {}。",after.version));},
            Mutation::Uninstall=>{if install.package.exists(){return Err("npm 已退出，但 CLI 安装目录仍存在，请用原安装工具检查。".into())}progress("Codex CLI 已卸载。用户配置和历史记录已保留。");}
        }Ok(())
    })();finish(result)?;drop(operation);Ok(snapshot())
}
/// Caller must obtain a product confirmation for this exact target token first.
pub fn update(target_token:String)->Result<CliState,String>{mutate(&target_token,Mutation::Update)}
/// Removes the managed npm package only, never the assistant or ChatGPT.
pub fn uninstall(target_token:String)->Result<CliState,String>{mutate(&target_token,Mutation::Uninstall)}

#[cfg(test)]mod tests{
    use super::*;
    fn sandbox()->PathBuf{let path=std::env::temp_dir().join(format!("cli-manager-check-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir(&path).unwrap();path}
    fn fixture(root:&Path)->(Vec<String>,PathBuf){let prefix=root.join("current user npm");let node=root.join("node/node.exe");let npm=root.join("node/node_modules/npm/bin/npm-cli.js");let script=prefix.join("node_modules/@openai/codex/bin/codex.js");
        for path in [&node,&npm,&script]{fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,b"fixture").unwrap();}
        fs::write(npm.parent().unwrap().parent().unwrap().join("package.json"),br#"{"name":"npm","bin":{"npm":"bin/npm-cli.js"}}"#).unwrap();
        fs::write(script.parent().unwrap().parent().unwrap().join("package.json"),br#"{"name":"@openai/codex","version":"0.167.0","bin":{"codex":"bin/codex.js"}}"#).unwrap();
        fs::write(prefix.join("codex.cmd"),br#"node "%dp0%\node_modules\@openai\codex\bin\codex.js" %*"#).unwrap();(vec![node.display().to_string(),script.display().to_string()],prefix)
    }
    #[test]fn background_refresh_detects_installation_changes_and_respects_active_operations(){
        let root=sandbox();let(command,prefix)=fixture(&root);let before=snapshot();
        assert!(refresh_with(||Ok(None)));assert_eq!(snapshot().source,"not_installed");
        assert!(refresh_with(||inspect_install(&command,&prefix).map(Some)));let installed=snapshot();assert!(installed.supported);assert_eq!(installed.current.as_deref(),Some("0.167.0"));assert!(installed.target_token.is_some());assert!(installed.operation.is_none());
        let operation=Operation::begin("update",true).unwrap();assert!(!refresh_with(||panic!("Background discovery must not run during an update")));assert_eq!(snapshot().operation.as_deref(),Some("update"));drop(operation);
        fs::remove_dir_all(prefix.join("node_modules/@openai/codex")).unwrap();assert!(refresh_with(||Ok(None)));assert!(!snapshot().supported);assert!(snapshot().target_token.is_none());
        *state().lock().unwrap()=before;fs::remove_dir_all(root).unwrap();
    }
    #[test]fn ownership_and_confirmation_reject_foreign_layouts_and_changed_files(){let root=sandbox();let(command,prefix)=fixture(&root);let first=inspect_install(&command,&prefix).unwrap();assert_eq!(first.version,"0.167.0");
        assert!(inspect_install(&command,&root.join("different prefix")).is_err());assert!(inspect_install(&[command[0].clone()],&prefix).is_err());
        fs::write(&command[1],b"changed entry").unwrap();assert_ne!(first.token,inspect_install(&command,&prefix).unwrap().token);
        fs::write(prefix.join("node_modules/.modules.yaml"),b"pnpm").unwrap();assert!(inspect_install(&command,&prefix).is_err());fs::remove_file(prefix.join("node_modules/.modules.yaml")).unwrap();
        fs::write(prefix.join("node_modules/@openai/codex/package.json"),br#"{"name":"other","version":"1.0.0","bin":{"codex":"bin/codex.js"}}"#).unwrap();assert!(inspect_install(&command,&prefix).is_err());fs::remove_dir_all(root).unwrap();}
    #[test]fn npm_arguments_are_pinned_scoped_and_do_not_remove_configuration(){let root=sandbox();let(command,prefix)=fixture(&root);let install=inspect_install(&command,&prefix).unwrap();let update=mutation_args(&install,Mutation::Update,Some("0.168.0"),&root).unwrap();
        assert_eq!(&update[1..3],&["install","@openai/codex@0.168.0"]);assert!(update.windows(2).any(|args|args==["--prefix",prefix.to_str().unwrap()]));assert!(update.contains(&"--global".into()));assert!(update.contains(&"--ignore-scripts".into()));assert!(update.contains(&REGISTRY.into()));
        assert!(mutation_args(&install,Mutation::Update,Some("latest && erase"),&root).is_err());let uninstall=mutation_args(&install,Mutation::Uninstall,None,&root).unwrap();assert_eq!(&uninstall[1..3],&["uninstall",PACKAGE]);assert!(!uninstall.iter().any(|arg|arg.contains(".codex")||arg.contains("ChatGPT")));fs::remove_dir_all(root).unwrap();}
    #[test]fn output_is_drained_but_retention_is_bounded(){let input=vec![b'x';128*1024];assert_eq!(drain(input.as_slice()).len(),16384);for invalid in ["0.1.0-beta","v0.1.0","0.1","01.2.3","1.2.3 & calc"]{assert!(!valid_version(invalid));}}
    #[test]fn isolated_mock_manager_receives_exact_prefix_and_preserves_user_data(){
        let root=sandbox();let(command,prefix)=fixture(&root);let mut install=inspect_install(&command,&prefix).unwrap();
        // CI provides Node. Run only the fresh fake npm entry below, never the installed npm CLI.
        let node=Command::new("node").args(["-p","process.execPath"]).creation_flags(0x08000000).output().expect("Node is required by the Windows CI job");assert!(node.status.success());install.node=PathBuf::from(String::from_utf8(node.stdout).unwrap().trim());
        let expected=serde_json::to_string(&prefix.display().to_string()).unwrap();
        let fake=format!(r#"const fs=require('node:fs'),path=require('node:path');
const [verb,pkg,...args]=process.argv.slice(2),prefix=args[args.indexOf('--prefix')+1];
if(path.resolve(prefix)!==path.resolve({expected})||!args.includes('--global')||!args.includes('--ignore-scripts'))throw Error('Wrong fixture target');
const target=path.join(prefix,'node_modules/@openai/codex');
fs.writeFileSync(path.join(prefix,'observed.json'),JSON.stringify({{verb,pkg,args}}));
if(verb==='install'){{let data=JSON.parse(fs.readFileSync(path.join(target,'package.json')));data.version=pkg.slice(pkg.lastIndexOf('@')+1);fs.writeFileSync(path.join(target,'package.json'),JSON.stringify(data));}}
else if(verb==='uninstall'){{if(pkg!=='@openai/codex')throw Error('Wrong package');fs.rmSync(target,{{recursive:true}});}}
else process.exit(9);"#);
        fs::write(&install.npm,fake).unwrap();fs::write(root.join("config-and-history"),b"keep").unwrap();
        fs::write(root.join("user.npmrc"),b"").unwrap();fs::write(root.join("global.npmrc"),b"").unwrap();
        let update=mutation_args(&install,Mutation::Update,Some("0.168.0"),&root).unwrap();execute_npm(&install,&update,&Config::default(),Mutation::Update).unwrap();
        let metadata:serde_json::Value=serde_json::from_slice(&fs::read(install.package.join("package.json")).unwrap()).unwrap();assert_eq!(metadata["version"],"0.168.0");
        let uninstall=mutation_args(&install,Mutation::Uninstall,None,&root).unwrap();execute_npm(&install,&uninstall,&Config::default(),Mutation::Uninstall).unwrap();
        assert!(!install.package.exists());assert_eq!(fs::read(root.join("config-and-history")).unwrap(),b"keep");
        let observed:serde_json::Value=serde_json::from_slice(&fs::read(prefix.join("observed.json")).unwrap()).unwrap();assert_eq!(observed["verb"],"uninstall");assert_eq!(observed["pkg"],PACKAGE);fs::remove_dir_all(root).unwrap();
    }
    #[test]fn owned_cli_process_fixture(){
        let Some(path)=std::env::var_os("CODEX_PROXY_CLI_MANAGER_PROCESS_FIXTURE")else{return};let root=PathBuf::from(path);
        fs::write(root.join("ready"),b"ready").unwrap();let started=Instant::now();while !root.join("stop").exists()&&started.elapsed()<Duration::from_secs(10){thread::sleep(Duration::from_millis(20));}
    }
    #[test]fn active_process_check_uses_exact_binary_paths_and_never_name_alone(){
        let root=sandbox();let(command,prefix)=fixture(&root);let install=inspect_install(&command,&prefix).unwrap();
        let expected=install.package.join("vendor/x86_64-pc-windows-msvc/bin/codex.exe");let foreign=root.join("unrelated/codex.exe");
        for path in [&expected,&foreign]{fs::create_dir_all(path.parent().unwrap()).unwrap();fs::copy(std::env::current_exe().unwrap(),path).unwrap();}
        for (index,path,wanted) in [(0,&foreign,false),(1,&expected,true)]{
            let handshake=root.join(format!("process-{index}"));fs::create_dir(&handshake).unwrap();
            let mut child=Command::new(path).args(["--exact","cli_manager::tests::owned_cli_process_fixture","--nocapture"]).env("CODEX_PROXY_CLI_MANAGER_PROCESS_FIXTURE",&handshake).creation_flags(0x08000000).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            let result=(||{let deadline=Instant::now()+Duration::from_secs(5);while !handshake.join("ready").exists()&&Instant::now()<deadline{if child.try_wait().unwrap().is_some(){break}thread::sleep(Duration::from_millis(20));}
                if !handshake.join("ready").exists(){return Err("Owned test fixture did not start.")}
                if active_cli(&root.join("assistant"),&install).unwrap()!=wanted{return Err("Exact path attribution failed.")}Ok(())})();
            // Cooperatively stop only this test's fresh child; its own deadline is also bounded.
            fs::write(handshake.join("stop"),b"stop").unwrap();assert!(child.wait().unwrap().success());result.unwrap();
        }fs::remove_dir_all(root).unwrap();
    }
}
